import { Smile } from "lucide-react";
import { useRef, useState } from "react";
import { useDismiss } from "../call/useDismiss";
import { useT } from "../ui/i18n";
import { EMOJI_GROUPS } from "./emoji";

/** The chat's emoji button and its grid above the input. Stays open so several can be picked in a row. */
export function EmojiPicker({ onPick }: { onPick: (emoji: string) => void }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [group, setGroup] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, open, () => setOpen(false));

  return (
    // Taps here don't take focus from the input: the keyboard stays up, and so does the caret where the emoji goes
    <div className="emoji" ref={ref} onMouseDown={(e) => e.preventDefault()}>
      <button
        type="button"
        className={`emoji-toggle ${open ? "active" : ""}`}
        aria-label={t.emoji}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <Smile size={20} />
      </button>
      {open && (
        <div className="emoji-picker" role="dialog" aria-label={t.emoji}>
          <div className="emoji-tabs" role="tablist">
            {EMOJI_GROUPS.map((g, i) => (
              <button key={g.icon} type="button" role="tab" aria-selected={i === group} onClick={() => setGroup(i)}>
                {g.icon}
              </button>
            ))}
          </div>
          <div className="emoji-grid">
            {EMOJI_GROUPS[group].emoji.map((e) => (
              <button key={e} type="button" onClick={() => onPick(e)}>
                {e}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
