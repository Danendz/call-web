import { useEffect, useRef, useSyncExternalStore } from "react";

// iOS refuses sound that doesn't start right after a tap, and the call's media arrives seconds after one.
// Such a video plays muted so its picture shows, until the next tap turns its sound on.
const soundBlocked = new Set<HTMLVideoElement>();
const listeners = new Set<() => void>();
const changed = () => listeners.forEach((l) => l());

function blockSound(v: HTMLVideoElement) {
  if (soundBlocked.size === 0) {
    // Events that count as a user gesture: pointerup for touch, keydown for keyboards
    document.addEventListener("pointerup", unblockSound, true);
    document.addEventListener("keydown", unblockSound, true);
  }
  soundBlocked.add(v);
  changed();
}

function unblock(v: HTMLVideoElement) {
  if (soundBlocked.delete(v)) changed();
  if (soundBlocked.size === 0) {
    document.removeEventListener("pointerup", unblockSound, true);
    document.removeEventListener("keydown", unblockSound, true);
  }
}

function unblockSound() {
  for (const v of soundBlocked) {
    unblock(v);
    v.muted = false;
    v.play().catch((err) => console.warn("sound still blocked", err));
  }
}

/** True while a video plays muted because the browser blocked its sound; any tap turns it on. */
export function useSoundBlocked(): boolean {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => soundBlocked.size > 0,
  );
}

/**
 * Plays a stream. onAspect reports width/height of the frames, again whenever the source changes shape.
 * rotation turns the picture clockwise in 90° steps; at 90/270 the `turned` class lets CSS swap the box's sides.
 * sinkId is the speaker its sound plays on, "" for the system's default; left out, the browser picks.
 */
export function Video({
  stream,
  className,
  muted,
  mirror,
  rotation = 0,
  sinkId,
  onAspect,
}: {
  stream: MediaStream;
  className: string;
  muted?: boolean;
  mirror?: boolean;
  rotation?: number;
  sinkId?: string;
  onAspect?: (aspect: number) => void;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  // React doesn't keep the muted property in sync after the first render
  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    if (muted) unblock(v);
    v.muted = !!muted || soundBlocked.has(v);
  }, [muted]);
  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    if (v.srcObject !== stream) v.srcObject = stream;
    // autoPlay alone fails silently when the sound is blocked, leaving a black, paused video
    v.play().catch((err) => {
      if (err?.name !== "NotAllowedError" || v.muted) return;
      v.muted = true;
      blockSound(v);
      v.play().catch((err) => console.warn("video blocked", err));
    });
  }, [stream]);
  useEffect(() => {
    const v = ref.current;
    return () => {
      if (v) unblock(v);
    };
  }, []);
  useEffect(() => {
    const v = ref.current;
    if (!v || sinkId === undefined || !("setSinkId" in v)) return;
    v.setSinkId(sinkId).catch((err) => console.warn("speaker not available", err));
  }, [sinkId]);

  const report = () => {
    const v = ref.current;
    if (v?.videoWidth && v.videoHeight) onAspect?.(v.videoWidth / v.videoHeight);
  };

  const turned = rotation % 180 !== 0;
  return (
    <video
      ref={ref}
      className={`${className} ${mirror ? "mirror" : ""} ${turned ? "turned" : ""}`}
      style={rotation ? { transform: `${turned ? "translate(-50%, -50%) " : ""}rotate(${rotation}deg)` } : undefined}
      autoPlay
      playsInline
      muted={muted}
      onLoadedMetadata={report}
      onResize={report}
    />
  );
}
