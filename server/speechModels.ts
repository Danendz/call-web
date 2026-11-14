// The server's speech models, run by sherpa-onnx: SenseVoice-Small (the file the native app downloads for on-device
// speech) and Silero VAD. Fetched at startup from the releases bucket, or from the public /updates/ when there's no
// bucket configured (a local run), into MODELS_DIR, and checked against their SHA-256 before they're used. A file
// already there with the right hash isn't fetched again.

import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, rename, stat } from "node:fs/promises";
import { createRequire } from "node:module";
import { join } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream as WebReadableStream } from "node:stream/web";
import { TRANSCRIBE_SAMPLE_RATE } from "../shared/protocol.js";
import type { Decoder, Vad } from "./speechLines.js";
import { fetchFromBucket } from "./updates.js";

// A native addon in CommonJS
const sherpa = createRequire(import.meta.url)("sherpa-onnx-node") as typeof import("sherpa-onnx-node");

interface ModelFile {
  name: string;
  size: number;
  sha256: string;
}

const SENSE_VOICE: ModelFile = {
  name: "sensevoice-small-int8-2024-07-17.onnx",
  size: 239_233_841,
  sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
};
const SENSE_VOICE_TOKENS: ModelFile = {
  name: "sensevoice-small-2024-07-17-tokens.txt",
  size: 315_894,
  sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
};
const SILERO_VAD: ModelFile = {
  name: "silero-vad-v4.onnx",
  size: 643_854,
  sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
};

const DIR = process.env.MODELS_DIR || join(process.cwd(), ".models");
const PUBLIC_URL = "https://call.danendz.com/updates/";
// Threads for one decode; decodes run one at a time, off the main thread
const THREADS = 2;
// A pause in speech this long ends a line. Live subtitles cut sooner, so each line shows and finishes quickly;
// chat-only lines are about where the client's recorder ends the utterance anyway
const LIVE_PAUSE_S = 0.4;
const CHAT_PAUSE_S = 0.8;
// Talk without a pause this long is cut into a line anyway
const MAX_LINE_S = 20;
// What the VAD holds: a whole utterance, at most MAX_UTTERANCE_S, with room to spare
const VAD_BUFFER_S = 40;

export interface Speech {
  decoder: Decoder;
  vad(live: boolean): Vad;
}

async function sha256(path: string): Promise<string> {
  const hash = createHash("sha256");
  await pipeline(createReadStream(path), hash);
  return hash.digest("hex");
}

async function fetchModel(file: ModelFile): Promise<string> {
  const path = join(DIR, file.name);
  const have = await stat(path).catch(() => null);
  if (have?.size === file.size && (await sha256(path)) === file.sha256) return path;

  const res = await (fetchFromBucket(file.name) ?? fetch(PUBLIC_URL + file.name));
  if (!res.ok || !res.body) throw new Error(`${file.name}: ${res.status}`);
  const hash = createHash("sha256");
  const hashing = new Transform({
    transform(chunk: Buffer, _, done) {
      hash.update(chunk);
      done(null, chunk);
    },
  });
  const part = `${path}.part`;
  await pipeline(Readable.fromWeb(res.body as WebReadableStream), hashing, createWriteStream(part));
  if (hash.digest("hex") !== file.sha256) throw new Error(`${file.name}: SHA-256 doesn't match`);
  await rename(part, path);
  return path;
}

/** Decodes one at a time: two at once would only split the same cores, and a queue keeps lines in order. */
function serial(recognizer: import("sherpa-onnx-node").OfflineRecognizer): Decoder {
  let queue: Promise<unknown> = Promise.resolve();
  let waiting = 0;
  return {
    decode(samples) {
      waiting++;
      const result = queue.then(async () => {
        const stream = recognizer.createStream();
        stream.acceptWaveform({ samples, sampleRate: TRANSCRIBE_SAMPLE_RATE });
        return (await recognizer.decodeAsync(stream)).text;
      });
      queue = result.catch(() => {});
      return result
        .catch((err: unknown) => {
          console.warn("speech decode failed:", err instanceof Error ? err.message : err);
          return "";
        })
        .finally(() => waiting--);
    },
    waiting: () => waiting,
  };
}

async function load(): Promise<Speech> {
  const started = Date.now();
  await mkdir(DIR, { recursive: true });
  const [model, tokens, vadModel] = await Promise.all([SENSE_VOICE, SENSE_VOICE_TOKENS, SILERO_VAD].map(fetchModel));
  const recognizer = await sherpa.OfflineRecognizer.createAsync({
    featConfig: { sampleRate: TRANSCRIBE_SAMPLE_RATE, featureDim: 80 },
    // The language is left to the model: it tells Mandarin from English itself, also mixed in one line
    modelConfig: { senseVoice: { model, language: "auto", useInverseTextNormalization: 1 }, tokens, numThreads: THREADS, debug: 0 },
  });
  console.info(`speech models ready in ${Date.now() - started}ms`);
  return {
    decoder: serial(recognizer),
    vad(live) {
      const vad = new sherpa.Vad(
        {
          sileroVad: {
            model: vadModel,
            threshold: 0.5,
            minSilenceDuration: live ? LIVE_PAUSE_S : CHAT_PAUSE_S,
            minSpeechDuration: 0.25,
            windowSize: 512,
            maxSpeechDuration: MAX_LINE_S,
          },
          sampleRate: TRANSCRIBE_SAMPLE_RATE,
          numThreads: 1,
          debug: 0,
        },
        VAD_BUFFER_S,
      );
      return {
        accept: (samples) => vad.acceptWaveform(samples),
        ended() {
          const ended = !vad.isEmpty();
          while (!vad.isEmpty()) vad.pop();
          return ended;
        },
        flush: () => vad.flush(),
        speaking: () => vad.isDetected(),
      };
    },
  };
}

/** The models once they're loaded; null when they couldn't be, and speech is unavailable. */
export const speech: Promise<Speech | null> = load().catch((err: unknown) => {
  console.error("speech models failed to load:", err instanceof Error ? err.message : err);
  return null;
});
