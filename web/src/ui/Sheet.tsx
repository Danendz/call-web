import type { ReactNode } from "react";
import { Drawer } from "vaul";
import "./sheet.css";

/**
 * A panel that slides in over a dimmed screen: up from the bottom on narrow screens, or in from the right as a
 * side panel. vaul does the behavior (drag or flick it away, tap on the dim, Esc, focus kept inside and returned
 * to the opener); sheet.css the look.
 */
export function Sheet({
  label,
  open,
  onClose,
  side = "bottom",
  showTitle = false,
  children,
}: {
  label: string;
  open: boolean;
  onClose: () => void;
  side?: "bottom" | "right";
  /** Show `label` as a heading; otherwise it's only read out by screen readers. */
  showTitle?: boolean;
  children: ReactNode;
}) {
  return (
    <Drawer.Root open={open} onOpenChange={(next) => !next && onClose()} direction={side}>
      <Drawer.Portal>
        <Drawer.Overlay className="sheet-dim" />
        <Drawer.Content className={`sheet ${side}`} aria-describedby={undefined}>
          {side === "bottom" && (
            <div className="sheet-grip" aria-hidden>
              <span />
            </div>
          )}
          <Drawer.Title className={showTitle ? "sheet-title" : "sr-only"}>{label}</Drawer.Title>
          {children}
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer.Root>
  );
}
