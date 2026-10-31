import { useEffect, useRef } from "react";

/** Live input level as a ring around the mic button, so you can see the right mic is picked up before joining. */
export function MicLevel({ stream }: { stream: MediaStream }) {
  const ringRef = useRef<HTMLSpanElement>(null);
  const track = stream.getAudioTracks()[0];
  useEffect(() => {
    if (!track) return;
    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    ctx.createMediaStreamSource(new MediaStream([track])).connect(analyser);
    const samples = new Uint8Array(analyser.fftSize);
    let frame = 0;
    let level = 0;
    const tick = () => {
      analyser.getByteTimeDomainData(samples);
      let peak = 0;
      for (const v of samples) peak = Math.max(peak, Math.abs(v - 128));
      // Jumps up at once, falls back slowly, so the ring doesn't flicker
      level = Math.max(Math.min(1, peak / 64), level * 0.88);
      ringRef.current?.style.setProperty("--level", level.toFixed(3));
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
  return <span ref={ringRef} className="mic-level" aria-hidden />;
}
