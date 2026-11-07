import { useRef, useState, type CSSProperties, type PointerEvent } from "react";

export type Corner = "tl" | "tr" | "bl" | "br";

const CORNERS: readonly string[] = ["tl", "tr", "bl", "br"];
const CORNERS_KEY = "tile-corners";
// Less movement than this between press and release is a tap, not a drag
const DRAG_THRESHOLD_PX = 6;

/** Which corner each small tile sits in, remembered across calls. */
export function useCorners<K extends string>(defaults: Record<K, Corner>) {
  const [corners, setCorners] = useState<Record<K, Corner>>(() => ({ ...defaults, ...loadCorners(Object.keys(defaults)) }));
  const setCorner = (key: K, corner: Corner) =>
    setCorners((prev) => {
      const next = { ...prev, [key]: corner };
      try {
        localStorage.setItem(CORNERS_KEY, JSON.stringify(next));
      } catch {
        // storage blocked: the corner holds until the page closes
      }
      return next;
    });
  return { corners, setCorner };
}

function loadCorners(keys: string[]): Record<string, Corner> {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(CORNERS_KEY) ?? "{}");
    if (typeof saved !== "object" || saved === null) return {};
    return Object.fromEntries(
      Object.entries(saved).filter(([k, v]) => keys.includes(k) && typeof v === "string" && CORNERS.includes(v)),
    );
  } catch {
    return {};
  }
}

/**
 * Lets a tile follow the pointer (mouse or finger) around its positioned parent. On release it reports the corner
 * nearest its centre and drops the inline position, so CSS glides it there. A press that barely moves is a tap:
 * wasDragged() tells the click handler which one just happened.
 */
export function useDragCorner(onDrop: (corner: Corner) => void) {
  const start = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const dragged = useRef(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  const onPointerDown = (e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    start.current = { x: e.clientX, y: e.clientY, left: el.offsetLeft, top: el.offsetTop };
    dragged.current = false;
    el.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<HTMLElement>) => {
    const s = start.current;
    if (!s) return;
    const dx = e.clientX - s.x;
    const dy = e.clientY - s.y;
    if (!dragged.current && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
    dragged.current = true;
    const el = e.currentTarget;
    const parent = el.offsetParent as HTMLElement;
    setPos({
      left: clamp(s.left + dx, 0, parent.clientWidth - el.offsetWidth),
      top: clamp(s.top + dy, 0, parent.clientHeight - el.offsetHeight),
    });
  };

  const onPointerUp = (e: PointerEvent<HTMLElement>) => {
    if (!start.current) return;
    start.current = null;
    if (!dragged.current) return;
    const el = e.currentTarget;
    const parent = el.offsetParent as HTMLElement;
    const top = el.offsetTop + el.offsetHeight / 2 < parent.clientHeight / 2;
    const left = el.offsetLeft + el.offsetWidth / 2 < parent.clientWidth / 2;
    onDrop(`${top ? "t" : "b"}${left ? "l" : "r"}`);
    setPos(null);
  };

  return {
    dragging: pos !== null,
    style: (pos ?? undefined) as CSSProperties | undefined,
    handlers: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: onPointerUp },
    wasDragged: () => dragged.current,
  };
}

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(n, max));
}
