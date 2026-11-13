import { useSyncExternalStore } from "react";

/**
 * Phones, upright or on their side: the pill becomes a rail down the right edge, its menus open as bottom sheets and
 * the chat spans the width beside it. Same as call.css's phone block.
 */
export const SHEET_QUERY = "(max-width: 899px), (max-height: 500px)";

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const mql = window.matchMedia(query);
      mql.addEventListener("change", onChange);
      return () => mql.removeEventListener("change", onChange);
    },
    () => window.matchMedia(query).matches,
  );
}
