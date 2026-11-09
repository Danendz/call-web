import { ChevronUp } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { MenuPanel } from "./MenuPanel";
import { MenuButton, type MenuItem } from "./MoreMenu";

export interface MenuSection {
  title: string;
  /** A toggle beside the title, which leaves the menu open to show its new state. */
  toggle?: { label: string; icon: ReactNode; on: boolean; onToggle: () => void };
  items: MenuItem[];
}

/**
 * A pill button with a chevron beside it that opens a menu above: the button keeps its one-tap job (mute), the
 * chevron holds the choices behind it (which device). Picking leaves it open, so the spinner and then the check
 * mark show the switch, and a mic and a speaker can be picked in one go.
 */
export function DeviceMenu({
  label,
  sections,
  footer = [],
  open,
  onOpenChange,
  children,
}: {
  /** The chevron's name, for screen readers and its tooltip; also the sheet's. */
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
      <MenuPanel
        label={label}
        open={open}
        onClose={close}
        within={ref}
        className="device-menu"
        footer={footer.length > 0 && footer.map((item) => <MenuButton key={item.label} item={item} />)}
      >
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
              <MenuButton key={item.id ?? item.label} item={item} />
            ))}
          </div>
        ))}
      </MenuPanel>
    </div>
  );
}
