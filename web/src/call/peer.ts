import type { SignalPayload } from "../../../shared/protocol";

const DISCONNECTED_GRACE_MS = 4_000;
const MIN_RESTART_INTERVAL_MS = 5_000;
// Enough for clean 720p; a direct call into mainland China carried ~2 Mbps with negligible loss.
const VIDEO_MAX_BITRATE = 2_000_000;
// While sharing: the screen, and a small face for the camera. The screen's cap is only a ceiling: congestion control
// finds what the path carries below it, so a LAN or a good link gets a sharp, smooth screen (a playing video needs far
// more than text), and a weak one isn't hurt.
const SHARE_SCREEN_BITRATE = 15_000_000;
const SHARE_SCREEN_FPS = 60;
const SHARE_CAMERA_BITRATE = 400_000;
const SHARE_CAMERA_SHORT_SIDE = 360;

interface PeerCallbacks {
  sendSignal: (payload: SignalPayload) => void;
  /** camera: the other person's mic and camera; screen: their shared screen and its audio, while they share */
  onRemoteStreams: (camera: MediaStream | null, screen: MediaStream | null) => void;
  onStateChange: () => void;
}

/**
 * One RTCPeerConnection using the "perfect negotiation" pattern, so either side can renegotiate
 * (ICE restart, relay toggle) without offer glare breaking the call.
 */
export class PeerSession {
  readonly pc: RTCPeerConnection;
  private readonly senders: Record<"audio" | "video", RTCRtpSender>;
  private makingOffer = false;
  private ignoreOffer = false;
  private signalQueue: Promise<void> = Promise.resolve();
  // Incoming tracks by the id of the stream the sender put them in; "" for tracks sent without one.
  private remoteStreams = new Map<string, MediaStream>();
  // Every id the other side announced as its screen, so a stopped share's tracks are never taken for the camera
  private remoteScreenIds = new Set<string>();
  private remoteScreenId: string | null = null;
  private remoteCamera: MediaStream | null = null;
  private disconnectTimer?: number;
  private lastRestart = 0;
  private closed = false;
  // Our shared screen goes out in its own stream, so the other side can tell it from the camera by stream id.
  private readonly screenStream = new MediaStream();
  private screenVideo: RTCRtpSender | null = null;
  private screenAudio: RTCRtpSender | null = null;

