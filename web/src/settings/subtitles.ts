import { useSyncExternalStore } from "react";

// "Live subtitles" is saved so it stays across calls; on unless turned off. Off, the chat shows spoken lines only once
// they're finished, and nothing is translated on this device.

const KEY = "subtitles";
const listeners = new Set<() => void>();

function load(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

let subtitles = load();

export function setSubtitles(on: boolean): void {
  subtitles = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // storage blocked: the choice lasts until the page closes
  }
  listeners.forEach((l) => l());
}

export function getSubtitles(): boolean {
  return subtitles;
}

/** Calls `listener` on every change; returns what stops it. */
export function onSubtitlesChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSubtitles(): boolean {
  return useSyncExternalStore(onSubtitlesChange, getSubtitles);
}
