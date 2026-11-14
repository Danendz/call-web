import { Mic, SendHorizontal } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { MAX_CHAT_LENGTH, type ChatMessage, type Lang, type LiveMessage } from "../../../shared/protocol";
import { useT } from "../ui/i18n";
import "./chat.css";
import { useFeed } from "./useFeed";
import type { ReactionRun } from "./useReactionRuns";

// Closer to the bottom than this counts as at the bottom, so new lines keep it scrolled down
const STICK_PX = 24;

interface Props {
  open: boolean;
  messages: ChatMessage[];
  reactions: ReactionRun[];
  live: LiveMessage | null;
  myId: string | null;
  lang: Lang;
  /** What the other person is saying, as they say it, and their spoken lines translated on this device */
  liveOn: boolean;
  /** Put the cursor in the input when shown again. Off on phones, where it would pop the keyboard over the video. */
  focusOnOpen: boolean;
  onSend: (text: string) => boolean;
}

/**
 * The chat over the video: see-through bubbles in the lower left whose oldest fade out at the top, scrolled back for
 * the rest. Stays mounted while hidden so it can fade, and so the draft survives hiding it.
 */
export function ChatOverlay({ open, messages, reactions, live, myId, lang, liveOn, focusOnOpen, onSend }: Props) {
  const t = useT();
  const lines = useFeed(messages, reactions, live, myId, lang, liveOn);
  const [draft, setDraft] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const stuck = useRef(true);

  // Follows new lines and growing live text, unless scrolled back to read
  const lastLine = lines.at(-1);
  useLayoutEffect(() => {
    const list = listRef.current;
    if (list && stuck.current) list.scrollTop = list.scrollHeight;
  }, [lines.length, lastLine?.first, lastLine?.text, open]);

  const onScroll = () => {
    const list = listRef.current;
    if (list) stuck.current = list.scrollHeight - list.scrollTop - list.clientHeight < STICK_PX;
  };

  // Not when the call starts with it shown, only when brought back
  const shownBefore = useRef(open);
  useEffect(() => {
    if (open && !shownBefore.current && focusOnOpen) inputRef.current?.focus({ preventScroll: true });
    shownBefore.current = open;
  }, [open, focusOnOpen]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || !onSend(text)) return;
    setDraft("");
    stuck.current = true;
  };

  return (
    <aside className={`chat ${open ? "open" : ""}`} aria-label={t.chat} aria-hidden={!open}>
      <div className="messages" ref={listRef} onScroll={onScroll}>
        {lines.length === 0 && <p className="empty">{t.noMessages}</p>}
        {lines.map((l) => (
          <div
            key={l.key}
            className={`msg ${l.mine ? "mine" : ""} ${l.live ? "live" : ""} ${l.reaction ? "reaction" : ""}`}
            title={l.ts === null ? undefined : new Date(l.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
          >
            <span className="text">
              {l.spoken && <Mic size={13} className="spoken" aria-label={t.spoken} />}
              {/* The server's translation crossfades over what was shown first, both in one cell so it doesn't jump */}
              <span className={`swap ${l.translated ? "translated" : ""}`}>
                <span className="first">{l.first}</span>
                <span className="final" aria-hidden={!l.translated}>
                  {l.translated && l.text}
                </span>
              </span>
            </span>
            {l.original && <span className="original">{l.original}</span>}
          </div>
        ))}
      </div>
      {/* Stays while typing even when the controls fade */}
      <form onSubmit={submit} className={draft ? "drafting" : ""}>
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={t.message}
          maxLength={MAX_CHAT_LENGTH}
          enterKeyHint="send"
        />
        <button type="submit" className="send" disabled={!draft.trim()} aria-label={t.send}>
          <SendHorizontal size={18} />
        </button>
      </form>
    </aside>
  );
}