  constructor(
    config: RTCConfiguration,
    localStream: MediaStream,
    private polite: boolean,
    private cb: PeerCallbacks,
  ) {
    this.pc = new RTCPeerConnection(config);
    const pc = this.pc;

    for (const track of localStream.getTracks()) pc.addTrack(track, localStream);
    // Always have both senders: screen share works without a camera, and a device picked later has one to go in.
    const sender = (kind: "audio" | "video") =>
      pc.getSenders().find((s) => s.track?.kind === kind) ??
      pc.addTransceiver(kind, { direction: "sendrecv", streams: [localStream] }).sender;
    this.senders = { audio: sender("audio"), video: sender("video") };
    preferRed(pc.getTransceivers().find((t) => t.sender === this.senders.audio));

    pc.ontrack = ({ track, streams }) => {
      const id = streams[0]?.id ?? "";
      const others = this.remoteStreams.get(id)?.getTracks().filter((t) => t.kind !== track.kind) ?? [];
      this.remoteStreams.set(id, new MediaStream([...others, track]));
      this.emitRemote();
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

  /** The id to announce in a `media` message while sharing. */
  get screenId(): string {
    return this.screenStream.id;
  }

  /** Swaps in another mic or camera without renegotiating. */
  async setTrack(kind: "audio" | "video", track: MediaStreamTrack | null): Promise<void> {
    await this.senders[kind].replaceTrack(track);
    // The bitrate cap can follow; the new picture shouldn't wait for it
    this.tuneSenders().catch((err) => console.warn("tuning senders failed", err));
  }

  /**
   * Sends the shared screen alongside the camera. The first share adds its transceivers (one renegotiation); after
   * that, stopping and starting again only swaps tracks in them. Screen audio gets its own transceiver the first time
   * a share has any.
   */
  async setScreen(video: MediaStreamTrack | null, audio: MediaStreamTrack | null): Promise<void> {
    this.screenVideo = await this.sendScreenTrack(this.screenVideo, video);
    this.screenAudio = await this.sendScreenTrack(this.screenAudio, audio);
    await this.tuneSenders();
  }

  private async sendScreenTrack(sender: RTCRtpSender | null, track: MediaStreamTrack | null): Promise<RTCRtpSender | null> {
    if (sender) {
      await sender.replaceTrack(track);
      return sender;
    }
    return track ? this.pc.addTransceiver(track, { direction: "sendonly", streams: [this.screenStream] }).sender : null;
  }

  /** Which incoming stream is the other side's screen; null once they stop sharing. */
  setRemoteScreen(id: string | null): void {
    if (id) this.remoteScreenIds.add(id);
    this.remoteScreenId = id;
    this.emitRemote();
  }

  /** Everything not announced as a screen is the camera and mic. The camera stream is only remade when its tracks change. */
  private emitRemote(): void {
    const camera = [...this.remoteStreams]
      .filter(([id]) => !this.remoteScreenIds.has(id))
      .flatMap(([, stream]) => stream.getTracks());
    const same = camera.length === this.remoteCamera?.getTracks().length && camera.every((t) => this.remoteCamera?.getTrackById(t.id));
    if (!same) this.remoteCamera = camera.length ? new MediaStream(camera) : null;
    const screen = (this.remoteScreenId && this.remoteStreams.get(this.remoteScreenId)) || null;
    this.cb.onRemoteStreams(this.remoteCamera, screen?.getVideoTracks().length ? screen : null);
  }

  /**
   * Caps video bitrate and tells the encoder what to give up when bandwidth runs short: resolution for the camera
   * (a smaller clean picture beats a blocky one), some of both for the screen (a playing video stays watchable instead
   * of freezing, and text is sharp again once it settles). While sharing, the camera shrinks to a small face so the
   * screen gets the bandwidth. The mic gets priority so voices survive congestion. Encodings only exist after
   * negotiation, so this runs after every description change.
   */
  private async tuneSenders(): Promise<void> {
    const sharing = !!this.screenVideo?.track;
    for (const transceiver of this.pc.getTransceivers()) {
      const sender = transceiver.sender;
      const params = sender.getParameters();
      if (!params.encodings?.length || sender === this.screenAudio) continue;
      const isScreen = sender === this.screenVideo;
      const isVideo = transceiver.receiver.track.kind === "video";
      const maxBitrate = isScreen ? SHARE_SCREEN_BITRATE : sharing ? SHARE_CAMERA_BITRATE : VIDEO_MAX_BITRATE;
      const scale = !isScreen && sharing ? downscaleTo(sender.track, SHARE_CAMERA_SHORT_SIDE) : 1;
      for (const enc of params.encodings) {
        if (isVideo) {
          enc.maxBitrate = maxBitrate;
          enc.scaleResolutionDownBy = scale;
          if (isScreen) enc.maxFramerate = SHARE_SCREEN_FPS;
        } else enc.priority = enc.networkPriority = "high";
      }
      if (isVideo) params.degradationPreference = isScreen ? "balanced" : "maintain-framerate";
      try {
        await sender.setParameters(params);
      } catch {
        // Some browsers reject priority/degradation fields; the bitrate cap is the part that matters.
        if (!isVideo) continue;
        const capped = sender.getParameters();
        capped.encodings.forEach((enc) => (enc.maxBitrate = maxBitrate));
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

/** How much to scale a camera down so its short side is about `short` pixels; 1 when it's already that small. */
function downscaleTo(track: MediaStreamTrack | null, short: number): number {
  const { width = 0, height = 0 } = track?.getSettings() ?? {};
  const side = Math.min(width, height);
  return side > short ? side / short : 1;
}

/**
 * Puts RED before Opus on the mic, so each packet also carries the previous one and a single lost packet leaves no
 * gap. The answer keeps only what both sides have, so a peer without RED gets plain Opus.
 */
function preferRed(transceiver: RTCRtpTransceiver | undefined): void {
  const codecs = RTCRtpReceiver.getCapabilities?.("audio")?.codecs ?? [];
  const isRed = (c: RTCRtpCodec) => c.mimeType.toLowerCase() === "audio/red";
  if (!transceiver?.setCodecPreferences || !codecs.some(isRed)) return;
  const rank = (c: RTCRtpCodec) => (isRed(c) ? 0 : c.mimeType.toLowerCase() === "audio/opus" ? 1 : 2);
  try {
    transceiver.setCodecPreferences([...codecs].sort((a, b) => rank(a) - rank(b)));
  } catch (err) {
    console.warn("couldn't prefer RED", err);
  }
}
