import { useSyncExternalStore } from "react";

// "Noise suppression" is saved so it stays on across calls; on unless turned off. The sheet shows it and the call
// controller follows it, live.

const KEY = "noise";
const listeners = new Set<() => void>();

function load(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

let noiseSuppression = load();

export function getNoiseSuppression(): boolean {
  return noiseSuppression;
}

export function setNoiseSuppression(on: boolean): void {
  noiseSuppression = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // storage blocked: the choice lasts until the page closes
  }
  listeners.forEach((l) => l());
}

export function onNoiseSuppressionChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useNoiseSuppression(): boolean {
  return useSyncExternalStore(onNoiseSuppressionChange, () => noiseSuppression);
}
