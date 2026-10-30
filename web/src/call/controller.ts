import type { ChatMessage, IceServerConfig, ServerMessage } from "../../../shared/protocol";
import { PeerSession } from "./peer";
import { Signaling, type SocketStatus } from "./signaling";
import { sampleStats, type CallStats } from "./stats";

export type Phase = "idle" | "starting" | "waiting" | "in-call" | "full" | "ended" | "error";

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
  hasCamera: boolean;
  sharing: boolean;
  forceRelay: boolean;
  stats: CallStats | null;
  error: string | null;
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
  hasCamera: false,
  sharing: false,
  forceRelay: new URLSearchParams(location.search).has("relay"),
  stats: null,
  error: null,
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
  private statsTimer?: number;
  private statsCounters: Parameters<typeof sampleStats>[1] = null;

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

  async start(): Promise<void> {
    this.set({ phase: "starting", error: null, chat: [], remoteStream: null });
    let stream: MediaStream;
    try {
      stream = await getLocalMedia();
    } catch (err) {
      this.set({ phase: "error", error: `Can't access microphone/camera: ${(err as Error).message}` });
      return;
    }
    this.camTrack = stream.getVideoTracks()[0] ?? null;
    this.set({ localStream: stream, hasCamera: !!this.camTrack, micOn: true, camOn: !!this.camTrack });

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
    await this.peer?.videoSender.replaceTrack(track);
    this.set({ sharing: true });
  }

  private async stopScreenShare(): Promise<void> {
    this.screenTrack?.stop();
    this.screenTrack = null;
    await this.peer?.videoSender.replaceTrack(this.camTrack);
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
    if (this.screenTrack) void this.peer.videoSender.replaceTrack(this.screenTrack);
    this.statsCounters = null;
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
      this.set({ stats });
    } catch {
      // pc closed mid-sample
    }
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

async function getLocalMedia(): Promise<MediaStream> {
  const audio: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };
  try {
    return await navigator.mediaDevices.getUserMedia({
      audio,
      video: { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 }, facingMode: "user" },
    });
  } catch (err) {
    // No camera, or camera busy/denied: still allow a voice call.
    return navigator.mediaDevices.getUserMedia({ audio, video: false }).catch(() => Promise.reject(err));
  }
}
