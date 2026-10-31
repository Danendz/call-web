import { useEffect, useRef, useState } from "react";
import type { ChatMessage, Lang } from "../../../shared/protocol";
import { readable } from "../chat/readable";
import { Fade } from "./Fade";

const SHOWN_MS = 6_000;

/** The other person's latest spoken line over the video, while the chat is closed. Its translation replaces it when it arrives. */
export function Subtitle({
  messages,
  myId,
  lang,
  chatOpen,
}: {
  messages: ChatMessage[];
  myId: string | null;
  lang: Lang;
  chatOpen: boolean;
}) {
  const last = [...messages].reverse().find((m) => m.spoken && m.from !== myId);
  const [visible, setVisible] = useState(false);
  // A line already read in the chat isn't shown again once the chat closes
  const shownFor = useRef(last?.id);

  // Restarts when a new line or its translation lands, not on every render
  useEffect(() => {
    if (chatOpen) {
      shownFor.current = last?.id;
      setVisible(false);
      return;
    }
    if (!last || (shownFor.current === last.id && !visible)) return;
    shownFor.current = last.id;
    setVisible(true);
    const timer = setTimeout(() => setVisible(false), SHOWN_MS);
    return () => clearTimeout(timer);
  }, [last?.id, last?.translation, chatOpen]);

  return <Fade className="subtitle" text={last && visible && !chatOpen ? readable(last, myId, lang).text : null} />;
}
