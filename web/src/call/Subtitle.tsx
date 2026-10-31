import { useEffect, useRef, useState } from "react";
import type { ChatMessage, Lang } from "../../../shared/protocol";
import { readable } from "../chat/readable";

const SHOWN_MS = 6_000;

/** The other person's latest spoken line over the video, while the chat is closed. Its translation replaces it when it arrives. */
export function Subtitle({ messages, myId, lang }: { messages: ChatMessage[]; myId: string | null; lang: Lang }) {
  const last = [...messages].reverse().find((m) => m.spoken && m.from !== myId);
  const [visible, setVisible] = useState(false);
  // A line already in the chat when the subtitle appears (the chat was just closed) isn't shown again
  const shownFor = useRef(last?.id);

  // Restarts when a new line or its translation lands, not on every render
  useEffect(() => {
    if (!last || (shownFor.current === last.id && !visible)) return;
    shownFor.current = last.id;
    setVisible(true);
    const timer = setTimeout(() => setVisible(false), SHOWN_MS);
    return () => clearTimeout(timer);
  }, [last?.id, last?.translation]);

  if (!last || !visible) return null;
  return <div className="subtitle">{readable(last, myId, lang).text}</div>;
}
