import type { Reaction } from "../../../../shared/protocol";

export type BigEffect = "hearts" | "confetti" | "flames";

/** The reactions that fill the screen at a combo. */
export const BIG_EFFECTS: Partial<Record<Reaction, BigEffect>> = { "❤️": "hearts", "🎉": "confetti", "🔥": "flames" };

const EMOJI_FONT = `system-ui, "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif`;
const CONFETTI_COLORS = ["#ff5d73", "#ffc94d", "#5dd39e", "#5aa9ff", "#b57bff", "#ff9a4d"];
const GRAVITY = 520; // px/s², confetti only

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  rot: number;
  spin: number;
  size: number;
  born: number; // s after the effect started; it appears then
  life: number; // s
  sway: number; // phase, so neighbors don't move in step
  glyph?: HTMLCanvasElement;
  color?: string;
}

const rand = (min: number, max: number) => min + Math.random() * (max - min);

/** Draws an emoji once, so each frame copies pixels instead of laying out text. */
function glyph(emoji: string, px: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = Math.ceil(px * 1.25);
  const ctx = c.getContext("2d");
  if (ctx) {
    ctx.font = `${px}px ${EMOJI_FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(emoji, c.width / 2, c.height / 2);
  }
  return c;
}

function spawn(effect: BigEffect, w: number, h: number, dpr: number): Particle[] {
  const out: Particle[] = [];
  if (effect === "hearts") {
    const heart = glyph("❤️", 48 * dpr);
    for (let i = 0; i < 46; i++) {
      out.push({
        x: rand(0, w),
        y: h + rand(20, 80) * dpr,
        vx: 0,
        vy: -rand(220, 460) * dpr,
        rot: rand(-0.3, 0.3),
        spin: rand(-0.6, 0.6),
        size: rand(0.45, 1.1),
        born: rand(0, 1.1),
        life: rand(2, 2.8),
        sway: rand(0, Math.PI * 2),
        glyph: heart,
      });
    }
  } else if (effect === "confetti") {
    for (let i = 0; i < 150; i++) {
      out.push({
        x: rand(0, w),
        y: -rand(10, 120) * dpr,
        vx: rand(-90, 90) * dpr,
        vy: rand(60, 260) * dpr,
        rot: rand(0, Math.PI * 2),
        spin: rand(-9, 9),
        size: rand(6, 11) * dpr,
        born: rand(0, 0.7),
        life: rand(2.2, 3),
        sway: rand(0, Math.PI * 2),
        color: CONFETTI_COLORS[i % CONFETTI_COLORS.length],
      });
    }
  } else {
    const fire = glyph("🔥", 56 * dpr);
    for (let i = 0; i < 60; i++) {
      out.push({
        x: rand(-0.05, 1.05) * w,
        y: h + rand(10, 40) * dpr,
        vx: rand(-20, 20) * dpr,
        vy: -rand(160, 380) * dpr,
        rot: rand(-0.2, 0.2),
        spin: 0,
        size: rand(0.6, 1.4),
        born: rand(0, 1.2),
        life: rand(1, 1.7),
        sway: rand(0, Math.PI * 2),
        glyph: fire,
      });
    }
  }
  return out;
}

/** Plays a full-screen effect on `canvas` until its last particle is gone; returns a stop. */
export function playEffect(canvas: HTMLCanvasElement, effect: BigEffect, onDone: () => void): () => void {
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    onDone();
    return () => {};
  }
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = (canvas.width = Math.round(canvas.clientWidth * dpr));
  const h = (canvas.height = Math.round(canvas.clientHeight * dpr));
  const particles = spawn(effect, w, h, dpr);
  const start = performance.now();
  let last = start;
  let frame = 0;

  const draw = (now: number) => {
    const t = (now - start) / 1000;
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now;
    ctx.clearRect(0, 0, w, h);
    let alive = 0;
    for (const p of particles) {
      const age = t - p.born;
      if (age < 0) {
        alive++;
        continue;
      }
      if (age > p.life) continue;
      alive++;
      if (p.color) p.vy += GRAVITY * dpr * dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.rot += p.spin * dt;
      const k = age / p.life;
      // In fast, out slow: full for most of the life, then fading
      ctx.globalAlpha = Math.min(1, age * 6) * (k > 0.7 ? 1 - (k - 0.7) / 0.3 : 1);
      ctx.save();
      ctx.translate(p.x + Math.sin(age * 3 + p.sway) * 14 * dpr, p.y);
      ctx.rotate(p.rot);
      if (p.glyph) {
        // Flames grow then shrink as they burn out
        const s = effect === "flames" ? p.size * Math.sin(Math.PI * Math.min(k * 1.2, 1)) : p.size;
        const gw = p.glyph.width * s;
        ctx.drawImage(p.glyph, -gw / 2, -gw / 2, gw, gw);
      } else if (p.color) {
        ctx.fillStyle = p.color;
        // A flat strip seen turning: its width follows the spin
        ctx.fillRect(-p.size / 2, -p.size / 4, p.size * Math.abs(Math.cos(p.rot * 1.7)) + 1, p.size / 2);
      }
      ctx.restore();
    }
    ctx.globalAlpha = 1;
    if (alive > 0) frame = requestAnimationFrame(draw);
    else {
      ctx.clearRect(0, 0, w, h);
      onDone();
    }
  };
  frame = requestAnimationFrame(draw);
  return () => {
    cancelAnimationFrame(frame);
    ctx.clearRect(0, 0, w, h);
  };
}
