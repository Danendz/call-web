import { useSyncExternalStore } from "react";

// "Show connection stats" is saved so it stays on across calls; the sheet and the call screen read the same value.

const KEY = "stats";
const listeners = new Set<() => void>();

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "on";
  } catch {
    return false;
  }
}

let showStats = load();

export function setShowStats(on: boolean): void {
  showStats = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // storage blocked: the choice lasts until the page closes
  }
  listeners.forEach((l) => l());
}

export function useShowStats(): boolean {
  return useSyncExternalStore(
    (onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
    () => showStats,
  );
}
