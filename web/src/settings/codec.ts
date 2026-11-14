import { useSyncExternalStore } from "react";
import type { VideoCodec } from "../call/codec";

// "Video codec" is saved so a call that only works on VP8 stays on it. The sheet shows it and the call controller
// follows it, live.

const KEY = "videoCodec";
const listeners = new Set<() => void>();

function load(): VideoCodec {
  try {
    const saved = localStorage.getItem(KEY);
    return saved === "h264" || saved === "vp8" ? saved : "auto";
  } catch {
    return "auto";
  }
}

let videoCodec = load();

export function getVideoCodec(): VideoCodec {
  return videoCodec;
}

export function setVideoCodec(pick: VideoCodec): void {
  videoCodec = pick;
  try {
    localStorage.setItem(KEY, pick);
  } catch {
    // storage blocked: the choice lasts until the page closes
  }
  listeners.forEach((l) => l());
}

export function onVideoCodecChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useVideoCodec(): VideoCodec {
  return useSyncExternalStore(onVideoCodecChange, () => videoCodec);
}
