import { Check, Ellipsis } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";

export interface MenuItem {
  label: string;
  icon: ReactNode;
  /** A toggle that's on gets a check mark. */
  checked?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

/** The ⋯ button and its popover with the controls that don't fit in the pill. Closes on outside tap and Esc. */
export function MoreMenu({ items, open, onOpenChange }: { items: MenuItem[]; open: boolean; onOpenChange: (open: boolean) => void }) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onOpenChange(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onOpenChange]);

  return (
    <div className="more" ref={ref}>
      <IconButton label={t.more} variant={open ? "active" : null} expanded={open} onClick={() => onOpenChange(!open)}>
        <Ellipsis size={22} />
      </IconButton>
      <div className={`menu ${open ? "open" : ""}`} role="menu" aria-hidden={!open}>
        {items.map((item) => (
          <button
            key={item.label}
            type="button"
            role="menuitemcheckbox"
            aria-checked={item.checked ?? false}
            className="menu-item"
            disabled={item.disabled}
            onClick={() => {
              item.onSelect();
              onOpenChange(false);
            }}
          >
            {item.icon}
            <span>{item.label}</span>
            {item.checked && <Check size={16} className="check" />}
          </button>
        ))}
      </div>
    </div>
  );
}
