import { useEffect, useRef, useState } from "react";
import type { Reaction } from "../../../../shared/protocol";
import type { CallController } from "../controller";
import { BIG_EFFECT_EVERY, nextCombo, type Combo } from "./combo";
import { BIG_EFFECTS, playEffect } from "./effects";
import "./reactions.css";

// More than this on screen at once and the oldest go first
const MAX_FLOATERS = 30;
const BURST_PARTICLES = 6;

interface Floater {
  id: number;
  emoji: Reaction;
  mine: boolean;
  count: number;
  x: number; // % of the width
  drift: number; // px
}

let nextId = 0;

/** Reactions flying up over the video, both yours and theirs, and the full-screen effects of a combo. */
export function Reactions({ controller }: { controller: CallController }) {
  const [floaters, setFloaters] = useState<Floater[]>([]);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const combos: Record<"mine" | "theirs", Combo | null> = { mine: null, theirs: null };
    const still = window.matchMedia("(prefers-reduced-motion: reduce)");
    let stopEffect: (() => void) | null = null;

    const off = controller.onReaction(({ emoji, mine }) => {
      const side = mine ? "mine" : "theirs";
      const combo = (combos[side] = nextCombo(combos[side], emoji, performance.now()));
      // Yours rise on the right, theirs on the left, so a back-and-forth reads as one
      const x = mine ? 58 + Math.random() * 30 : 12 + Math.random() * 30;
      const floater = { id: nextId++, emoji, mine, count: combo.count, x, drift: 10 + Math.random() * 16 };
      setFloaters((list) => [...list.slice(-(MAX_FLOATERS - 1)), floater]);

      if (!mine) navigator.vibrate?.(25);
      const effect = BIG_EFFECTS[emoji];
      const canvas = canvasRef.current;
      if (effect && canvas && combo.count % BIG_EFFECT_EVERY === 0 && !stopEffect && !still.matches) {
        stopEffect = playEffect(canvas, effect, () => (stopEffect = null));
      }
    });
    return () => {
      off();
      stopEffect?.();
    };
  }, [controller]);

  const remove = (id: number) => setFloaters((list) => list.filter((f) => f.id !== id));

  return (
    <div className="reactions" aria-hidden>
      <canvas ref={canvasRef} />
      {floaters.map((f) => (
        <span
          key={f.id}
          className={`floater ${f.count > 1 ? "combo" : ""}`}
          style={{ left: `${f.x}%`, "--drift": `${f.drift}px`, "--grow": Math.min(f.count, 8) } as React.CSSProperties}
          onAnimationEnd={(e) => e.target === e.currentTarget && remove(f.id)}
        >
          <span className="sway">
            {f.emoji}
            {f.count > 1 && <b className="count">×{f.count}</b>}
          </span>
          {f.count > 1 && (
            <span className="burst">
              {Array.from({ length: BURST_PARTICLES }, (_, i) => (
                <i key={i} style={{ "--a": `${(360 / BURST_PARTICLES) * i}deg` } as React.CSSProperties}>
                  {f.emoji}
                </i>
              ))}
            </span>
          )}
        </span>
      ))}
    </div>
  );
}
