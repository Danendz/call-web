/// <reference types="vite/client" />
import { useEffect, useState, useSyncExternalStore } from "react";
import type { Lang } from "../../../shared/protocol";
import wasmUrl from "../../../vendor/bergamot/bergamot.wasm?url";
import glueUrl from "../../../vendor/bergamot/glue.js?url";
import { pairFor, read, useModelStatus, type Pair } from "./models";
import type { WorkerRequest, WorkerResponse } from "./worker";
import TranslateWorker from "./worker.ts?worker";

// Lines waiting for the Worker. Live text changes faster than it's ever translated only if the device is very slow;
// then the oldest waiting lines are dropped, since newer text has replaced them.
const MAX_WAITING = 6;
// Translations kept, so a line shown again isn't translated again
const MAX_KEPT = 200;

/** One direction's Worker, and what it has translated. */
class Translator {
  private readonly worker = new TranslateWorker();
  private readonly done = new Map<string, string>();
  private readonly waiting: string[] = [];
  private busy = false;
  private ready = false;
  private readonly listeners = new Set<() => void>();
  version = 0;

  constructor(pair: Pair) {
    this.worker.onmessage = (e: MessageEvent<WorkerResponse>) => this.received(e.data);
    void this.load(pair);
  }

  private async load(pair: Pair): Promise<void> {
    try {
      const [files, wasm] = await Promise.all([read(pair), fetch(wasmUrl).then((r) => r.arrayBuffer())]);
      if (!files) return;
      const [from, to] = pair.split("-");
      const request: WorkerRequest = { type: "load", glue: new URL(glueUrl, location.href).href, wasm, from, to, files };
      this.worker.postMessage(request, [wasm, ...Object.values(files)]);
    } catch (err) {
      console.error("live translation didn't load:", err);
    }
  }

  private received(response: WorkerResponse): void {
    if (response.type === "ready") {
      this.ready = true;
      this.changed();
    } else if (response.type === "translated") {
      this.busy = false;
      const oldest = this.done.keys().next();
      if (this.done.size >= MAX_KEPT && !oldest.done) this.done.delete(oldest.value);
      this.done.set(response.text, response.translation);
      this.changed();
    } else {
      this.busy = false;
      console.error("live translation:", response.error);
    }
    this.next();
  }

  private next(): void {
    if (!this.ready || this.busy) return;
    const text = this.waiting.shift();
    if (text === undefined) return;
    this.busy = true;
    this.worker.postMessage({ type: "translate", text } satisfies WorkerRequest);
  }

  want(text: string): void {
    if (this.done.has(text) || this.waiting.includes(text)) return;
    this.waiting.push(text);
    if (this.waiting.length > MAX_WAITING) this.waiting.shift();
    this.next();
  }

  get(text: string): string | undefined {
    return this.done.get(text);
  }

  isReady(): boolean {
    return this.ready;
  }

  private changed(): void {
    this.version++;
    this.listeners.forEach((l) => l());
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  stop(): void {
    this.worker.terminate();
  }
}

const noSubscribe = () => () => {};

/**
 * Translates `texts` from the other language into `lang` on this device, once its model is downloaded. Returns the
 * translations so far, or null while there's nothing to translate with (no model, or still loading).
 */
export function useQuickTranslations(texts: string[], lang: Lang, on: boolean): Map<string, string> | null {
  const pair = pairFor(lang);
  const ready = useModelStatus(pair).kind === "ready";
  const [translator, setTranslator] = useState<Translator | null>(null);
  useEffect(() => {
    if (!on || !ready) return;
    // The engine and model take a few hundred MB, so they're only kept while live subtitles are on
    const t = new Translator(pair);
    setTranslator(t);
    return () => {
      t.stop();
      setTranslator(null);
    };
  }, [pair, on, ready]);

  useSyncExternalStore(translator?.subscribe ?? noSubscribe, () => translator?.version ?? 0);
  const key = texts.join("\n");
  useEffect(() => {
    if (translator) for (const text of texts) translator.want(text);
    // `key` stands for `texts`, which is a new array each render
  }, [translator, key]);

  if (!translator?.isReady()) return null;
  const out = new Map<string, string>();
  for (const text of texts) {
    const t = translator.get(text);
    if (t !== undefined) out.set(text, t);
  }
  return out;
}
