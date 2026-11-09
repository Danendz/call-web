import { useSyncExternalStore } from "react";

/** Phone held upright: chat opens under the video instead of beside it. Same query as in call.css. */
export const STACKED_QUERY = "(max-width: 899px) and (orientation: portrait)";

/** Narrow screens, where the pill's menus open as bottom sheets. Same width as call.css's narrow block. */
export const SHEET_QUERY = "(max-width: 899px)";

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
