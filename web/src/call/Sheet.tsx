import type { ReactNode } from "react";
import { Drawer } from "vaul";

/**
 * A panel that slides up from the bottom over a dimmed screen, for narrow screens. vaul does the behavior (drag
 * or flick down to close, tap on the dim, Esc, focus kept inside and returned to the opener); call.css the look.
 */
export function Sheet({ label, open, onClose, children }: { label: string; open: boolean; onClose: () => void; children: ReactNode }) {
  return (
    <Drawer.Root open={open} onOpenChange={(next) => !next && onClose()}>
      <Drawer.Portal>
        <Drawer.Overlay className="sheet-dim" />
        <Drawer.Content className="sheet" aria-describedby={undefined}>
          <div className="sheet-grip" aria-hidden>
            <span />
          </div>
          <Drawer.Title className="sr-only">{label}</Drawer.Title>
          {children}
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer.Root>
  );
}
