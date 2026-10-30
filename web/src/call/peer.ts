import type { SignalPayload } from "../../../shared/protocol";

const DISCONNECTED_GRACE_MS = 4_000;
const MIN_RESTART_INTERVAL_MS = 5_000;
// Enough for clean 720p; a direct China <-> Uzbekistan call carried ~2 Mbps with negligible loss.
const VIDEO_MAX_BITRATE = 2_000_000;

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
  private readonly videoSender: RTCRtpSender;
  private makingOffer = false;
  private ignoreOffer = false;
  private signalQueue: Promise<void> = Promise.resolve();
  private remoteStream = new MediaStream();
  private disconnectTimer?: number;
  private lastRestart = 0;
  private closed = false;
  private screen = false;

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
        void this.tuneSenders();
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
      void this.tuneSenders();
    } else {
      try {
        await pc.addIceCandidate(payload.candidate as RTCIceCandidateInit);
      } catch (err) {
        if (!this.ignoreOffer) throw err;
      }
    }
  }

  /** Swaps camera ↔ screen without renegotiating, and retunes the encoder for the new content. */
  async setVideoTrack(track: MediaStreamTrack | null, screen: boolean): Promise<void> {
    this.screen = screen;
    await this.videoSender.replaceTrack(track);
    await this.tuneSenders();
  }

  /**
   * Caps video bitrate and tells the encoder what to give up when bandwidth runs short: resolution for the camera
   * (a smaller clean picture beats a blocky one), frame rate for screen share (text must stay readable).
   * Audio gets priority so voices survive congestion. Encodings only exist after negotiation, so this runs after
   * every description change.
   */
  private async tuneSenders(): Promise<void> {
    for (const transceiver of this.pc.getTransceivers()) {
      const sender = transceiver.sender;
      const params = sender.getParameters();
      if (!params.encodings?.length) continue;
      const isVideo = transceiver.receiver.track.kind === "video";
      for (const enc of params.encodings) {
        if (isVideo) enc.maxBitrate = VIDEO_MAX_BITRATE;
        else enc.priority = enc.networkPriority = "high";
      }
      if (isVideo) params.degradationPreference = this.screen ? "maintain-resolution" : "maintain-framerate";
      try {
        await sender.setParameters(params);
      } catch {
        // Some browsers reject priority/degradation fields; the bitrate cap is the part that matters.
        if (!isVideo) continue;
        const capped = sender.getParameters();
        capped.encodings.forEach((enc) => (enc.maxBitrate = VIDEO_MAX_BITRATE));
        await sender.setParameters(capped).catch((err) => console.warn("setParameters failed", err));
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
