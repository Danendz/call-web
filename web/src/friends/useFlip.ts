import { useLayoutEffect, useRef, type RefObject } from "react";

/**
 * Glides the container's `[data-flip]` children from where they were to where the last render put them, matched by
 * the attribute's value. A child that's new has nowhere to glide from and comes in with its own CSS animation.
 */
export function useFlip(container: RefObject<HTMLElement>) {
  const was = useRef(new Map<string, number>());
  useLayoutEffect(() => {
    const box = container.current;
    if (!box) return;
    const style = getComputedStyle(box);
    const duration = matchMedia("(prefers-reduced-motion: reduce)").matches
      ? 0
      : parseFloat(style.getPropertyValue("--med"));
    const easing = style.getPropertyValue("--ease");
    // Against the container, so scrolling or a card coming in above doesn't count as moving
    const top = box.getBoundingClientRect().top;
    const now = new Map<string, number>();
    for (const child of box.querySelectorAll<HTMLElement>(":scope > [data-flip]")) {
      const key = child.dataset.flip!;
      const y = child.getBoundingClientRect().top - top;
      now.set(key, y);
      const from = was.current.get(key);
      if (duration && from !== undefined && from !== y) {
        child.animate([{ transform: `translateY(${from - y}px)` }, { transform: "none" }], { duration, easing });
      }
    }
    was.current = now;
  });
}
