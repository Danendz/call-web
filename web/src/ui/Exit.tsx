import { cloneElement, useReducer, useRef, type AnimationEvent, type ReactElement } from "react";

type Animatable = ReactElement<{
  className?: string;
  onAnimationEnd?: (e: AnimationEvent) => void;
  "aria-hidden"?: boolean;
}>;

/**
 * Shows its child, and once there's none keeps the last one on screen with `leaving` added to its class until that
 * animation ends. The child's CSS gives `.leaving` an exit animation, or it would stay.
 */
export function Exit({ children }: { children: Animatable | null | false }) {
  const last = useRef<Animatable | null>(null);
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  if (children) {
    last.current = children;
    return children;
  }
  const leaving = last.current;
  if (!leaving) return null;
  return cloneElement(leaving, {
    className: `${leaving.props.className ?? ""} leaving`,
    "aria-hidden": true,
    onAnimationEnd: (e: AnimationEvent) => {
      // A child's own animation (a pulsing icon) bubbles up here too
      if (e.target !== e.currentTarget) return;
      last.current = null;
      rerender();
    },
  });
}
