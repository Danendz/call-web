import { formatDuration } from "./CallTimer";

const FRAME_TIMEOUT_MS = 3_000;

/**
 * Draws the other person's frame at its full resolution, your camera as a rounded tile in the corner, and a small
 * date stamp. Reads from fresh <video> elements rather than the ones on screen, so the layout (swap, tile size)
 * doesn't matter.
 */
export async function captureCall({
  remote,
  local,
  rotation,
  mirrorLocal,
  elapsedMs,
}: {
  remote: MediaStream;
  /** null when your camera is off */
  local: MediaStream | null;
  /** Clockwise degrees, as the viewer rotated the other person's video */
  rotation: number;
  mirrorLocal: boolean;
  elapsedMs: number;
}): Promise<Blob> {
  const [them, me] = await Promise.all([frameSource(remote), local ? frameSource(local) : null]);
  try {
    const turned = rotation % 180 !== 0;
    const canvas = document.createElement("canvas");
    canvas.width = turned ? them.videoHeight : them.videoWidth;
    canvas.height = turned ? them.videoWidth : them.videoHeight;
    const ctx = canvas.getContext("2d")!;
    const short = Math.min(canvas.width, canvas.height);
    const margin = Math.round(short * 0.03);

    ctx.save();
    ctx.translate(canvas.width / 2, canvas.height / 2);
    ctx.rotate((rotation * Math.PI) / 180);
    ctx.drawImage(them, -them.videoWidth / 2, -them.videoHeight / 2);
    ctx.restore();

    if (me) {
      const long = short * 0.3;
      const aspect = me.videoWidth / me.videoHeight;
      const [w, h] = aspect >= 1 ? [long, long / aspect] : [long * aspect, long];
      const x = canvas.width - w - margin;
      const y = canvas.height - h - margin;
      ctx.save();
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, Math.min(w, h) * 0.08);
      ctx.clip();
      if (mirrorLocal) {
        ctx.translate(x + w, y);
        ctx.scale(-1, 1);
        ctx.drawImage(me, 0, 0, w, h);
      } else {
        ctx.drawImage(me, x, y, w, h);
      }
      ctx.restore();
    }

    const at = new Date();
    const stamp = [
      at.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }),
      at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }),
      formatDuration(elapsedMs),
    ].join(" · ");
    ctx.font = `600 ${Math.max(14, Math.round(short * 0.03))}px system-ui, -apple-system, sans-serif`;
    ctx.textBaseline = "bottom";
    ctx.fillStyle = "rgba(255, 255, 255, 0.85)";
    ctx.shadowColor = "rgba(0, 0, 0, 0.6)";
    ctx.shadowBlur = Math.round(short * 0.01);
    ctx.fillText(stamp, margin, canvas.height - margin);

    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("could not encode screenshot"))), "image/jpeg", 0.92),
    );
  } finally {
    them.srcObject = null;
    if (me) me.srcObject = null;
  }
}

/** A detached, muted video playing the stream, resolved once it has a frame to draw. */
function frameSource(stream: MediaStream): Promise<HTMLVideoElement> {
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  return new Promise((resolve, reject) => {
    // A track that sends no frames (camera paused on the other end) never fires loadeddata
    const timeout = setTimeout(() => reject(new Error("no video frame")), FRAME_TIMEOUT_MS);
    video.onloadeddata = () => {
      clearTimeout(timeout);
      resolve(video);
    };
    video.play().catch((err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });
}

/**
 * Phones get the share sheet, which offers Save to Photos (a download on iOS lands in Files). Everyone else, or a
 * phone where sharing is refused, gets a plain download.
 */
export async function saveImage(blob: Blob, name: string): Promise<void> {
  const file = new File([blob], name, { type: blob.type });
  if (window.matchMedia("(pointer: coarse)").matches && navigator.canShare?.({ files: [file] })) {
    try {
      await navigator.share({ files: [file] });
      return;
    } catch (err) {
      if ((err as DOMException).name === "AbortError") return; // closed the sheet on purpose
      // NotAllowedError: the tap expired while the picture was drawn; download instead
    }
  }
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** call-20261107-1554.jpg */
export function screenshotName(at = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `call-${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}-${pad(at.getHours())}${pad(at.getMinutes())}.jpg`;
}
