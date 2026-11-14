import { useRef } from "react";
import type { ChatMessage, Lang, LiveMessage } from "../../../shared/protocol";
import { useQuickTranslations } from "../translate/translator";
import { hasOtherScript, readable } from "./readable";
import type { ReactionRun } from "./useReactionRuns";

// Only the latest spoken lines are translated on this device: what's in view as they come
const QUICK = 8;

export interface FeedLine {
  key: string;
  mine: boolean;
  spoken: boolean;
  /** What showed when the line came: their spoken words translated on this device when it can be */
  first: string;
  /** What's shown once the server's translation is in */
  text: string;
  /** Showing the server's translation, which replaces `first` */
  translated: boolean;
  /** What they wrote or said, under the translation */
  original: string | null;
  /** When it was sent; null for what they're saying right now and for reactions */
  ts: number | null;
  /** What they're saying right now */
  live: boolean;
  /** A reaction, or a run of the same one */
  reaction: boolean;
}

/**
 * The chat as one list, reactions among the messages, with what the other person is saying right now at its end. A
 * finished spoken line takes the place of its live text. With `live` off there's no live text and nothing is
 * translated on this device.
 */
export function useFeed(
  messages: ChatMessage[],
  reactions: ReactionRun[],
  live: LiveMessage | null,
  myId: string | null,
  lang: Lang,
  liveOn: boolean,
): FeedLine[] {
  // The live text being shown gets its own key, which the line finishing it takes over, so it stays in place and keeps
  // its crossfade. Live ids come from the other side, so they're only matched against the live text just shown.
  const current = useRef<{ id: string; key: string } | null>(null);
  const finished = useRef(new Map<string, string>());
  const last = messages.at(-1);
  if (current.current && last?.liveId === current.current.id && !finished.current.has(last.id)) {
    finished.current.set(last.id, current.current.key);
    current.current = null;
  }
  const shownLive = liveOn ? live : null;
  if (shownLive && shownLive.id !== current.current?.id) {
    current.current = { id: shownLive.id, key: `live-${shownLive.id}-${Date.now()}` };
  }

  // Their spoken lines as said, which may need translating here
  const said = (m: ChatMessage) => m.spoken && m.from !== myId && hasOtherScript(m.text, lang);
  const foreign = messages.filter(said).map((m) => m.text);
  if (shownLive && hasOtherScript(shownLive.text, lang)) foreign.push(shownLive.text);
  const wanted = foreign.slice(-QUICK);
  const quick = useQuickTranslations(wanted, lang, liveOn);

  // Each line keeps its last translation until the new one is in, so growing live text doesn't flash the original
  const lastQuick = useRef(new Map<string, string>());
  const quickFor = (key: string, source: string): string | null | undefined => {
    if (!quick || !wanted.includes(source)) return null;
    const q = quick.get(source) ?? lastQuick.current.get(key);
    if (q !== undefined) lastQuick.current.set(key, q);
    return q;
  };

  const lines: FeedLine[] = [];
  let run = 0;
  const reactionsUpTo = (count: number) => {
    for (; run < reactions.length && reactions[run].after <= count; run++) {
      const r = reactions[run];
      const text = r.count > 1 ? `${r.emoji} ×${r.count}` : r.emoji;
      lines.push({ key: r.key, mine: r.mine, spoken: false, first: text, text, translated: false, original: null, ts: null, live: false, reaction: true });
    }
  };
  for (const [i, m] of messages.entries()) {
    reactionsUpTo(i);
    const key = finished.current.get(m.id) ?? m.id;
    const { text, original } = readable(m, myId, lang);
    const q = said(m) ? quickFor(key, m.text) : null;
    // One still waiting for its translation here waits the few ms it takes, rather than flash the original
    if (q === undefined) continue;
    const first = q ?? text;
    lines.push({
      key,
      mine: m.from === myId,
      spoken: !!m.spoken,
      first,
      text,
      translated: text !== m.text && text !== first,
      original,
      ts: m.ts,
      live: false,
      reaction: false,
    });
  }
  reactionsUpTo(Infinity);
  if (shownLive && current.current) {
    const key = current.current.key;
    const q = hasOtherScript(shownLive.text, lang) ? quickFor(key, shownLive.text) : null;
    if (q !== undefined) {
      const first = q ?? shownLive.text;
      lines.push({ key, mine: false, spoken: true, first, text: first, translated: false, original: null, ts: null, live: true, reaction: false });
    }
  }

  for (const key of lastQuick.current.keys()) if (!lines.some((l) => l.key === key)) lastQuick.current.delete(key);
  return lines;
}
