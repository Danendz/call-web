import type { ChatMessage, FullReason, IceServerConfig, Lang, LiveMessage, Reaction, ServerMessage } from "../../../shared/protocol";
import { getNoiseSuppression, onNoiseSuppressionChange } from "../settings/noise";
import { denoise, preloadDenoise, type Denoised } from "./denoise";
import {
  BROWSER_AUDIO,
  getDisplayMedia,
  getLocalMedia,
  getTrack,
  loadDevicePref,
  loadSpeakerPref,
  loadOrientation,
  saveDevicePref,
  saveOrientation,
  saveSpeakerPref,
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
// through the relay, and a P2P attempt that failed at call start often works a few minutes later.
const DIRECT_RETRY_DELAYS_MS = [15_000, 30_000, 60_000, 120_000];
const TRYING_DIRECT_SHOWN_MS = 5_000;

export type Phase = "idle" | "starting" | "waiting" | "in-call" | "full" | "ended";

export interface CallState {
  phase: Phase;
  socket: SocketStatus;
  myId: string | null;
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
  /** The other person's shared screen (with its audio, if they shared some), while they share. */
  remoteScreen: MediaStream | null;
  /** The other person's camera is on; when off, their video track still sends black frames. */
  remoteCamOn: boolean;
  iceState: RTCIceConnectionState | null;
  chat: ChatMessage[];
  /** What the other person is saying right now, until its chat line comes. */
  live: LiveMessage | null;
  micOn: boolean;
  camOn: boolean;
  micAccess: MediaAccess;
  camAccess: MediaAccess;
  mics: DeviceOption[];
  cams: DeviceOption[];
  micDeviceId: string | null;
  camDeviceId: string | null;
  /** The device of each kind being opened, until it's in use: opening one can take a second or two. */
  switching: Record<MediaKind, string | null>;
  /** Empty where the browser can't pick where sound plays (iOS Safari): it plays where the system sends it. */
  speakers: DeviceOption[];
  /** The speaker picked; null for the system's default, also when the picked one is unplugged. */
  speakerDeviceId: string | null;
  /** You muted the other person's sound. Only for this call: the next one starts with sound. */
  speakerOff: boolean;
  /** The other person muted your sound. */
  remoteSpeakerOff: boolean;
  sharing: boolean;
  orientation: Orientation;
  forceRelay: boolean;
  stats: CallStats | null;
  tryingDirect: boolean;
  lang: Lang;
  transcribeOn: boolean;
  speechNotice: SpeechNotice | null;
  /** performance.now() at which the call with the other person began; null while alone. */
  callStartedAt: number | null;
  /** The other person just saved a screenshot of the call. */
  peerScreenshot: boolean;
  /** Why the room can't be joined, in phase "full"; undefined: two people are already in it. */
  fullReason?: FullReason;
}

const initialState: CallState = {
  phase: "idle",
  socket: "closed",
  myId: null,
  localStream: null,
  remoteStream: null,
  remoteScreen: null,
  remoteCamOn: true,
  iceState: null,
  chat: [],
  live: null,
  micOn: true,
  camOn: true,
  micAccess: "pending",
  camAccess: "pending",
  mics: [],
  cams: [],
  micDeviceId: null,
  camDeviceId: null,
  switching: { audio: null, video: null },
  speakers: [],
  speakerDeviceId: null,
  speakerOff: false,
  remoteSpeakerOff: false,
  sharing: false,
  orientation: loadOrientation(),
  forceRelay: new URLSearchParams(location.search).has("relay"),
  stats: null,
  tryingDirect: false,
  lang: loadLang(),
  transcribeOn: loadTranscribeOn(),
  speechNotice: null,
  callStartedAt: null,
  peerScreenshot: false,
};

const PEER_SCREENSHOT_SHOWN_MS = 3_000;

/** A reaction to fly over the video: one you sent, or one from the other person. */
export interface ReactionEvent {
  emoji: Reaction;
  mine: boolean;
}

/** Owns media, signaling and the peer connection; React only renders its state snapshots. */
export class CallController {
  // The language may have been changed on another page since this module loaded
  private state: CallState = { ...initialState, lang: loadLang() };
  private disposed = false;
  private listeners = new Set<() => void>();
  // Reactions are moments, not state: they go straight to whoever draws them
  private reactionListeners = new Set<(r: ReactionEvent) => void>();
  private signaling: Signaling;
  private peer: PeerSession | null = null;
  private polite = false;
  private iceServers: IceServerConfig[] = [];
  private token: string | null = null;
  private camTrack: MediaStreamTrack | null = null;
  private screen: MediaStream | null = null;
  // The id the other side last announced for its screen, kept until they leave: it can arrive before our peer exists.
  private remoteScreenId: string | null = null;
  private mediaReady: Promise<void> | null = null;
  private switchingDevice = false;
  private pendingDevices = new Map<MediaKind, string>();
  // The mic after noise suppression: what's sent and transcribed. The raw mic stays in localStream, for the level
  // meter, the picker and muting.
  private denoised: Denoised | null = null;
  private micVersion = 0;
  // The next mic switch closes the old one first, so the browser's audio processing follows the new constraints
  private reopenMic = false;
  private statsTimer?: number;
  private statsCounters: Parameters<typeof sampleStats>[1] = null;
  private peerScreenshotTimer?: number;
  private directRetries = 0;
  private nextDirectRetry = 0;
  private lastDirectRetry = 0;
  private transcriber = new Transcriber({
    sendText: (text) => this.signaling.send({ type: "chat", text, spoken: true }),
    speechUrl: () => this.token && `/ws/speech?${new URLSearchParams({ room: this.roomId, token: this.token })}`,
    onNotice: (speechNotice) => this.set({ speechNotice }),
  });

  private stopNoiseListener: () => void;

  constructor(private roomId: string) {
    this.signaling = new Signaling(
      roomId,
      (msg) => this.onMessage(msg),
      (socket) => this.set({ socket }),
    );
    if (getNoiseSuppression()) preloadDenoise();
    this.stopNoiseListener = onNoiseSuppressionChange(() => this.noiseSuppressionChanged());
  }

  /** The page moved away from this room: leave it and let go of camera and mic. */
  dispose(): void {
    this.disposed = true;
    this.stopNoiseListener();
    this.teardown(true);
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = () => this.state;

  onReaction = (listener: (r: ReactionEvent) => void) => {
    this.reactionListeners.add(listener);
    return () => this.reactionListeners.delete(listener);
  };

  private emitReaction(r: ReactionEvent): void {
    for (const l of this.reactionListeners) l(r);
  }

  private set(patch: Partial<CallState>): void {
    this.state = { ...this.state, ...patch };
    const { transcribeOn, micOn, phase, localStream, lang } = this.state;
    // Only while someone is there to read it, and never while muted
    const speaking = transcribeOn && micOn && phase === "in-call";
    this.transcriber.sync((speaking && (this.denoised?.track ?? localStream?.getAudioTracks()[0])) || null, lang);
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
    // The page left this room while the browser was asking: don't keep the camera on for nothing
    if (this.disposed) return stream.getTracks().forEach((t) => t.stop());
    stream.getAudioTracks().forEach((t) => (t.enabled = this.state.micOn));
    this.camTrack = stream.getVideoTracks()[0] ?? null;
    if (this.camTrack) this.camTrack.enabled = this.state.camOn;
    this.set({ localStream: stream, micAccess: audio, camAccess: video, ...selectedDevices(stream) });
    void this.useMic(stream.getAudioTracks()[0] ?? null);
    navigator.mediaDevices?.addEventListener("devicechange", this.refreshDevices);
    await this.refreshDevices();
  }

  /**
   * Switches mic or camera; remembered for next time. In a call the new track replaces the old one in the sender, so
   * there's no renegotiation. A pick made while a switch is running isn't lost: the latest one for each kind follows it.
   */
  async selectDevice(kind: MediaKind, deviceId: string): Promise<void> {
    if (!this.state.localStream) return;
    this.pendingDevices.set(kind, deviceId);
    this.setSwitching(kind, deviceId);
    if (this.switchingDevice) return;
    this.switchingDevice = true;
    try {
      for (let next = first(this.pendingDevices); next; next = first(this.pendingDevices)) {
        const [kind, deviceId] = next;
        this.pendingDevices.delete(kind);
        const stream = this.state.localStream;
        if (stream) await this.swapDevice(stream, kind, deviceId);
        // A newer pick for this kind is still to come; it keeps showing until it's done
        if (!this.pendingDevices.has(kind)) this.setSwitching(kind, null);
      }
    } finally {
      this.switchingDevice = false;
      this.set({ switching: { audio: null, video: null } });
    }
  }

  private setSwitching(kind: MediaKind, deviceId: string | null): void {
    this.set({ switching: { ...this.state.switching, [kind]: deviceId } });
  }

  private async swapDevice(stream: MediaStream, kind: MediaKind, deviceId: string): Promise<void> {
    saveDevicePref(kind, deviceId);
    const old = stream.getTracks().filter((t) => t.kind === kind);
    if (kind === "audio" && this.reopenMic) {
      this.reopenMic = false;
      old.forEach((t) => t.stop());
    }
    // The new device opens while the old one still runs, so there's no gap. A phone that can't hold two cameras at
    // once says it's busy: stop the old one and ask again. A device that's gone falls back to the default.
    let got = await getTrack(kind, deviceId, true);
    if (!got.track) {
      old.forEach((t) => t.stop());
      if (got.access === "busy") got = await getTrack(kind, deviceId, true);
      if (!got.track) got = await getTrack(kind, null);
    }
    // Hung up or reacquired while the device was opening
    if (this.state.localStream !== stream) {
      got.track?.stop();
      return;
    }
    if (got.track) got.track.enabled = kind === "audio" ? this.state.micOn : this.state.camOn;
    if (kind === "video") this.camTrack = got.track;
    if (kind === "audio") await this.useMic(got.track);
    else await this.peer?.setTrack(kind, got.track);
    // The same stream keeps playing with the new track, so the <video> showing it doesn't reload
    old.forEach((t) => {
      stream.removeTrack(t);
      t.stop();
    });
    if (got.track) stream.addTrack(got.track);
    this.set({
      localStream: stream,
      ...(kind === "audio" ? { micAccess: got.access } : { camAccess: got.access }),
      ...selectedDevices(stream),
    });
    if (kind === "video") this.sendMedia();
  }

  /**
   * Sends and transcribes `mic` through noise suppression when it's on, or as it is. A newer mic (or setting) that
   * comes along while the model loads wins.
   */
  private async useMic(mic: MediaStreamTrack | null): Promise<void> {
    const version = ++this.micVersion;
    const on = !!mic && getNoiseSuppression();
    const next = mic && on ? await denoise(mic) : null;
    if (version !== this.micVersion) {
      next?.stop();
      return;
    }
    // This browser can't run the model: its own noise suppression back on, rather than none
    if (mic && on && !next) void mic.applyConstraints(BROWSER_AUDIO).catch(() => {});
    const old = this.denoised;
    this.denoised = next;
    await this.peer?.setTrack("audio", next?.track ?? mic);
    old?.stop();
    this.set({});
  }

  /** The switch in Settings: the mic reopens with the browser's noise suppression off or on, then goes through ours or not. */
  private noiseSuppressionChanged(): void {
    if (getNoiseSuppression()) preloadDenoise();
    const id = this.state.micDeviceId;
    if (!this.state.localStream?.getAudioTracks().length || !id) return;
    this.reopenMic = true;
    void this.selectDevice("audio", id);
  }

  private refreshDevices = async () => {
    const all = await navigator.mediaDevices.enumerateDevices().catch(() => []);
    const list = (kind: MediaDeviceKind, fallback: string) =>
      all
        .filter((d) => d.kind === kind && d.deviceId)
        .map((d, i) => ({ id: d.deviceId, label: d.label || `${fallback} ${i + 1}` }));
    const speakers = canPickSpeaker() ? list("audiooutput", "Speaker") : [];
    const saved = loadSpeakerPref();
    this.set({
      mics: list("audioinput", "Microphone"),
      cams: list("videoinput", "Camera"),
      speakers,
      speakerDeviceId: speakers.some((s) => s.id === saved) ? saved : null,
    });
  };

  /** Where the other person's sound plays; remembered for next time. */
  selectSpeaker(deviceId: string): void {
    saveSpeakerPref(deviceId);
    this.set({ speakerDeviceId: deviceId });
  }

  /** Mutes or unmutes the other person, and tells them so they know you can't hear them. */
  toggleSpeaker(): void {
    this.set({ speakerOff: !this.state.speakerOff });
    this.sendMedia();
  }

  async start(): Promise<void> {
    this.set({ phase: "starting", chat: [], live: null, remoteStream: null });
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

  /** Lets the other person know a picture of the call was saved. */
  notifyScreenshot(): void {
    this.signaling.send({ type: "screenshot" });
  }

  sendReaction(emoji: Reaction): void {
    if (this.signaling.send({ type: "reaction", emoji })) this.emitReaction({ emoji, mine: true });
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
    this.sendMedia();
  }

  /** Reshapes the live camera track in place, so a call in progress needs no renegotiation. */
  async setOrientation(orientation: Orientation): Promise<void> {
    saveOrientation(orientation);
    this.set({ orientation });
    await this.camTrack?.applyConstraints(videoConstraints(orientation)).catch(() => {
      // camera can't do that shape: it keeps sending what it had
    });
  }

  /** Shares the screen next to the camera, with tab or system audio when the browser offers it and the user ticks it. */
  async toggleScreenShare(): Promise<void> {
    if (this.screen) return this.stopScreenShare();
    let display: MediaStream;
    try {
      display = await getDisplayMedia();
    } catch {
      return; // user cancelled the picker
    }
    const [video] = display.getVideoTracks();
    if (!video) return display.getTracks().forEach((t) => t.stop());
    video.contentHint = "detail";
    video.onended = () => this.stopScreenShare();
    this.screen = display;
    this.set({ sharing: true });
    // Announce first, so the other side knows the screen's stream before its tracks arrive
    this.sendMedia();
    await this.peer?.setScreen(video, display.getAudioTracks()[0] ?? null);
  }

  private async stopScreenShare(): Promise<void> {
    if (!this.screen) return;
    this.screen.getTracks().forEach((t) => t.stop());
    this.screen = null;
    this.set({ sharing: false });
    this.sendMedia();
    await this.peer?.setScreen(null, null);
  }

  /** Tells the other side what we're sending; see MediaMessage. */
  private sendMedia(): void {
    if (!this.peer) return;
    const camOn = !!this.camTrack && this.state.camOn;
    const { speakerOff } = this.state;
    this.signaling.send({ type: "media", camOn, screen: this.screen ? this.peer.screenId : null, speakerOff });
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
        this.set({ myId: msg.clientId, chat: msg.chat, live: null });
        if (msg.resumed && this.peer) {
          // Our socket dropped (network switch); the media path likely died with it.
          if (msg.peerPresent) {
            this.peer.restartIce();
            this.sendMedia(); // in case an update was lost with the socket
          } else this.dropPeer();
        } else {
          this.polite = msg.polite;
          if (msg.peerPresent) this.createPeer();
        }
        if (!this.peer) this.set({ phase: "waiting" });
        else this.setCallElapsed(msg.callElapsedMs);
        break;
      case "peer-joined":
        this.polite = msg.polite;
        this.createPeer(); // drops any old peer, and its timer with it
        this.setCallElapsed(msg.callElapsedMs);
        break;
      case "peer-left":
        this.remoteScreenId = null;
        this.dropPeer();
        this.set({ remoteCamOn: true, remoteSpeakerOff: false, live: null });
        break;
      case "signal":
        this.peer?.handleSignal(msg.payload);
        break;
      case "chat": {
        const finishesLive = msg.message.liveId !== undefined && msg.message.liveId === this.state.live?.id;
        this.set({ chat: [...this.state.chat, msg.message], ...(finishesLive && { live: null }) });
        break;
      }
      case "chat-live":
        if (msg.text) this.set({ live: msg });
        else if (msg.id === this.state.live?.id) this.set({ live: null });
        break;
      case "chat-translation":
        this.set({ chat: this.state.chat.map((m) => (m.id === msg.id ? { ...m, translation: msg.translation } : m)) });
        break;
      case "full":
        this.teardown(false);
        this.set({ phase: "full", fullReason: msg.reason });
        break;
      case "screenshot":
        clearTimeout(this.peerScreenshotTimer);
        this.set({ peerScreenshot: true });
        this.peerScreenshotTimer = window.setTimeout(() => this.set({ peerScreenshot: false }), PEER_SCREENSHOT_SHOWN_MS);
        break;
      case "reaction":
        this.emitReaction({ emoji: msg.emoji, mine: false });
        break;
      case "media":
        this.remoteScreenId = msg.screen;
        this.peer?.setRemoteScreen(msg.screen);
        this.set({ remoteCamOn: msg.camOn, remoteSpeakerOff: msg.speakerOff === true });
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
        onRemoteStreams: (remoteStream, remoteScreen) => this.set({ remoteStream, remoteScreen }),
        onStateChange: () => this.set({ iceState: this.peer?.pc.iceConnectionState ?? null }),
      },
    );
    if (this.remoteScreenId) this.peer.setRemoteScreen(this.remoteScreenId);
    if (this.denoised) {
      // Started before anyone clicked (the setup screen), its audio may still be suspended
      this.denoised.resume();
      void this.peer.setTrack("audio", this.denoised.track);
    }
    this.sendMedia();
    if (this.screen) void this.peer.setScreen(this.screen.getVideoTracks()[0], this.screen.getAudioTracks()[0] ?? null);
    this.statsCounters = null;
    this.directRetries = this.nextDirectRetry = 0;
    this.set({ phase: "in-call", iceState: this.peer.pc.iceConnectionState });
  }

  private setCallElapsed(ms: unknown): void {
    if (typeof ms === "number" && Number.isFinite(ms) && ms >= 0) this.set({ callStartedAt: performance.now() - ms });
  }

  private dropPeer(): void {
    this.peer?.close();
    this.peer = null;
    this.set({ phase: "waiting", remoteStream: null, remoteScreen: null, iceState: null, stats: null, callStartedAt: null });
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
    this.screen?.getTracks().forEach((t) => t.stop());
    this.screen = null;
    this.remoteScreenId = null;
    this.state.localStream?.getTracks().forEach((t) => t.stop());
    this.micVersion++;
    this.denoised?.stop();
    this.denoised = null;
    clearTimeout(this.peerScreenshotTimer);
    this.set({
      localStream: null,
      remoteStream: null,
      remoteScreen: null,
      remoteCamOn: true,
      speakerOff: false,
      remoteSpeakerOff: false,
      iceState: null,
      stats: null,
      sharing: false,
      callStartedAt: null,
      peerScreenshot: false,
    });
  }
}

// Safari on iOS has no setSinkId: sound goes where the system sends it
function canPickSpeaker(): boolean {
  return typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype;
}

function first<K, V>(map: Map<K, V>): [K, V] | undefined {
  return map.entries().next().value;
}
