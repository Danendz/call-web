import { Mic, SendHorizontal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { MAX_CHAT_LENGTH, type ChatMessage, type Lang } from "../../../shared/protocol";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import "./chat.css";
import { EmojiPicker } from "./EmojiPicker";
import { readable } from "./readable";

interface Props {
  open: boolean;
  messages: ChatMessage[];
  myId: string | null;
  lang: Lang;
  /** Put the cursor in the input on open. Off on phones, where it would pop the keyboard over the video. */
  focusOnOpen: boolean;
  onSend: (text: string) => boolean;
  onClose: () => void;
}

/** Stays mounted while closed so it can slide out, and so the draft survives closing it. */
export function ChatPanel({ open, messages, myId, lang, focusOnOpen, onSend, onClose }: Props) {
  const t = useT();
  const [draft, setDraft] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: "smooth" });
  }, [messages.length, open]);

  useEffect(() => {
    if (open && focusOnOpen) inputRef.current?.focus({ preventScroll: true });
  }, [open, focusOnOpen]);

  // At the caret while typing, else at the end
  const insert = (emoji: string) => {
    const input = inputRef.current;
    const typing = input !== null && document.activeElement === input;
    const start = typing ? (input.selectionStart ?? draft.length) : draft.length;
    const end = typing ? (input.selectionEnd ?? start) : start;
    const next = draft.slice(0, start) + emoji + draft.slice(end);
    if (next.length > MAX_CHAT_LENGTH) return;
    setDraft(next);
    if (typing) requestAnimationFrame(() => input.setSelectionRange(start + emoji.length, start + emoji.length));
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (text && onSend(text)) setDraft("");
  };

  return (
    <aside className="chat" aria-hidden={!open}>
      <div className="chat-inner">
        <header>
          <span>{t.chat}</span>
          <IconButton label={t.closeChat} variant="small" onClick={onClose}>
            <X size={18} />
          </IconButton>
        </header>
        <div className="messages" ref={listRef}>
          {messages.length === 0 && <p className="muted empty">{t.noMessages}</p>}
          {messages.map((m) => {
            const { text, original } = readable(m, myId, lang);
            return (
              <div key={m.id} className={`msg ${m.from === myId ? "mine" : ""}`}>
                <span className="text">
                  {m.spoken && <Mic size={13} className="spoken" aria-label={t.spoken} />}
                  {text}
                </span>
                {original && <span className="original">{original}</span>}
                <time>{new Date(m.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>
              </div>
            );
          })}
        </div>
        <form onSubmit={submit}>
          <EmojiPicker onPick={insert} />
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
      </div>
    </aside>
  );
}
