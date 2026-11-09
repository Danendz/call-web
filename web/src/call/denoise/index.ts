/// <reference types="vite/client" />
import wasmUrl from "./denoise.wasm?url";
import workletUrl from "./worklet.ts?worker&url";

/** A mic track run through DeepFilterNet3, to send and transcribe in its place. */
export interface Denoised {
  readonly track: MediaStreamTrack;
  /** A context made before the user touched the page starts suspended; any call after a click starts it. */
  resume(): void;
  /** Ends the processed track; the mic it came from keeps running. */
  stop(): void;
}

// The model is about 10 MB: fetched and compiled once, then cached by the browser
let compiled: Promise<WebAssembly.Module> | null = null;

function loadModule(): Promise<WebAssembly.Module> {
  compiled ??= fetch(wasmUrl)
    .then((res) => {
      if (!res.ok) throw new Error(`denoise.wasm: ${res.status}`);
      return res.arrayBuffer();
    })
    .then((bytes) => WebAssembly.compile(bytes))
    .catch((err) => {
      compiled = null;
      throw err;
    });
  return compiled;
}

/** Starts the download early, so the model is ready by the time a call needs it. */
export function preloadDenoise(): void {
  loadModule().catch((err) => console.warn("noise suppression didn't load", err));
}

/** Null when this browser can't run it, or the model didn't load: the caller sends the mic as it is. */
export async function denoise(mic: MediaStreamTrack): Promise<Denoised | null> {
  if (typeof AudioWorkletNode === "undefined") return null;
  let ctx: AudioContext | null = null;
  try {
    const module = await loadModule();
    // The model's rate; the browser resamples the mic to it
    ctx = new AudioContext({ sampleRate: 48_000 });
    await ctx.audioWorklet.addModule(workletUrl);
    const node = new AudioWorkletNode(ctx, "denoise", {
      channelCount: 1,
      channelCountMode: "explicit",
      processorOptions: { module },
    });
    const out = ctx.createMediaStreamDestination();
    ctx.createMediaStreamSource(new MediaStream([mic])).connect(node).connect(out);
    const track = out.stream.getAudioTracks()[0];
    const context = ctx;
    void context.resume().catch(() => {});
    return {
      track,
      resume: () => void context.resume().catch(() => {}),
      stop: () => {
        track.stop();
        void context.close();
      },
    };
  } catch (err) {
    // Firefox, for one, won't take a mic at another rate into a 48 kHz context
    console.warn("noise suppression unavailable", err);
    void ctx?.close();
    return null;
  }
}
