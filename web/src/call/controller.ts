import type { ChatMessage, IceServerConfig, Lang, ServerMessage } from "../../../shared/protocol";
import {
  getLocalMedia,
  getTrack,
  loadDevicePref,
  loadOrientation,
  saveDevicePref,
  saveOrientation,
  selectedDevices,
  videoConstraints,
  type DeviceOption,
  type MediaAccess,
  type MediaKind,
  type Orientation,
} from "./media";
import { PeerSession } from "./peer";
import { Signaling, type SocketStatus } from "./signaling";
import { loadLang, loadTranscribeOn, saveLang, saveTranscribeOn } from "./speech/lang";
import { Transcriber, type SpeechNotice } from "./speech/transcriber";
import { sampleStats, type CallStats } from "./stats";

// While a call is relayed, retry direct after these delays (the last one repeats). Direct is ~2x faster than going
// through Singapore, and a P2P attempt that failed at call start often works a few minutes later.
const DIRECT_RETRY_DELAYS_MS = [15_000, 30_000, 60_000, 120_000];
const TRYING_DIRECT_SHOWN_MS = 5_000;

export type Phase = "idle" | "starting" | "waiting" | "in-call" | "full" | "ended";

export interface CallState {
  phase: Phase;
  socket: SocketStatus;
  myId: string | null;
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
  iceState: RTCIceConnectionState | null;
  chat: ChatMessage[];
  micOn: boolean;
  camOn: boolean;
  micAccess: MediaAccess;
  camAccess: MediaAccess;
  mics: DeviceOption[];
  cams: DeviceOption[];
  micDeviceId: string | null;
  camDeviceId: string | null;
  sharing: boolean;
  orientation: Orientation;
  forceRelay: boolean;
  stats: CallStats | null;
  tryingDirect: boolean;
  lang: Lang;
  transcribeOn: boolean;
  speechNotice: SpeechNotice | null;
}

const initialState: CallState = {
  phase: "idle",
  socket: "closed",
  myId: null,
  localStream: null,
  remoteStream: null,
  iceState: null,
  chat: [],
  micOn: true,
  camOn: true,
  micAccess: "pending",
  camAccess: "pending",
  mics: [],
  cams: [],
  micDeviceId: null,
  camDeviceId: null,
  sharing: false,
  orientation: loadOrientation(),
  forceRelay: new URLSearchParams(location.search).has("relay"),
  stats: null,
  tryingDirect: false,
  lang: loadLang(),
  transcribeOn: loadTranscribeOn(),
  speechNotice: null,
};

/** Owns media, signaling and the peer connection; React only renders its state snapshots. */
export class CallController {
  private state: CallState = initialState;
  private listeners = new Set<() => void>();
  private signaling: Signaling;
  private peer: PeerSession | null = null;
  private polite = false;
  private iceServers: IceServerConfig[] = [];
  private token: string | null = null;
  private camTrack: MediaStreamTrack | null = null;
  private screenTrack: MediaStreamTrack | null = null;
  private mediaReady: Promise<void> | null = null;
  private switchingDevice = false;
  private statsTimer?: number;
  private statsCounters: Parameters<typeof sampleStats>[1] = null;
  private directRetries = 0;
  private nextDirectRetry = 0;
  private lastDirectRetry = 0;
  private transcriber = new Transcriber({
    sendText: (text) => this.signaling.send({ type: "chat", text, spoken: true }),
    speechUrl: () => this.token && `/ws/speech?${new URLSearchParams({ room: this.roomId, token: this.token })}`,
    onNotice: (speechNotice) => this.set({ speechNotice }),
  });

