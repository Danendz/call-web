import type { Reaction } from "../../../../shared/protocol";

// Taps of the same reaction closer than this make a combo. Each side counts the other's from when they arrive, so
// the gap leaves room for network jitter.
const COMBO_GAP_MS = 800;
// Every this many in a row, a reaction that has a big effect plays it
export const BIG_EFFECT_EVERY = 5;

export interface Combo {
  emoji: Reaction;
  count: number;
  at: number;
}

/** One side's run of the same reaction, after another one at `at` ms. */
export function nextCombo(prev: Combo | null, emoji: Reaction, at: number): Combo {
  const continues = prev !== null && prev.emoji === emoji && at - prev.at <= COMBO_GAP_MS;
  return { emoji, count: continues ? prev.count + 1 : 1, at };
}
