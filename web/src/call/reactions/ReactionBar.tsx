import { SmilePlus } from "lucide-react";
import { useRef } from "react";
import { REACTIONS, type Reaction } from "../../../../shared/protocol";
import { useT } from "../../ui/i18n";
import { IconButton } from "../../ui/IconButton";
import { useDismiss } from "../useDismiss";

/** The pill's reaction button and the row of reactions above it, which stays open for tapping again and again. */
export function ReactionBar({
  open,
  onOpenChange,
  disabled,
  onReact,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  disabled: boolean;
  onReact: (emoji: Reaction) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, open, () => onOpenChange(false));

  return (
    <div className="react" ref={ref}>
      <IconButton label={t.react} variant={open ? "active" : null} expanded={open} disabled={disabled} onClick={() => onOpenChange(!open)}>
        <SmilePlus size={20} />
      </IconButton>
      <div className={`reaction-bar ${open ? "open" : ""}`} role="toolbar" aria-label={t.react} aria-hidden={!open}>
        {REACTIONS.map((emoji) => (
          <button key={emoji} type="button" tabIndex={open ? 0 : -1} onClick={() => onReact(emoji)}>
            {emoji}
          </button>
        ))}
      </div>
    </div>
  );
}
