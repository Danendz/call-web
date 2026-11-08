export interface CallStats {
  path: "direct" | "relay" | "unknown";
  localType?: string; // host | srflx | prflx | relay
  remoteType?: string;
  protocol?: string; // udp | tcp
  relayProtocol?: string; // udp | tcp | tls, when we're the one relaying
  rttMs?: number;
  inKbps: number;
  outKbps: number;
  lossPct?: number; // what we receive
  lossOutPct?: number; // what we send, as reported back by the other side
  jitterMs?: number;
  video?: VideoStats;
  videoOut?: VideoStats & { limitedBy?: string };
  sendBudgetKbps?: number; // the browser's estimate of how much it can send
}

export interface VideoStats {
  width?: number;
  height?: number;
  fps?: number;
  codec?: string; // VP8, H264…
  implementation?: string; // the encoder or decoder doing the work, e.g. libvpx or a hardware one
}

interface Counters {
  ts: number;
  bytesIn: number;
  bytesOut: number;
  packetsIn: number;
  packetsLost: number;
}

type Stat = Record<string, any>;

/** Reads the active candidate pair and RTP counters. Pass the previous counters to get per-second rates. */
export async function sampleStats(pc: RTCPeerConnection, prev: Counters | null): Promise<{ stats: CallStats; counters: Counters }> {
  const report = await pc.getStats();
  const all: Stat[] = [];
  report.forEach((s) => all.push(s));
  const byId = (id?: string) => (id ? (report.get(id) as Stat | undefined) : undefined);

  const transport = all.find((s) => s.type === "transport" && s.selectedCandidatePairId);
  const pair =
    byId(transport?.selectedCandidatePairId) ??
    all.find((s) => s.type === "candidate-pair" && s.selected) ?? // Firefox
    all.find((s) => s.type === "candidate-pair" && s.nominated && s.state === "succeeded");
  const local = byId(pair?.localCandidateId);
  const remote = byId(pair?.remoteCandidateId);

  const codecOf = (s: Stat, implementation?: string) => ({
    codec: (byId(s.codecId)?.mimeType as string | undefined)?.split("/").pop(),
    implementation: implementation || undefined,
  });

  const counters: Counters = { ts: Date.now(), bytesIn: 0, bytesOut: 0, packetsIn: 0, packetsLost: 0 };
  let jitter: number | undefined;
  let video: CallStats["video"];
  let videoOut: CallStats["videoOut"];
  let lossOutPct: number | undefined;
  for (const s of all) {
    if (s.type === "inbound-rtp") {
      counters.bytesIn += s.bytesReceived ?? 0;
      counters.packetsIn += s.packetsReceived ?? 0;
      counters.packetsLost += s.packetsLost ?? 0;
      if (s.kind === "audio" && s.jitter !== undefined) jitter = s.jitter * 1000;
      if (s.kind === "video") {
        const next = { width: s.frameWidth, height: s.frameHeight, fps: s.framesPerSecond, ...codecOf(s, s.decoderImplementation) };
        video = bigger(video, next);
      }
    } else if (s.type === "outbound-rtp") {
      counters.bytesOut += s.bytesSent ?? 0;
      if (s.kind === "video" && s.frameWidth) {
        const limitedBy = s.qualityLimitationReason && s.qualityLimitationReason !== "none" ? s.qualityLimitationReason : undefined;
        const next = { width: s.frameWidth, height: s.frameHeight, fps: s.framesPerSecond, limitedBy, ...codecOf(s, s.encoderImplementation) };
        videoOut = bigger(videoOut, next);
      }
    } else if (s.type === "remote-inbound-rtp" && s.fractionLost !== undefined) {
      lossOutPct = Math.max(lossOutPct ?? 0, s.fractionLost * 100);
    }
  }

  let inKbps = 0;
  let outKbps = 0;
  let lossPct: number | undefined;
  if (prev) {
    const dt = (counters.ts - prev.ts) / 1000;
    if (dt > 0) {
      inKbps = Math.max(0, ((counters.bytesIn - prev.bytesIn) * 8) / 1000 / dt);
      outKbps = Math.max(0, ((counters.bytesOut - prev.bytesOut) * 8) / 1000 / dt);
    }
    const lost = counters.packetsLost - prev.packetsLost;
    const total = counters.packetsIn - prev.packetsIn + lost;
    if (total > 0) lossPct = Math.max(0, (lost / total) * 100);
  }

  const isRelay = local?.candidateType === "relay" || remote?.candidateType === "relay";
  return {
    counters,
    stats: {
      path: pair ? (isRelay ? "relay" : "direct") : "unknown",
      localType: local?.candidateType,
      remoteType: remote?.candidateType,
      protocol: local?.protocol,
      relayProtocol: local?.relayProtocol,
      rttMs: pair?.currentRoundTripTime !== undefined ? pair.currentRoundTripTime * 1000 : undefined,
      inKbps,
      outKbps,
      lossPct,
      lossOutPct,
      jitterMs: jitter,
      video,
      videoOut,
      sendBudgetKbps: pair?.availableOutgoingBitrate !== undefined ? pair.availableOutgoingBitrate / 1000 : undefined,
    },
  };
}

/** While sharing, the camera and the screen both send: the panel shows the bigger picture, which is the screen. */
function bigger<T extends VideoStats>(current: T | undefined, next: T): T {
  const pixels = (v: VideoStats) => (v.width ?? 0) * (v.height ?? 0);
  return current && pixels(current) >= pixels(next) ? current : next;
}
