import { SmilePlus } from "lucide-react";
import { useRef, useState } from "react";
import { EMOJI_GROUPS } from "../../../../shared/emoji";
import { REACTIONS, type Reaction } from "../../../../shared/protocol";
import { useT } from "../../ui/i18n";
import { IconButton } from "../../ui/IconButton";
import { useDismiss } from "../useDismiss";

/**
 * The pill's emoji button and its panel: the quick reactions, then the grid of the rest. Each tap sends one, which
 * flies over the video and shows in the chat. Stays open for tapping again and again.
 */
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
  const [group, setGroup] = useState(0);
  useDismiss(ref, open, () => onOpenChange(false));

  return (
    <div className="react" ref={ref}>
      <IconButton label={t.react} variant={open ? "active" : null} expanded={open} disabled={disabled} onClick={() => onOpenChange(!open)}>
        <SmilePlus size={20} />
      </IconButton>
      <div className={`reaction-bar ${open ? "open" : ""}`} role="dialog" aria-label={t.react} aria-hidden={!open}>
        <div className="quick" role="toolbar" aria-label={t.react}>
          {REACTIONS.map((emoji) => (
            <button key={emoji} type="button" tabIndex={open ? 0 : -1} onClick={() => onReact(emoji)}>
              {emoji}
            </button>
          ))}
        </div>
        <div className="emoji-tabs" role="tablist" aria-label={t.emoji}>
          {EMOJI_GROUPS.map((g, i) => (
            <button key={g.icon} type="button" role="tab" tabIndex={open ? 0 : -1} aria-selected={i === group} onClick={() => setGroup(i)}>
              {g.icon}
            </button>
          ))}
        </div>
        <div className="emoji-grid">
          {EMOJI_GROUPS[group].emoji.map((e) => (
            <button key={e} type="button" tabIndex={open ? 0 : -1} onClick={() => onReact(e)}>
              {e}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
