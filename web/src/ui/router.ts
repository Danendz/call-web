import { useSyncExternalStore } from "react";
import { ROOM_ID_RE } from "../../../shared/protocol";

// The page's few addresses: / (friends, or a new room when signed out), /signin and /r/<room id>.

export type Route =
  | { page: "home" }
  | { page: "signin" }
  // autoJoin: skip the pre-join screen, for a call just answered or a room rejoined from the list
  | { page: "room"; roomId: string; autoJoin: boolean };

function parse(autoJoin = false): Route {
  const match = location.pathname.match(/^\/r\/([^/]+)\/?$/);
  if (match && ROOM_ID_RE.test(match[1])) return { page: "room", roomId: match[1], autoJoin };
  if (location.pathname === "/signin") return { page: "signin" };
  return { page: "home" };
}

const ROOM_ALPHABET = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/** The address of a new room link, random enough that nobody stumbles into it. */
export function newRoomPath(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  return `/r/${Array.from(bytes, (b) => ROOM_ALPHABET[b % ROOM_ALPHABET.length]).join("")}`;
}

let route = parse();
const listeners = new Set<() => void>();

function changed(next: Route): void {
  route = next;
  listeners.forEach((l) => l());
}

window.addEventListener("popstate", () => changed(parse()));

export function navigate(path: string, { replace = false, autoJoin = false } = {}): void {
  if (replace) history.replaceState(null, "", path);
  else history.pushState(null, "", path);
  changed(parse(autoJoin));
}

export function useRoute(): Route {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => route,
  );
}
