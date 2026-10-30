import type { SignalPayload } from "../../../shared/protocol";

const DISCONNECTED_GRACE_MS = 4_000;
const MIN_RESTART_INTERVAL_MS = 5_000;

interface PeerCallbacks {
  sendSignal: (payload: SignalPayload) => void;
  onRemoteStream: (stream: MediaStream) => void;
  onStateChange: () => void;
}

/**
 * One RTCPeerConnection using the "perfect negotiation" pattern, so either side can renegotiate
 * (ICE restart, relay toggle) without offer glare breaking the call.
 */
export class PeerSession {
  readonly pc: RTCPeerConnection;
  readonly videoSender: RTCRtpSender;
  private makingOffer = false;
  private ignoreOffer = false;
  private signalQueue: Promise<void> = Promise.resolve();
  private remoteStream = new MediaStream();
  private disconnectTimer?: number;
  private lastRestart = 0;
  private closed = false;

  constructor(
    config: RTCConfiguration,
    localStream: MediaStream,
    private polite: boolean,
    private cb: PeerCallbacks,
  ) {
    this.pc = new RTCPeerConnection(config);
    const pc = this.pc;

    for (const track of localStream.getTracks()) pc.addTrack(track, localStream);
    if (localStream.getAudioTracks().length === 0) pc.addTransceiver("audio", { direction: "sendrecv" });
    // Always have a video sender, so screen share works even when joined without a camera.
    this.videoSender =
      pc.getSenders().find((s) => s.track?.kind === "video") ??
      pc.addTransceiver("video", { direction: "sendrecv", streams: [localStream] }).sender;

    pc.ontrack = ({ track }) => {
      this.remoteStream = new MediaStream([...this.remoteStream.getTracks().filter((t) => t.kind !== track.kind), track]);
      cb.onRemoteStream(this.remoteStream);
    };

    pc.onnegotiationneeded = async () => {
      try {
        this.makingOffer = true;
        await pc.setLocalDescription();
        if (pc.localDescription) cb.sendSignal({ kind: "description", description: pc.localDescription.toJSON() });
      } catch (err) {
        console.warn("negotiation failed", err);
      } finally {
        this.makingOffer = false;
      }
    };

    pc.onicecandidate = ({ candidate }) => {
      if (candidate) cb.sendSignal({ kind: "candidate", candidate: candidate.toJSON() });
    };

    pc.oniceconnectionstatechange = () => {
      clearTimeout(this.disconnectTimer);
      if (pc.iceConnectionState === "failed") this.restartIce();
      if (pc.iceConnectionState === "disconnected") {
        this.disconnectTimer = window.setTimeout(() => {
          if (pc.iceConnectionState === "disconnected") this.restartIce();
        }, DISCONNECTED_GRACE_MS);
      }
      cb.onStateChange();
    };
    pc.onconnectionstatechange = () => cb.onStateChange();
  }

  /** Signals are processed strictly in order so candidates never race their description. */
  handleSignal(payload: SignalPayload): void {
    this.signalQueue = this.signalQueue.then(() => this.applySignal(payload)).catch((err) => console.warn("signal error", err));
  }

  private async applySignal(payload: SignalPayload): Promise<void> {
    if (this.closed) return;
    const pc = this.pc;
    if (payload.kind === "description") {
      const description = payload.description as RTCSessionDescriptionInit;
      const offerCollision = description.type === "offer" && (this.makingOffer || pc.signalingState !== "stable");
      this.ignoreOffer = !this.polite && offerCollision;
      if (this.ignoreOffer) return;
      await pc.setRemoteDescription(description);
      if (description.type === "offer") {
        await pc.setLocalDescription();
        if (pc.localDescription) this.cb.sendSignal({ kind: "description", description: pc.localDescription.toJSON() });
      }
    } else {
      try {
        await pc.addIceCandidate(payload.candidate as RTCIceCandidateInit);
      } catch (err) {
        if (!this.ignoreOffer) throw err;
      }
    }
  }

  restartIce(): void {
    if (this.closed || Date.now() - this.lastRestart < MIN_RESTART_INTERVAL_MS) return;
    this.lastRestart = Date.now();
    this.pc.restartIce();
  }

  setForceRelay(relay: boolean): void {
    this.pc.setConfiguration({ ...this.pc.getConfiguration(), iceTransportPolicy: relay ? "relay" : "all" });
    this.lastRestart = 0;
    this.restartIce();
  }

  close(): void {
    this.closed = true;
    clearTimeout(this.disconnectTimer);
    this.pc.ontrack = this.pc.onnegotiationneeded = this.pc.onicecandidate = null;
    this.pc.oniceconnectionstatechange = this.pc.onconnectionstatechange = null;
    this.pc.close();
  }
}
