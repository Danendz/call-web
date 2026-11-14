import { Check, Ellipsis } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import { Spinner } from "../ui/Spinner";
import { MenuPanel } from "./MenuPanel";

export interface MenuItem {
  /** For lists of devices, whose names can repeat. */
  id?: string;
  label: string;
  icon: ReactNode;
  /** A toggle that's on gets a check mark. */
  checked?: boolean;
  /** On its way: a spinner where the check mark goes. */
  busy?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

/** The ⋯ button and its menu with the controls that don't fit in the pill. Picking an action closes it. */
export function MoreMenu({
  items,
  badge,
  open,
  onOpenChange,
}: {
  items: MenuItem[];
  /** Unread messages, when the chat toggle is one of the items */
  badge?: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const close = () => onOpenChange(false);

  return (
    <div className="more" ref={ref}>
      <IconButton label={t.more} variant={open ? "active" : null} expanded={open} badge={badge} onClick={() => onOpenChange(!open)}>
        <Ellipsis size={22} />
      </IconButton>
      <MenuPanel label={t.more} open={open} onClose={close} within={ref}>
        {items.map((item) => (
          <MenuButton key={item.label} item={item} onDone={close} />
        ))}
      </MenuPanel>
    </div>
  );
}

/** One row of a menu; `onDone` runs once it's picked, to close the menu. */
export function MenuButton({ item, onDone }: { item: MenuItem; onDone?: () => void }) {
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={item.checked ?? false}
      className="menu-item"
      disabled={item.disabled}
      onClick={() => {
        item.onSelect();
        onDone?.();
      }}
    >
      {item.icon}
      <span>{item.label}</span>
      {item.busy ? <Spinner size={16} /> : item.checked && <Check size={16} className="check" />}
    </button>
  );
}
