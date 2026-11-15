import { useSyncExternalStore } from "react";

// "Sharpen video" is saved so it stays as picked across calls. Until it's picked, it's on where there's a mouse and
// off on phones and tablets, to save their battery.

const KEY = "sharpenVideo";
const listeners = new Set<() => void>();

function load(): boolean {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved) return saved === "on";
  } catch {
    // storage blocked: the default below
  }
  return !matchMedia("(pointer: coarse)").matches;
}

let sharpen = load();

export function setSharpenVideo(on: boolean): void {
  sharpen = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // storage blocked: the choice lasts until the page closes
  }
  listeners.forEach((l) => l());
}

export function useSharpenVideo(): boolean {
  return useSyncExternalStore(
    (onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
    () => sharpen,
  );
}
