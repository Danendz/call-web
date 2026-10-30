export interface CallStats {
  path: "direct" | "relay" | "unknown";
  localType?: string; // host | srflx | prflx | relay
  remoteType?: string;
  protocol?: string; // udp | tcp
  relayProtocol?: string; // udp | tcp | tls, when we're the one relaying
  rttMs?: number;
  inKbps: number;
  outKbps: number;
  lossPct?: number;
  jitterMs?: number;
  video?: { width?: number; height?: number; fps?: number };
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

  const counters: Counters = { ts: Date.now(), bytesIn: 0, bytesOut: 0, packetsIn: 0, packetsLost: 0 };
  let jitter: number | undefined;
  let video: CallStats["video"];
  for (const s of all) {
    if (s.type === "inbound-rtp") {
      counters.bytesIn += s.bytesReceived ?? 0;
      counters.packetsIn += s.packetsReceived ?? 0;
      counters.packetsLost += s.packetsLost ?? 0;
      if (s.kind === "audio" && s.jitter !== undefined) jitter = s.jitter * 1000;
      if (s.kind === "video") video = { width: s.frameWidth, height: s.frameHeight, fps: s.framesPerSecond };
    } else if (s.type === "outbound-rtp") {
      counters.bytesOut += s.bytesSent ?? 0;
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
      jitterMs: jitter,
      video,
    },
  };
}
