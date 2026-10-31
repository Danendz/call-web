import { useState } from "react";

/** A line of text that fades in, and fades out still showing its last text instead of vanishing. */
export function Fade({ text, className }: { text: string | null; className: string }) {
  const [last, setLast] = useState(text);
  if (text && text !== last) setLast(text);
  return (
    <div className={`${className} fade ${text ? "shown" : ""}`} aria-hidden={!text} role="status">
      {text ?? last}
    </div>
  );
}
