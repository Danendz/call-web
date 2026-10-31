import { useEffect, useRef } from "react";

export function Video({ stream, className, muted, mirror }: { stream: MediaStream; className: string; muted?: boolean; mirror?: boolean }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    if (ref.current && ref.current.srcObject !== stream) ref.current.srcObject = stream;
  }, [stream]);
  return <video ref={ref} className={`${className} ${mirror ? "mirror" : ""}`} autoPlay playsInline muted={muted} />;
}
