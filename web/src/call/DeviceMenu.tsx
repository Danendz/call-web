import { ChevronUp } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { MenuButton, type MenuItem } from "./MoreMenu";
import { useDismiss } from "./useDismiss";

export interface MenuSection {
  title: string;
  /** A toggle beside the title, which leaves the menu open to show its new state. */
  toggle?: { label: string; icon: ReactNode; on: boolean; onToggle: () => void };
  items: MenuItem[];
}

/**
 * A pill button with a chevron beside it that opens a menu above: the button keeps its one-tap job (mute), the
 * chevron holds the choices behind it (which device). Closes on outside tap and Esc.
 */
export function DeviceMenu({
  label,
  sections,
  footer = [],
  open,
  onOpenChange,
  children,
}: {
  /** The chevron's name, for screen readers and its tooltip. */
  label: string;
  sections: MenuSection[];
  footer?: MenuItem[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The button the chevron belongs to. */
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const close = () => onOpenChange(false);
  useDismiss(ref, open, close);

  return (
    <div className="split" ref={ref}>
      {children}
      <button
        type="button"
        className="split-chevron"
        aria-label={label}
        aria-expanded={open}
        title={label}
        onClick={() => onOpenChange(!open)}
      >
        <ChevronUp size={16} />
      </button>
      <div className={`menu ${open ? "open" : ""}`} role="menu" aria-hidden={!open}>
        {sections.map((section) => (
          <div key={section.title} className="menu-section" role="group" aria-label={section.title}>
            <div className="menu-head">
              <span>{section.title}</span>
              {section.toggle && (
                <button
                  type="button"
                  className={`menu-chip ${section.toggle.on ? "on" : ""}`}
                  aria-pressed={section.toggle.on}
                  onClick={section.toggle.onToggle}
                >
                  {section.toggle.icon}
                  {section.toggle.label}
                </button>
              )}
            </div>
            {section.items.map((item) => (
              <MenuButton key={item.id ?? item.label} item={item} onDone={close} />
            ))}
          </div>
        ))}
        {footer.length > 0 && <div className="menu-sep" />}
        {footer.map((item) => (
          <MenuButton key={item.label} item={item} onDone={close} />
        ))}
      </div>
    </div>
  );
}
