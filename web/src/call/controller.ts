import type { ChatMessage, IceServerConfig, ServerMessage } from "../../../shared/protocol";
import { PeerSession } from "./peer";
import { Signaling, type SocketStatus } from "./signaling";
import { sampleStats, type CallStats } from "./stats";

// While a call is relayed, retry direct after these delays (the last one repeats). Direct is ~2x faster than going
// through Singapore, and a P2P attempt that failed at call start often works a few minutes later.
const DIRECT_RETRY_DELAYS_MS = [15_000, 30_000, 60_000, 120_000];
const TRYING_DIRECT_SHOWN_MS = 5_000;

export type Phase = "idle" | "starting" | "waiting" | "in-call" | "full" | "ended";

/** What happened when we asked for a device: denied = blocked by the user/browser, busy = another app holds it. */
export type MediaAccess = "pending" | "granted" | "denied" | "missing" | "busy";
export type MediaKind = "audio" | "video";

export interface DeviceOption {
  id: string;
  label: string;
}

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
  forceRelay: boolean;
  stats: CallStats | null;
  tryingDirect: boolean;
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
  forceRelay: new URLSearchParams(location.search).has("relay"),
  stats: null,
  tryingDirect: false,
};

/** Owns media, signaling and the peer connection; React only renders its state snapshots. */
export class CallController {
  private state: CallState = initialState;
  private listeners = new Set<() => void>();
  private signaling: Signaling;
  private peer: PeerSession | null = null;
  private polite = false;
  private iceServers: IceServerConfig[] = [];
  private camTrack: MediaStreamTrack | null = null;
  private screenTrack: MediaStreamTrack | null = null;
  private mediaReady: Promise<void> | null = null;
  private statsTimer?: number;
  private statsCounters: Parameters<typeof sampleStats>[1] = null;
  private directRetries = 0;
  private nextDirectRetry = 0;
  private lastDirectRetry = 0;

  constructor(roomId: string) {
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

  /** Switches mic or camera before joining; remembered for next time. */
  async selectDevice(kind: MediaKind, deviceId: string): Promise<void> {
    const stream = this.state.localStream;
    if (!stream || this.peer) return;
    saveDevicePref(kind, deviceId);
    const others = stream.getTracks().filter((t) => t.kind !== kind);
    // Stop the old track first: iOS won't open a second camera while one is live.
    stream.getTracks().filter((t) => t.kind === kind).forEach((t) => t.stop());
    let got = await getTrack(kind, deviceId);
    if (!got.track) got = await getTrack(kind, null);
    if (got.track) got.track.enabled = kind === "audio" ? this.state.micOn : this.state.camOn;
    const next = new MediaStream(got.track ? [...others, got.track] : others);
    if (kind === "video") this.camTrack = got.track;
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

  toggleCam(): void {
    if (!this.camTrack) return;
    const on = !this.state.camOn;
    this.camTrack.enabled = on;
    this.set({ camOn: on });
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
        this.signaling.setClientId(msg.clientId);
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

const AUDIO: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };
const VIDEO: MediaTrackConstraints = { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30, max: 30 } };

function constraintsFor(kind: MediaKind, deviceId: string | null): MediaTrackConstraints {
  const base = kind === "audio" ? AUDIO : VIDEO;
  if (deviceId) return { ...base, deviceId: { ideal: deviceId } }; // ideal: falls back if the device is gone
  return kind === "video" ? { ...base, facingMode: "user" } : base;
}

async function getLocalMedia(
  micId: string | null,
  camId: string | null,
): Promise<{ stream: MediaStream; audio: MediaAccess; video: MediaAccess }> {
  if (!navigator.mediaDevices?.getUserMedia) return { stream: new MediaStream(), audio: "missing", video: "missing" };
  try {
    // One request, so the browser shows a single permission prompt.
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: constraintsFor("audio", micId),
      video: constraintsFor("video", camId),
    });
    return { stream, audio: "granted", video: "granted" };
  } catch {
    // No camera, camera busy or one of them denied: ask separately to find out which, and keep the other.
    const audio = await getTrack("audio", micId);
    const video = await getTrack("video", camId);
    const tracks = [audio.track, video.track].filter((t): t is MediaStreamTrack => t !== null);
    return { stream: new MediaStream(tracks), audio: audio.access, video: video.access };
  }
}

async function getTrack(kind: MediaKind, deviceId: string | null): Promise<{ track: MediaStreamTrack | null; access: MediaAccess }> {
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ [kind]: constraintsFor(kind, deviceId) });
    return { track: stream.getTracks()[0], access: "granted" };
  } catch (err) {
    const name = (err as DOMException).name;
    if (name === "NotAllowedError" || name === "SecurityError") return { track: null, access: "denied" };
    if (name === "NotFoundError" || name === "OverconstrainedError") return { track: null, access: "missing" };
    return { track: null, access: "busy" };
  }
}

function selectedDevices(stream: MediaStream): Pick<CallState, "micDeviceId" | "camDeviceId"> {
  return {
    micDeviceId: stream.getAudioTracks()[0]?.getSettings().deviceId ?? null,
    camDeviceId: stream.getVideoTracks()[0]?.getSettings().deviceId ?? null,
  };
}

function loadDevicePref(kind: MediaKind): string | null {
  try {
    return localStorage.getItem(`device.${kind}`);
  } catch {
    return null;
  }
}

function saveDevicePref(kind: MediaKind, deviceId: string): void {
  try {
    localStorage.setItem(`device.${kind}`, deviceId);
  } catch {
    // storage blocked: the choice just isn't remembered
  }
}
