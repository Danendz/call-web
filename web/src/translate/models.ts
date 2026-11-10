import { useSyncExternalStore } from "react";
import type { Lang } from "../../../shared/protocol";

// The translation model for each direction: Firefox's Bergamot models, served from our own server (/updates/, the
// releases bucket) and kept in Cache Storage. A file is only stored once its SHA-256 matches. The names are versioned,
// so a stored file never goes stale. Kept the same as native/crates/engine/src/translate/model.rs.

export type Pair = "en-zh" | "zh-en";
export type FileType = "model" | "lex" | "vocab" | "srcvocab" | "trgvocab";

interface ModelFile {
  type: FileType;
  name: string;
  size: number;
  sha256: string;
}

const FILES: Record<Pair, ModelFile[]> = {
  "en-zh": [
    {
      type: "model",
      name: "bergamot-enzh-2.2-model.bin",
      size: 43_849_787,
      sha256: "4e5accc141373565ddc8fa1565bceaa8d0c3482a82cab8131c719ebcc6c2157c",
    },
    {
      type: "lex",
      name: "bergamot-enzh-2.2-lex.bin",
      size: 6_506_248,
      sha256: "4a5e5827788060f1d718a8132b69440929387514a045796e9b77f935db68c055",
    },
    {
      type: "srcvocab",
      name: "bergamot-enzh-2.2-srcvocab.spm",
      size: 806_952,
      sha256: "bd9b65504acc6d9726dd281f7defc2adb7c2c22d0688fe2f84697de25197c8c5",
    },
    {
      type: "trgvocab",
      name: "bergamot-enzh-2.2-trgvocab.spm",
      size: 772_004,
      sha256: "aded6993c36e440284d11cec3f6b8aef9c0e43188a772d80be342a713adf223d",
    },
  ],
  "zh-en": [
    {
      type: "model",
      name: "bergamot-zhen-2.1-model.bin",
      size: 43_977_787,
      sha256: "5cd149601802fc8a18124a1c1306144dbbedc058630c4ddb2d53aa76fa9c7c06",
    },
    {
      type: "lex",
      name: "bergamot-zhen-2.1-lex.bin",
      size: 9_219_192,
      sha256: "8524dd4c93ffd7f0ae7be32f77a7c14fea7cdbbebe34ea6ad0ec368d5ddd8b13",
    },
    {
      type: "vocab",
      name: "bergamot-zhen-2.1-vocab.spm",
      size: 1_359_697,
      sha256: "dff594318ab7d8b7b60b844ab98ebe6b932ae8045fab15235404c787715965b3",
    },
  ],
};

const CACHE = "bergamot";

/** The direction this device needs: from the other language into the one you speak. */
export function pairFor(lang: Lang): Pair {
  return lang === "zh" ? "en-zh" : "zh-en";
}

/** The whole download for a direction, in MB, for the UI */
export function sizeMb(pair: Pair): number {
  return Math.round(FILES[pair].reduce((n, f) => n + f.size, 0) / 1_000_000);
}

export type ModelStatus =
  | { kind: "checking" }
  | { kind: "missing" }
  | { kind: "downloading"; percent: number }
  | { kind: "ready" }
  // The last download stopped on an error
  | { kind: "failed" };

const statuses: Record<Pair, ModelStatus> = { "en-zh": { kind: "checking" }, "zh-en": { kind: "checking" } };
const listeners = new Set<() => void>();

function set(pair: Pair, status: ModelStatus): void {
  statuses[pair] = status;
  listeners.forEach((l) => l());
}

function url(file: ModelFile): string {
  return `/updates/${file.name}`;
}

async function open(): Promise<Cache | null> {
  try {
    return await caches.open(CACHE);
  } catch {
    // No Cache Storage (an insecure page, or storage blocked): nothing can be kept
    return null;
  }
}

async function check(pair: Pair): Promise<void> {
  const cache = await open();
  const found = cache ? await Promise.all(FILES[pair].map((f) => cache.match(url(f)))) : [];
  if (statuses[pair].kind === "checking") set(pair, { kind: found.length && found.every(Boolean) ? "ready" : "missing" });
}

for (const pair of Object.keys(FILES) as Pair[]) void check(pair);

/** Downloads what's missing for a direction, unless it's already being downloaded. */
export async function download(pair: Pair): Promise<void> {
  const kind = statuses[pair].kind;
  if (kind === "downloading" || kind === "ready") return;
  const cache = await open();
  if (!cache) {
    set(pair, { kind: "failed" });
    return;
  }
  const total = FILES[pair].reduce((n, f) => n + f.size, 0);
  let done = 0;
  let shown = 0;
  set(pair, { kind: "downloading", percent: 0 });
  try {
    for (const file of FILES[pair]) {
      if (await cache.match(url(file))) {
        done += file.size;
        continue;
      }
      const bytes = await fetchFile(file, (n) => {
        const percent = Math.min(99, Math.floor(((done + n) * 100) / total));
        if (percent !== shown) set(pair, { kind: "downloading", percent: (shown = percent) });
      });
      const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
      const hex = Array.from(hash, (b) => b.toString(16).padStart(2, "0")).join("");
      if (hex !== file.sha256) throw new Error(`${file.name} didn't match its checksum`);
      await cache.put(url(file), new Response(bytes));
      done += file.size;
    }
    set(pair, { kind: "ready" });
  } catch (err) {
    console.error("translation model:", err instanceof Error ? err.message : err);
    set(pair, { kind: "failed" });
  }
}

async function fetchFile(file: ModelFile, progress: (n: number) => void): Promise<ArrayBuffer> {
  const res = await fetch(url(file));
  if (!res.ok || !res.body) throw new Error(`${file.name} answered ${res.status}`);
  const bytes = new Uint8Array(file.size);
  let at = 0;
  const reader = res.body.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (at + value.byteLength > file.size) throw new Error(`${file.name} is longer than expected`);
    bytes.set(value, at);
    at += value.byteLength;
    progress(at);
  }
  if (at !== file.size) throw new Error(`${file.name} ended at ${at} of ${file.size} bytes`);
  return bytes.buffer;
}

/** Removes a direction's files. */
export async function remove(pair: Pair): Promise<void> {
  const cache = await open();
  if (cache) await Promise.all(FILES[pair].map((f) => cache.delete(url(f))));
  set(pair, { kind: "missing" });
}

/** A ready direction's files, by type, for the Worker. Null if one has gone (storage cleared). */
export async function read(pair: Pair): Promise<Partial<Record<FileType, ArrayBuffer>> | null> {
  const cache = await open();
  if (!cache) return null;
  const files: Partial<Record<FileType, ArrayBuffer>> = {};
  for (const file of FILES[pair]) {
    const res = await cache.match(url(file));
    if (!res) {
      set(pair, { kind: "missing" });
      return null;
    }
    files[file.type] = await res.arrayBuffer();
  }
  return files;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useModelStatus(pair: Pair): ModelStatus {
  return useSyncExternalStore(subscribe, () => statuses[pair]);
}
