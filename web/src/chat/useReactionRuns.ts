import { useEffect, useRef, useState } from "react";
import type { Reaction } from "../../../shared/protocol";
import type { ReactionEvent } from "../call/controller";
import { nextCombo, type Combo } from "../call/reactions/combo";

// The chat keeps this many reaction bubbles; older ones go
const MAX_RUNS = 50;

export interface ReactionRun {
  key: string;
  emoji: Reaction;
  mine: boolean;
  count: number;
  /** How many messages there were when it came: it's shown after them */
  after: number;
}

let nextKey = 0;

/**
 * Reactions as chat bubbles: a run of the same one from one side grows in place (❤️ ×5) until another message comes
 * or the run ends. Built from the reactions as they fly, so they're not stored and go with a reload.
 */
export function useReactionRuns(subscribe: (listener: (r: ReactionEvent) => void) => () => void, messageCount: number): ReactionRun[] {
  const [runs, setRuns] = useState<ReactionRun[]>([]);
  const messages = useRef(messageCount);
  messages.current = messageCount;

  useEffect(() => {
    const combos: Record<"mine" | "theirs", Combo | null> = { mine: null, theirs: null };
    const current: Record<"mine" | "theirs", string | null> = { mine: null, theirs: null };
    let list: ReactionRun[] = [];
    return subscribe(({ emoji, mine }) => {
      const side = mine ? "mine" : "theirs";
      const combo = (combos[side] = nextCombo(combos[side], emoji, performance.now()));
      const after = messages.current;
      const run = combo.count > 1 ? list.find((r) => r.key === current[side] && r.after === after) : undefined;
      if (run) {
        list = list.map((r) => (r === run ? { ...r, count: combo.count } : r));
      } else {
        const key = `reaction-${nextKey++}`;
        current[side] = key;
        list = [...list.slice(-(MAX_RUNS - 1)), { key, emoji, mine, count: 1, after }];
      }
      setRuns(list);
    });
  }, [subscribe]);

  return runs;
}
