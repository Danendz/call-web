import { useEffect, useRef } from "react";

/** Live input level so you can see the right mic is picked up before joining. */
export function MicLevel({ stream }: { stream: MediaStream }) {
  const barRef = useRef<HTMLDivElement>(null);
  const track = stream.getAudioTracks()[0];
  useEffect(() => {
    if (!track) return;
    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    ctx.createMediaStreamSource(new MediaStream([track])).connect(analyser);
    const samples = new Uint8Array(analyser.fftSize);
    let frame = 0;
    const tick = () => {
      analyser.getByteTimeDomainData(samples);
      let peak = 0;
      for (const v of samples) peak = Math.max(peak, Math.abs(v - 128));
      if (barRef.current) barRef.current.style.transform = `scaleX(${Math.min(1, peak / 64)})`;
      frame = requestAnimationFrame(tick);
    };
    tick();
    // Browsers may start the context suspended until the first tap.
    const resume = () => void ctx.resume();
    resume();
    window.addEventListener("pointerdown", resume);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointerdown", resume);
      void ctx.close();
    };
  }, [track]);
  return (
    <div className="mic-level" aria-hidden>
      <div ref={barRef} />
    </div>
  );
}
