import { Check, Ellipsis } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import { Spinner } from "../ui/Spinner";
import { useDismiss } from "./useDismiss";

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

/** The ⋯ button and its popover with the controls that don't fit in the pill. Closes on outside tap and Esc. */
export function MoreMenu({ items, open, onOpenChange }: { items: MenuItem[]; open: boolean; onOpenChange: (open: boolean) => void }) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, open, () => onOpenChange(false));

  return (
    <div className="more" ref={ref}>
      <IconButton label={t.more} variant={open ? "active" : null} expanded={open} onClick={() => onOpenChange(!open)}>
        <Ellipsis size={22} />
      </IconButton>
      <div className={`menu ${open ? "open" : ""}`} role="menu" aria-hidden={!open}>
        {items.map((item) => (
          <MenuButton key={item.label} item={item} onDone={() => onOpenChange(false)} />
        ))}
      </div>
    </div>
  );
}

/** One row of a popover menu; it closes the menu once picked. */
export function MenuButton({ item, onDone }: { item: MenuItem; onDone: () => void }) {
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={item.checked ?? false}
      className="menu-item"
      disabled={item.disabled}
      onClick={() => {
        item.onSelect();
        onDone();
      }}
    >
      {item.icon}
      <span>{item.label}</span>
      {item.busy ? <Spinner size={16} /> : item.checked && <Check size={16} className="check" />}
    </button>
  );
}
