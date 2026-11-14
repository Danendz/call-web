// Which video codec a call uses, like the native app's `codec.rs`. In "auto", H.264 goes first where the browser
// encodes it in hardware, because encoding VP8 in software is most of a call's CPU; without hardware it's left out, so
// this side never runs a software H.264 encoder. Where the browser can't say, its own order stays. "vp8" leaves out
// everything else, which moves the whole call to VP8: the way out when H.264 misbehaves.
//
// The browser sends with the first codec the other side's description lists, and our own order only says what we'd
// like to receive. So a codec we leave out is also taken out of the other side's offer before it's applied
// (filterOffer); otherwise we'd still send it whenever they offer it first.

export type VideoCodec = "auto" | "h264" | "vp8";

/** Whether the browser encodes H.264 in hardware; null until it says, or where it can't. */
let hardwareH264: boolean | null = null;

/** Resolves once the browser has said whether it encodes H.264 in hardware. */
export const hardwareKnown: Promise<void> = (async () => {
  try {
    const info = await navigator.mediaCapabilities?.encodingInfo({
      type: "webrtc",
      video: {
        contentType: "video/H264;level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        width: 1280,
        height: 720,
        bitrate: 2_000_000,
        framerate: 30,
      },
    });
    if (info) hardwareH264 = info.supported && info.powerEfficient;
  } catch {
    // No "webrtc" type here (older Safari): leave the browser's own order
  }
})();

const REPAIR = ["rtx", "red", "ulpfec", "flexfec-03"];

/** Whether a codec, by its lowercase name, may be used at all. */
function allowed(name: string, pick: VideoCodec): boolean {
  if (name === "h264") return pick === "h264" || (pick === "auto" && hardwareH264 !== false);
  return name === "vp8" || REPAIR.includes(name) || pick !== "vp8";
}

/** Whether a parameter of an fmtp line starts with `param`. */
const fmtpHas = (line: string | undefined, param: string) =>
  !!line?.split(";").some((p) => p.trim().toLowerCase().startsWith(param));

function rank(c: RTCRtpCodec): number {
  const name = c.mimeType.split("/").pop()?.toLowerCase();
  if (name === "h264") {
    // Constrained baseline in packetization mode 1: what every hardware encoder and browser can do, at any level
    if (!fmtpHas(c.sdpFmtpLine, "packetization-mode=1")) return 2;
    return fmtpHas(c.sdpFmtpLine, "profile-level-id=42e0") ? 0 : 1;
  }
  return name === "vp8" ? 3 : 4;
}

/** Sets the codec order on every video transceiver, the other side's too. Takes effect at the next negotiation. */
export function applyCodecs(pc: RTCPeerConnection, pick: VideoCodec): void {
  const all = RTCRtpReceiver.getCapabilities?.("video")?.codecs ?? [];
  // An empty list puts back the browser's own order
  const own = pick === "auto" && hardwareH264 === null;
  const codecs = own
    ? []
    : all.filter((c) => allowed(c.mimeType.split("/").pop()?.toLowerCase() ?? "", pick)).sort((a, b) => rank(a) - rank(b));
  for (const transceiver of pc.getTransceivers()) {
    if (transceiver.receiver.track.kind !== "video" || !transceiver.setCodecPreferences) continue;
    try {
      transceiver.setCodecPreferences(codecs);
    } catch (err) {
      console.warn("couldn't set the video codecs", err);
    }
  }
}

/**
 * The other side's offer without the video codecs we leave out, and the retransmission entries that went with them.
 * A video section that would be left with no codec at all is kept whole: a codec we'd rather not use beats no video.
 */
export function filterOffer(sdp: string, pick: VideoCodec): string {
  const newline = sdp.includes("\r\n") ? "\r\n" : "\n";
  const sections: string[][] = [[]];
  for (const line of sdp.split(/\r?\n/)) {
    if (line.startsWith("m=")) sections.push([]);
    sections[sections.length - 1].push(line);
  }
  return sections.flatMap((s) => (s[0]?.startsWith("m=video ") ? filterSection(s, pick) : s)).join(newline);
}

function filterSection(section: string[], pick: VideoCodec): string[] {
  const [m, ...rest] = section;
  const fields = m.split(" ");
  const head = fields.slice(0, 3);
  const types = fields.slice(3);
  const attr = (prefix: string, pt: string) =>
    rest.find((l) => l.startsWith(`${prefix}${pt} `))?.slice(prefix.length + pt.length + 1);
  const name = (pt: string) => attr("a=rtpmap:", pt)?.split("/")[0].toLowerCase() ?? "";
  const keptCodec = (pt: string) => name(pt) !== "rtx" && allowed(name(pt), pick);
  const keep = types.filter((pt) => {
    if (name(pt) !== "rtx") return keptCodec(pt);
    // Retransmission for a codec that stays
    const apt = attr("a=fmtp:", pt)
      ?.split(";")
      .map((p) => p.trim())
      .find((p) => p.startsWith("apt="));
    return !!apt && keptCodec(apt.slice(4));
  });
  if (keep.length === types.length || keep.every((pt) => REPAIR.includes(name(pt)))) return section;
  const dropped = (line: string) =>
    ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"].some((prefix) => {
      if (!line.startsWith(prefix)) return false;
      const pt = line.slice(prefix.length).split(" ")[0];
      return types.includes(pt) && !keep.includes(pt);
    });
  return [[...head, ...keep].join(" "), ...rest.filter((l) => !dropped(l))];
}
