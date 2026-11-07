// Speech to chat for browsers whose own recognition doesn't work (in China it goes to Google): the
// client streams each utterance over /ws/speech while it's being said, Groq Whisper transcribes it once
// it ends, and it's posted to the room's chat as if the speaker had typed it. Streaming means only the
// last moment of audio is still on its way when the speaker stops, which matters on links out of China

import type { WebSocket } from "ws";
import {
  LANGS,
  MAX_UTTERANCE_S,
  TRANSCRIBE_SAMPLE_RATE,
  type Lang,
  type SpeechClientMessage,
  type SpeechServerMessage,
} from "../shared/protocol.js";
import { createQuota, failIfNotOk } from "./llm.js";
import { rateLimit } from "./limits.js";
import { memberByToken, postChat } from "./rooms.js";

const GROQ_URL = "https://api.groq.com/openai/v1";
// Each has its own free quota (2h of audio an hour): when large-v3 runs out, turbo still has its own
const WHISPER_MODELS = ["whisper-large-v3", "whisper-large-v3-turbo"] as const;
const TIMEOUT_MS = 15_000;
// 16-bit mono at the agreed rate; audio past this is dropped
const MAX_UTTERANCE_BYTES = MAX_UTTERANCE_S * TRANSCRIBE_SAMPLE_RATE * 2;
// Less than this is a click, not worth a request
const MIN_UTTERANCE_BYTES = TRANSCRIBE_SAMPLE_RATE * 2 * 0.2;
// The client sends ~25 frames a second; a cap on the count keeps tiny frames from costing more than their bytes
const MAX_UTTERANCE_FRAMES = 2000;
// Whisper's own guess that a segment is noise; above this the segment is dropped
const NO_SPEECH_PROB = 0.6;
// One utterance every couple of seconds of nonstop talking; past that someone is spending the quota.
const uploadsPerMember = rateLimit(30, 60_000);
// What Whisper makes up from noise it was trained on (video subtitles credits)
const HALLUCINATION_RE = /amara\.org|字幕由|明镜与点点|點點栏目|请不吝点赞/i;

class OutOfQuota extends Error {
  constructor(readonly until: number) {
    super("Transcribe: every model is out of quota");
  }
}

const withQuota = createQuota(WHISPER_MODELS);
const key = process.env.GROQ_API_KEY;
if (!key) console.warn("GROQ_API_KEY not set: speech is transcribed only in browsers that can do it themselves");

async function transcribe(wav: Buffer, lang: Lang): Promise<{ text: string; model: string }> {
  return withQuota(
    async (model) => {
      const form = new FormData();
      form.append("file", new Blob([wav], { type: "audio/wav" }), "speech.wav");
      form.append("model", model);
      form.append("language", lang);
      form.append("temperature", "0");
      form.append("response_format", "verbose_json");
      const res = await fetch(`${GROQ_URL}/audio/transcriptions`, {
        method: "POST",
        headers: { Authorization: `Bearer ${key}` },
        body: form,
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      await failIfNotOk(res, "Transcribe");
      const reply = (await res.json()) as { segments?: { text: string; no_speech_prob?: number }[] };
      const text = (reply.segments ?? [])
        .filter((s) => (s.no_speech_prob ?? 0) < NO_SPEECH_PROB && !HALLUCINATION_RE.test(s.text))
        .map((s) => s.text.trim())
        .join(lang === "zh" ? "" : " ")
        .trim();
      return { text, model };
    },
    ({ until }) => Promise.reject(new OutOfQuota(until)),
  );
}

function wavFrom(pcm: Buffer): Buffer {
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + pcm.length, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16); // fmt chunk size
  header.writeUInt16LE(1, 20); // PCM
  header.writeUInt16LE(1, 22); // mono
  header.writeUInt32LE(TRANSCRIBE_SAMPLE_RATE, 24);
  header.writeUInt32LE(TRANSCRIBE_SAMPLE_RATE * 2, 28); // bytes per second
  header.writeUInt16LE(2, 32); // bytes per frame
  header.writeUInt16LE(16, 34); // bits per sample
  header.write("data", 36);
  header.writeUInt32LE(pcm.length, 40);
  return Buffer.concat([header, pcm]);
}

function send(ws: WebSocket, msg: SpeechServerMessage): void {
  if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(msg));
}

/** A /ws/speech socket, already upgraded. Only a member of the room holding its token may use it. */
export function onSpeechConnection(ws: WebSocket, roomId: string, token: string): void {
  const clientId = memberByToken(roomId, token);
  if (!clientId) return ws.close(4003, "not in this room");
  if (!key) {
    send(ws, { type: "unavailable" });
    return ws.close(1000);
  }

  let utterance: { lang: Lang; chunks: Buffer[]; bytes: number; startedAt: number } | null = null;
  // Each line is transcribed as soon as it ends, but posted only after the ones before it
  let posted = Promise.resolve();

  const finish = (lang: Lang, pcm: Buffer, startedAt: number) => {
    const endedAt = Date.now();
    const transcribed = transcribe(wavFrom(pcm), lang).then(
      (result) => {
        // Timings only, never what was said. span against audio shows how far the stream fell behind
        console.info(
          `transcribe lang=${lang} audio=${(pcm.length / (TRANSCRIBE_SAMPLE_RATE * 2)).toFixed(1)}s span=${endedAt - startedAt}ms whisper=${Date.now() - endedAt}ms model=${result.model} chars=${result.text.length}`,
        );
        return result.text;
      },
      (error: unknown) => {
        if (error instanceof OutOfQuota) send(ws, { type: "paused", until: error.until });
        else console.warn((error as Error).message);
        return "";
      },
    );
    posted = posted.then(async () => {
      const text = await transcribed;
      if (text) postChat(roomId, clientId, text, true);
    });
  };

  ws.on("message", (data, isBinary) => {
    if (isBinary) {
      if (!utterance || !Buffer.isBuffer(data) || utterance.chunks.length >= MAX_UTTERANCE_FRAMES) return;
      const room = MAX_UTTERANCE_BYTES - utterance.bytes;
      if (room <= 0) return;
      const chunk = data.length > room ? data.subarray(0, room) : data;
      utterance.chunks.push(chunk);
      utterance.bytes += chunk.length;
      return;
    }

    let msg: SpeechClientMessage;
    try {
      msg = JSON.parse(data.toString());
    } catch {
      return;
    }
    // Valid JSON can still be null or a number; reading .type off null would crash the whole server.
    if (typeof msg !== "object" || msg === null) return;
    switch (msg.type) {
      case "start":
        utterance = LANGS.includes(msg.lang) ? { lang: msg.lang, chunks: [], bytes: 0, startedAt: Date.now() } : null;
        break;
      case "cancel":
        utterance = null;
        break;
      case "end": {
        const done = utterance;
        utterance = null;
        if (!done || done.bytes < MIN_UTTERANCE_BYTES) break;
        // The seat may have been given up since the socket opened
        if (memberByToken(roomId, token) !== clientId) return ws.close(4003, "not in this room");
        if (!uploadsPerMember(clientId)) break;
        const pcm = Buffer.concat(done.chunks);
        // 16-bit samples: an odd byte at the end is half of one
        finish(done.lang, pcm.length % 2 ? pcm.subarray(0, -1) : pcm, done.startedAt);
        break;
      }
    }
  });
  ws.on("close", () => {
    utterance = null;
  });
}
