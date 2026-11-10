import { useEffect, useRef, useState } from "react";
import type { ChatMessage, Lang, LiveMessage } from "../../../shared/protocol";
import { hasOtherScript, readable } from "../chat/readable";
import { useQuickTranslations } from "../translate/translator";

const MAX_LINES = 4;
// After this long without a change a line fades; lines change in order, so in silence they go one by one
const IDLE_MS = 8_000;
// Lines past the shown ones stay in the page a while, so pushing one out or letting it go idle animates
const KEPT = MAX_LINES + 4;

interface Line {
  key: string;
  // What showed when the line came: their words or live text, translated on this device when it can be
  first: string;
  text: string;
  mine: boolean;
  // Showing the server's translation, which replaces what was shown first
  translated: boolean;
}

/**
 * The last few spoken lines over the video while chat is closed: the other person's in your language, your own as
 * said, and what they're saying right now as it comes. A finished line takes the place of its live text.
 */
export function Subtitles({
  messages,
  live,
  myId,
  lang,
  chatOpen,
}: {
  messages: ChatMessage[];
  live: LiveMessage | null;
  myId: string | null;
  lang: Lang;
  chatOpen: boolean;
}) {
  // Lines from before you joined, or already read in chat, aren't shown. Chat grows one message at a time; any other
  // change in length is the history arriving on joining.
  const now = Date.now();
  const read = useRef(new Set<string>());
  const before = useRef<number | null>(null);
  const reset = before.current === null || (messages.length !== before.current && messages.length !== before.current + 1);
  before.current = messages.length;
  if (chatOpen || reset) for (const m of messages) read.current.add(m.id);

  // The live text being shown gets its own key, which the line finishing it takes over, so it stays in place and keeps
  // its fade. Live ids come from the other side, so they're only matched against the live text just shown.
  const current = useRef<{ id: string; key: string } | null>(null);
  const finished = useRef(new Map<string, string>());
  const last = messages.at(-1);
  if (current.current && last?.liveId === current.current.id && !finished.current.has(last.id)) {
    finished.current.set(last.id, current.current.key);
    current.current = null;
  }
  if (live && live.id !== current.current?.id) current.current = { id: live.id, key: `live-${live.id}-${now}` };

  // Their lines as said, and what's shown once translated
  const said: { key: string; source: string; text: string; mine: boolean }[] = [];
  for (const m of messages) {
    if (!m.spoken || read.current.has(m.id)) continue;
    const key = finished.current.get(m.id) ?? m.id;
    said.push({ key, source: m.text, text: readable(m, myId, lang).text, mine: m.from === myId });
  }
  if (live && current.current && !chatOpen) {
    said.push({ key: current.current.key, source: live.text, text: live.text, mine: false });
  }

  // The other language, translated here as it comes. Each line keeps its last translation until the new one is in,
  // so growing live text doesn't flash the original; one with none yet waits the few ms it takes.
  const foreign = said.filter((l) => !l.mine && hasOtherScript(l.source, lang)).map((l) => l.source);
  const quick = useQuickTranslations(foreign.slice(-KEPT), lang, true);
  const lastQuick = useRef(new Map<string, string>());
  const lines: Line[] = [];
  for (const l of said) {
    const needed = quick && !l.mine && hasOtherScript(l.source, lang);
    const q = needed ? (quick.get(l.source) ?? lastQuick.current.get(l.key)) : undefined;
    if (needed && q === undefined) continue;
    if (q !== undefined) lastQuick.current.set(l.key, q);
    const first = q ?? l.source;
    lines.push({ key: l.key, first, text: l.text, mine: l.mine, translated: l.text !== l.source && l.text !== first });
  }
  const kept = lines.slice(-KEPT);

  // When each line last changed, by its key
  const changed = useRef(new Map<string, { text: string; at: number }>());
  for (const l of kept) {
    const seen = changed.current.get(l.key);
    if (seen?.text !== l.text) changed.current.set(l.key, { text: l.text, at: now });
  }
  for (const key of changed.current.keys()) if (!kept.some((l) => l.key === key)) changed.current.delete(key);
  for (const key of lastQuick.current.keys()) if (!said.some((l) => l.key === key)) lastQuick.current.delete(key);
  const fresh = kept.filter((l) => (changed.current.get(l.key)?.at ?? 0) + IDLE_MS > now);
  // Hidden while chat is open, where the same lines are
  const shown = new Set(chatOpen ? [] : fresh.slice(-MAX_LINES).map((l) => l.key));

  // Renders again when the next shown line goes idle
  const [, setTick] = useState(0);
  const nextIdle = Math.min(...fresh.map((l) => (changed.current.get(l.key)?.at ?? 0) + IDLE_MS));
  useEffect(() => {
    if (!Number.isFinite(nextIdle)) return;
    const timer = setTimeout(() => setTick((t) => t + 1), Math.max(0, nextIdle - Date.now()));
    return () => clearTimeout(timer);
  }, [nextIdle]);

  return (
    <div className="subtitles" aria-live="polite">
      {kept.map((l) => (
        <div key={l.key} className={`subtitle-line ${shown.has(l.key) ? "shown" : ""} ${l.mine ? "mine" : ""}`}>
          <div>
            {/* The server's translation crossfades over what was shown first, both in one cell so it doesn't jump */}
            <span className={`subtitle-bubble ${l.translated ? "translated" : ""}`}>
              <span className="subtitle-first">{l.first}</span>
              <span className="subtitle-text">{l.translated ? l.text : ""}</span>
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}
