import { useSyncExternalStore } from "react";

// "Live subtitles" is saved so it stays across calls; on unless turned off. Off, nothing is shown over the video.

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

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSubtitles(): boolean {
  return useSyncExternalStore(subscribe, () => subtitles);
}
