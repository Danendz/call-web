import { useEffect, useRef } from "react";

/** Plays a stream. onAspect reports width/height of the frames, again whenever the source changes shape. */
export function Video({
  stream,
  className,
  muted,
  mirror,
  onAspect,
}: {
  stream: MediaStream;
  className: string;
  muted?: boolean;
  mirror?: boolean;
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

  return (
    <video
      ref={ref}
      className={`${className} ${mirror ? "mirror" : ""}`}
      autoPlay
      playsInline
      muted={muted}
      onLoadedMetadata={report}
      onResize={report}
    />
  );
}
