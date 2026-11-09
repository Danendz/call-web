import type { ReactNode, RefObject } from "react";
import { Sheet } from "../ui/Sheet";
import { SHEET_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import { useDismiss } from "./useDismiss";

/**
 * A pill menu's panel: a popover above its button, or a bottom sheet on narrow screens. The list scrolls and
 * the footer stays in view under it.
 */
export function MenuPanel({
  label,
  open,
  onClose,
  within,
  className = "",
  footer,
  children,
}: {
  label: string;
  open: boolean;
  onClose: () => void;
  /** Holds the button and the popover: taps inside it leave the popover open. */
  within: RefObject<HTMLElement | null>;
  className?: string;
  footer?: ReactNode;
  children: ReactNode;
}) {
  const sheet = useMediaQuery(SHEET_QUERY);
  useDismiss(within, open && !sheet, onClose);

  const body = (
    <>
      <div className="menu-list">{children}</div>
      {footer && <div className="menu-foot">{footer}</div>}
    </>
  );
  if (sheet) {
    return (
      <Sheet label={label} open={open} onClose={onClose}>
        <div className="menu-body" role="menu" aria-label={label}>
          {body}
        </div>
      </Sheet>
    );
  }
  return (
    <div className={`menu ${className} ${open ? "open" : ""}`} role="menu" aria-label={label} aria-hidden={!open}>
      {body}
    </div>
  );
}
