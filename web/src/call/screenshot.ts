import { formatDuration } from "./CallTimer";
import type { Corner } from "./useDragCorner";

const FRAME_TIMEOUT_MS = 3_000;

/** One video in a screenshot, drawn as it shows on screen. */
export interface Shot {
  stream: MediaStream;
  /** Clockwise degrees, as the viewer rotated the other person's video */
  rotation?: number;
  mirror?: boolean;
}

/**
 * Draws the stage's video at its full resolution, the small tiles as rounded tiles in the corners they sit in on
 * screen, and a small date stamp. Reads from fresh <video> elements rather than the ones on screen, so tile sizes and
 * animations don't matter. A tile that has no frame to give is left out rather than failing the picture.
 */
export async function captureCall({
  main,
  tiles,
  elapsedMs,
}: {
  main: Shot;
  tiles: (Shot & { corner: Corner })[];
  elapsedMs: number;
}): Promise<Blob> {
  const [stage, ...small] = await Promise.all([
    frameSource(main.stream),
    ...tiles.map((tile) => frameSource(tile.stream).catch(() => null)),
  ]);
  try {
    const turned = (main.rotation ?? 0) % 180 !== 0;
    const canvas = document.createElement("canvas");
    canvas.width = turned ? stage.videoHeight : stage.videoWidth;
    canvas.height = turned ? stage.videoWidth : stage.videoHeight;
    const ctx = canvas.getContext("2d")!;
    const short = Math.min(canvas.width, canvas.height);
    const margin = Math.round(short * 0.03);
    drawCover(ctx, stage, main, 0, 0, canvas.width, canvas.height);

    // How far along from its corner the next tile there goes
    const taken: Record<Corner, number> = { tl: 0, tr: 0, bl: 0, br: 0 };
    tiles.forEach((tile, i) => {
      const video = small[i];
      if (!video) return;
      const long = short * 0.3;
      const aspect = (tile.rotation ?? 0) % 180 ? video.videoHeight / video.videoWidth : video.videoWidth / video.videoHeight;
      const [w, h] = aspect >= 1 ? [long, long / aspect] : [long * aspect, long];
      const x = tile.corner.endsWith("l") ? margin : canvas.width - w - margin;
      const y = tile.corner.startsWith("t") ? margin + taken[tile.corner] : canvas.height - h - margin - taken[tile.corner];
      taken[tile.corner] += h + margin;
      ctx.save();
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, Math.min(w, h) * 0.08);
      ctx.clip();
      drawCover(ctx, video, tile, x, y, w, h);
      ctx.restore();
    });

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
    // Above a tile in the bottom-left corner
    ctx.fillText(stamp, margin, canvas.height - margin - taken.bl);

    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("could not encode screenshot"))), "image/jpeg", 0.92),
    );
  } finally {
    for (const video of [stage, ...small]) if (video) video.srcObject = null;
  }
}

/** Fills the box with the video, cropping what overflows, turned and mirrored as the shot says. */
function drawCover(ctx: CanvasRenderingContext2D, video: HTMLVideoElement, shot: Shot, x: number, y: number, w: number, h: number) {
  const rotation = shot.rotation ?? 0;
  // The box as the unturned video sees it
  const [bw, bh] = rotation % 180 ? [h, w] : [w, h];
  const scale = Math.max(bw / video.videoWidth, bh / video.videoHeight);
  const [sw, sh] = [bw / scale, bh / scale];
  ctx.save();
  ctx.translate(x + w / 2, y + h / 2);
  ctx.rotate((rotation * Math.PI) / 180);
  if (shot.mirror) ctx.scale(-1, 1);
  ctx.drawImage(video, (video.videoWidth - sw) / 2, (video.videoHeight - sh) / 2, sw, sh, -bw / 2, -bh / 2, bw, bh);
  ctx.restore();
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
