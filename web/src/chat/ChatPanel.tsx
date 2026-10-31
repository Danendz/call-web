import { useEffect, useRef, useState } from "react";
import { MAX_CHAT_LENGTH, type ChatMessage, type Lang } from "../../../shared/protocol";
import { readable } from "./readable";

interface Props {
  messages: ChatMessage[];
  myId: string | null;
  lang: Lang;
  onSend: (text: string) => boolean;
  onClose: () => void;
}

export function ChatPanel({ messages, myId, lang, onSend, onClose }: Props) {
  const [draft, setDraft] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [messages.length]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (text && onSend(text)) setDraft("");
  };

  return (
    <aside className="chat">
      <header>
        <span>Chat</span>
        <button className="link" onClick={onClose} aria-label="Close chat">
          ✕
        </button>
      </header>
      <div className="messages" ref={listRef}>
        {messages.length === 0 && <p className="muted empty">No messages yet. They disappear when the room closes.</p>}
        {messages.map((m) => {
          const { text, original } = readable(m, myId, lang);
          return (
            <div key={m.id} className={`msg ${m.from === myId ? "mine" : ""}`}>
              <span className="text">
                {m.spoken && <span title="Spoken">🎙 </span>}
                {text}
              </span>
              {original && <span className="original">{original}</span>}
              <time>{new Date(m.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>
            </div>
          );
        })}
      </div>
      <form onSubmit={submit}>
        <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Message" maxLength={MAX_CHAT_LENGTH} enterKeyHint="send" />
        <button type="submit" disabled={!draft.trim()}>
          Send
        </button>
      </form>
    </aside>
  );
}
