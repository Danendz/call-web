/**
 * Translates live subtitles on this device with Firefox's Bergamot engine (vendor/bergamot), one direction per Worker.
 * A classic Worker, so the engine's Emscripten loader can come in through importScripts, as Firefox loads it.
 */

import type { FileType } from "./models";

export type WorkerRequest =
  | {
      type: "load";
      glue: string;
      wasm: ArrayBuffer;
      from: string;
      to: string;
      files: Partial<Record<FileType, ArrayBuffer>>;
    }
  | { type: "translate"; text: string };

export type WorkerResponse =
  | { type: "ready" }
  | { type: "failed"; error: string }
  | { type: "translated"; text: string; translation: string };

// The Worker's globals, missing from TypeScript's DOM types
declare function importScripts(...urls: string[]): void;
declare function loadBergamot(module: object): Bergamot;
const scope = self as unknown as {
  onmessage: ((e: MessageEvent<WorkerRequest>) => void) | null;
  postMessage(message: WorkerResponse): void;
};

// The parts of the engine's embind API used here
interface Handle {
  delete(): void;
}
interface Bergamot {
  AlignedMemory: new (size: number, alignment: number) => Handle & { getByteArrayView(): Uint8Array };
  AlignedMemoryList: new () => Handle & { push_back(m: Handle): void };
  TranslationModel: new (
    from: string,
    to: string,
    config: string,
    model: Handle,
    lex: Handle | null,
    vocabs: Handle,
    quality: null,
  ) => Handle;
  BlockingService: new (config: { cacheSize: number }) => Handle & {
    translate(model: Handle, messages: Handle, options: Handle): Handle & {
      get(i: number): Handle & { getTranslatedText(): string };
    };
  };
  VectorString: new () => Handle & { push_back(s: string): void };
  VectorResponseOptions: new () => Handle & {
    push_back(o: { qualityScores: boolean; alignment: boolean; html: boolean }): void;
  };
}

// As Firefox lays each file out in the engine's memory
const ALIGNMENT: Record<FileType, number> = { model: 256, lex: 64, vocab: 64, srcvocab: 64, trgvocab: 64 };

// Firefox's settings for its int8 "alphas" models
const CONFIG: Record<string, string> = {
  "beam-size": "1",
  normalize: "1.0",
  "word-penalty": "0",
  "max-length-break": "128",
  "mini-batch-words": "1024",
  workspace: "128",
  "max-length-factor": "2.0",
  "skip-cost": "true",
  "cpu-threads": "0",
  quiet: "true",
  "quiet-translation": "true",
  "gemm-precision": "int8shiftAlphaAll",
  alignment: "soft",
};

let engine: {
  bergamot: Bergamot;
  service: InstanceType<Bergamot["BlockingService"]>;
  model: Handle;
} | null = null;

function start(glue: string, wasm: ArrayBuffer): Promise<Bergamot> {
  importScripts(glue);
  return new Promise((resolve, reject) => {
    const bergamot = loadBergamot({
      // 40 MB to start with; it grows as the model needs
      INITIAL_MEMORY: 41_943_040,
      wasmBinary: wasm,
      print: () => {},
      printErr: (m: string) => console.warn("bergamot:", m),
      onAbort: (reason: unknown) => reject(new Error(`bergamot aborted: ${String(reason)}`)),
      // Resolves after the loader returns, when `bergamot` is set
      onRuntimeInitialized: () => void Promise.resolve().then(() => resolve(bergamot)),
    });
  });
}

function load(bergamot: Bergamot, request: Extract<WorkerRequest, { type: "load" }>): Handle {
  const memory: Partial<Record<FileType, Handle>> = {};
  for (const [type, bytes] of Object.entries(request.files) as [FileType, ArrayBuffer][]) {
    const m = new bergamot.AlignedMemory(bytes.byteLength, ALIGNMENT[type]);
    m.getByteArrayView().set(new Uint8Array(bytes));
    memory[type] = m;
  }
  const vocabList = memory.vocab ? [memory.vocab] : [memory.srcvocab, memory.trgvocab];
  if (!memory.model || vocabList.some((v) => !v)) throw new Error("model files missing");
  const vocabs = new bergamot.AlignedMemoryList();
  for (const v of vocabList) if (v) vocabs.push_back(v);
  const indent = "            ";
  const config = `\n${Object.entries(CONFIG)
    .map(([k, v]) => `${indent}${k}: ${v}\n`)
    .join("")}${indent}`;
  // The model takes the aligned memory over, so it's not deleted here
  return new bergamot.TranslationModel(
    request.from,
    request.to,
    config,
    memory.model,
    memory.lex ?? null,
    vocabs,
    null,
  );
}

function translate(text: string): string {
  if (!engine) throw new Error("not loaded");
  const { bergamot, service, model } = engine;
  const messages = new bergamot.VectorString();
  const options = new bergamot.VectorResponseOptions();
  try {
    messages.push_back(text);
    options.push_back({ qualityScores: false, alignment: false, html: false });
    const responses = service.translate(model, messages, options);
    try {
      const response = responses.get(0);
      try {
        return response.getTranslatedText();
      } finally {
        response.delete();
      }
    } finally {
      responses.delete();
    }
  } finally {
    messages.delete();
    options.delete();
  }
}

scope.onmessage = (e) => {
  const request = e.data;
  if (request.type === "load") {
    start(request.glue, request.wasm)
      .then((bergamot) => {
        engine = { bergamot, service: new bergamot.BlockingService({ cacheSize: 0 }), model: load(bergamot, request) };
        scope.postMessage({ type: "ready" });
      })
      .catch((err: unknown) => scope.postMessage({ type: "failed", error: String(err) }));
  } else {
    try {
      scope.postMessage({ type: "translated", text: request.text, translation: translate(request.text) });
    } catch (err) {
      scope.postMessage({ type: "failed", error: String(err) });
    }
  }
};
