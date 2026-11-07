import { useEffect, useRef } from "react";

/**
 * Plays a stream. onAspect reports width/height of the frames, again whenever the source changes shape.
 * rotation turns the picture clockwise in 90° steps; at 90/270 the `turned` class lets CSS swap the box's sides.
 */
export function Video({
  stream,
  className,
  muted,
  mirror,
  rotation = 0,
  onAspect,
}: {
  stream: MediaStream;
  className: string;
  muted?: boolean;
  mirror?: boolean;
  rotation?: number;
  onAspect?: (aspect: number) => void;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    if (ref.current && ref.current.srcObject !== stream) ref.current.srcObject = stream;
  }, [stream]);

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