  constructor(private roomId: string) {
    this.signaling = new Signaling(
      roomId,
      (msg) => this.onMessage(msg),
      (socket) => this.set({ socket }),
    );
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = () => this.state;

  private set(patch: Partial<CallState>): void {
    this.state = { ...this.state, ...patch };
    const { transcribeOn, micOn, phase, localStream, lang } = this.state;
    // Only while someone is there to read it, and never while muted
    const speaking = transcribeOn && micOn && phase === "in-call";
    this.transcriber.sync((speaking && localStream?.getAudioTracks()[0]) || null, lang);
    for (const l of this.listeners) l();
  }

  /** Opens camera and mic for the pre-join screen. Calling it again (Retry) re-asks for both. */
  preview(): Promise<void> {
    this.mediaReady ??= this.acquireMedia().finally(() => (this.mediaReady = null));
    return this.mediaReady;
  }

  private async acquireMedia(): Promise<void> {
    this.state.localStream?.getTracks().forEach((t) => t.stop());
    this.set({ localStream: null, micAccess: "pending", camAccess: "pending" });
    const { stream, audio, video } = await getLocalMedia(loadDevicePref("audio"), loadDevicePref("video"));
    stream.getAudioTracks().forEach((t) => (t.enabled = this.state.micOn));
    this.camTrack = stream.getVideoTracks()[0] ?? null;
    if (this.camTrack) this.camTrack.enabled = this.state.camOn;
    this.set({ localStream: stream, micAccess: audio, camAccess: video, ...selectedDevices(stream) });
    navigator.mediaDevices?.addEventListener("devicechange", this.refreshDevices);
    await this.refreshDevices();
  }

  /**
   * Switches mic or camera; remembered for next time. In a call only the camera can change: the new track replaces
   * the old one in the sender, so there's no renegotiation.
   */
  async selectDevice(kind: MediaKind, deviceId: string): Promise<void> {
    const stream = this.state.localStream;
    if (!stream || (this.peer && kind === "audio") || this.switchingDevice) return;
    this.switchingDevice = true;
    try {
      await this.swapDevice(stream, kind, deviceId);
    } finally {
      this.switchingDevice = false;
    }
  }

  /** Moves to the next camera: front ↔ back on a phone. */
  nextCamera(): void {
    const { cams, camDeviceId } = this.state;
    if (cams.length < 2) return;
    const i = cams.findIndex((c) => c.id === camDeviceId);
    void this.selectDevice("video", cams[(i + 1) % cams.length].id);
  }

  private async swapDevice(stream: MediaStream, kind: MediaKind, deviceId: string): Promise<void> {
    saveDevicePref(kind, deviceId);
    const others = stream.getTracks().filter((t) => t.kind !== kind);
    // Stop the old track first: iOS won't open a second camera while one is live.
    stream.getTracks().filter((t) => t.kind === kind).forEach((t) => t.stop());
    let got = await getTrack(kind, deviceId);
    if (!got.track) got = await getTrack(kind, null);
    if (got.track) got.track.enabled = kind === "audio" ? this.state.micOn : this.state.camOn;
    const next = new MediaStream(got.track ? [...others, got.track] : others);
    if (kind === "video") {
      this.camTrack = got.track;
      // While sharing the screen the sender keeps the screen; stopScreenShare picks up the new camera
      if (!this.screenTrack) await this.peer?.setVideoTrack(got.track, false);
    }
    this.set({
      localStream: next,
      ...(kind === "audio" ? { micAccess: got.access } : { camAccess: got.access }),
      ...selectedDevices(next),
    });
  }

  private refreshDevices = async () => {
    const all = await navigator.mediaDevices.enumerateDevices().catch(() => []);
    const list = (kind: MediaDeviceKind, fallback: string) =>
      all
        .filter((d) => d.kind === kind && d.deviceId)
        .map((d, i) => ({ id: d.deviceId, label: d.label || `${fallback} ${i + 1}` }));
    this.set({ mics: list("audioinput", "Microphone"), cams: list("videoinput", "Camera") });
  };

  async start(): Promise<void> {
    this.set({ phase: "starting", chat: [], remoteStream: null });
    // Joining without camera/mic is allowed: you can still hear, watch and chat.
    if (this.mediaReady || !this.state.localStream) await this.preview();

    window.addEventListener("online", this.onOnline);
    window.addEventListener("pagehide", this.onPageHide);
    this.signaling.connect();
    this.statsTimer = window.setInterval(() => this.pollStats(), 1000);
  }

  hangUp(): void {
    this.teardown(true);
    this.set({ phase: "ended" });
  }

  sendChat(text: string): boolean {
    return this.signaling.send({ type: "chat", text });
  }

  toggleMic(): void {
    const on = !this.state.micOn;
    this.state.localStream?.getAudioTracks().forEach((t) => (t.enabled = on));
    this.set({ micOn: on });
  }

  toggleTranscribe(): void {
    saveTranscribeOn(!this.state.transcribeOn);
    this.set({ transcribeOn: !this.state.transcribeOn });
  }

  setLang(lang: Lang): void {
    saveLang(lang);
    this.set({ lang });
  }

  toggleCam(): void {
    if (!this.camTrack) return;
    const on = !this.state.camOn;
    this.camTrack.enabled = on;
    this.set({ camOn: on });
  }

  /** Reshapes the live camera track in place, so a call in progress needs no renegotiation. */
  async setOrientation(orientation: Orientation): Promise<void> {
    saveOrientation(orientation);
    this.set({ orientation });
    await this.camTrack?.applyConstraints(videoConstraints(orientation)).catch(() => {
      // camera can't do that shape: it keeps sending what it had
    });
  }

  async toggleScreenShare(): Promise<void> {
    if (this.screenTrack) return this.stopScreenShare();
    let track: MediaStreamTrack;
    try {
      const display = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 15 }, audio: false });
      track = display.getVideoTracks()[0];
    } catch {
      return; // user cancelled the picker
    }
    track.contentHint = "detail";
    track.onended = () => this.stopScreenShare();
    this.screenTrack = track;
    await this.peer?.setVideoTrack(track, true);
    this.set({ sharing: true });
  }

  private async stopScreenShare(): Promise<void> {
    this.screenTrack?.stop();
    this.screenTrack = null;
    await this.peer?.setVideoTrack(this.camTrack, false);
    this.set({ sharing: false });
  }

  setForceRelay(forceRelay: boolean): void {
    this.set({ forceRelay });
    this.peer?.setForceRelay(forceRelay);
  }

  private onMessage(msg: ServerMessage): void {
    switch (msg.type) {
      case "joined":
        this.signaling.setIdentity(msg.clientId, msg.token);
        this.token = msg.token;
        this.iceServers = msg.iceServers;
        this.set({ myId: msg.clientId, chat: msg.chat });
        if (msg.resumed && this.peer) {
          // Our socket dropped (network switch); the media path likely died with it.
          if (msg.peerPresent) this.peer.restartIce();
          else this.dropPeer();
        } else {
          this.polite = msg.polite;
          if (msg.peerPresent) this.createPeer();
        }
        if (!this.peer) this.set({ phase: "waiting" });
        break;
      case "peer-joined":
        this.polite = msg.polite;
        this.createPeer();
        break;
      case "peer-left":
        this.dropPeer();
        break;
      case "signal":
        this.peer?.handleSignal(msg.payload);
        break;
      case "chat":
        this.set({ chat: [...this.state.chat, msg.message] });
        break;
      case "chat-translation":
        this.set({ chat: this.state.chat.map((m) => (m.id === msg.id ? { ...m, translation: msg.translation } : m)) });
        break;
      case "full":
        this.teardown(false);
        this.set({ phase: "full" });
        break;
      case "error":
        console.warn("server error", msg.message);
        break;
    }
  }

  private createPeer(): void {
    this.dropPeer();
    const localStream = this.state.localStream!;
    this.peer = new PeerSession(
      { iceServers: this.iceServers, iceTransportPolicy: this.state.forceRelay ? "relay" : "all" },
      localStream,
      this.polite,
      {
        sendSignal: (payload) => this.signaling.send({ type: "signal", payload }),
        onRemoteStream: (remoteStream) => this.set({ remoteStream }),
        onStateChange: () => this.set({ iceState: this.peer?.pc.iceConnectionState ?? null }),
      },
    );
    if (this.screenTrack) void this.peer.setVideoTrack(this.screenTrack, true);
    this.statsCounters = null;
    this.directRetries = this.nextDirectRetry = 0;
    this.set({ phase: "in-call", iceState: this.peer.pc.iceConnectionState });
  }

  private dropPeer(): void {
    this.peer?.close();
    this.peer = null;
    this.set({ phase: "waiting", remoteStream: null, iceState: null, stats: null });
  }

  private async pollStats(): Promise<void> {
    const pc = this.peer?.pc;
    if (!pc) return;
    try {
      const { stats, counters } = await sampleStats(pc, this.statsCounters);
      if (this.peer?.pc !== pc) return;
      this.statsCounters = counters;
      this.maybeRetryDirect(stats);
      this.set({ stats, tryingDirect: Date.now() - this.lastDirectRetry < TRYING_DIRECT_SHOWN_MS });
    } catch {
      // pc closed mid-sample
    }
  }

  /**
   * Upgrades a relayed call to direct P2P via an ICE restart, with backoff. Only the impolite side drives it so the
   * two peers don't restart over each other.
   */
  private maybeRetryDirect(stats: CallStats): void {
    if (stats.path === "direct" || this.state.forceRelay || this.polite) {
      this.directRetries = this.nextDirectRetry = 0;
      return;
    }
    if (stats.path !== "relay") return;
    const now = Date.now();
    if (!this.nextDirectRetry) {
      this.nextDirectRetry = now + DIRECT_RETRY_DELAYS_MS[0];
      return;
    }
    if (now < this.nextDirectRetry) return;
    this.directRetries++;
    this.nextDirectRetry = now + DIRECT_RETRY_DELAYS_MS[Math.min(this.directRetries, DIRECT_RETRY_DELAYS_MS.length - 1)];
    this.lastDirectRetry = now;
    this.peer?.restartIce();
  }

  private onOnline = () => {
    this.signaling.kick();
    this.peer?.restartIce();
  };

  private onPageHide = () => this.teardown(true);

  private teardown(leave: boolean): void {
    window.removeEventListener("online", this.onOnline);
    window.removeEventListener("pagehide", this.onPageHide);
    clearInterval(this.statsTimer);
    this.peer?.close();
    this.peer = null;
    this.signaling.close(leave);
    this.screenTrack?.stop();
    this.screenTrack = null;
    this.state.localStream?.getTracks().forEach((t) => t.stop());
    this.set({ localStream: null, remoteStream: null, iceState: null, stats: null, sharing: false });
  }
}
