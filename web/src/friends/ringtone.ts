// The incoming call's ring: two short tones every few seconds, made with WebAudio so there's no file to ship.
// Browsers may keep it silent until the page has been clicked once.

const PATTERN_MS = 3_000;

let ctx: AudioContext | null = null;
let timer: number | undefined;

function beep(at: number, freq: number): void {
  if (!ctx) return;
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0, at);
  gain.gain.linearRampToValueAtTime(0.15, at + 0.02);
  gain.gain.linearRampToValueAtTime(0, at + 0.35);
  osc.connect(gain).connect(ctx.destination);
  osc.start(at);
  osc.stop(at + 0.4);
}

function ringOnce(): void {
  if (!ctx) return;
  const now = ctx.currentTime;
  beep(now, 880);
  beep(now + 0.45, 660);
}

export function startRinging(): void {
  if (timer !== undefined) return;
  try {
    ctx ??= new AudioContext();
    void ctx.resume();
  } catch {
    return;
  }
  ringOnce();
  timer = window.setInterval(ringOnce, PATTERN_MS);
}

export function stopRinging(): void {
  clearInterval(timer);
  timer = undefined;
}
