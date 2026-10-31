// Speech to chat for browsers whose own recognition doesn't work (in China it goes to Google): the
// client cuts each utterance and POSTs it as WAV, Groq Whisper transcribes it, and it's posted to the
// room's chat as if the speaker had typed it

import type { IncomingMessage, ServerResponse } from "node:http";
import { LANGS, MAX_UTTERANCE_S, TRANSCRIBE_SAMPLE_RATE, type Lang, type TranscribeReply } from "../shared/protocol.js";
import { createQuota, failIfNotOk } from "./llm.js";
import { isMember, postChat } from "./rooms.js";

const GROQ_URL = "https://api.groq.com/openai/v1";
// Each has its own free quota (2h of audio an hour): when large-v3 runs out, turbo still has its own
const WHISPER_MODELS = ["whisper-large-v3", "whisper-large-v3-turbo"] as const;
const TIMEOUT_MS = 15_000;
// 16-bit mono at the agreed rate, plus room for the WAV header
const MAX_BODY = MAX_UTTERANCE_S * TRANSCRIBE_SAMPLE_RATE * 2 + 1024;
// Whisper's own guess that a segment is noise; above this the segment is dropped
const NO_SPEECH_PROB = 0.6;
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

async function transcribe(wav: Buffer, lang: Lang): Promise<string> {
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
      return (reply.segments ?? [])
        .filter((s) => (s.no_speech_prob ?? 0) < NO_SPEECH_PROB && !HALLUCINATION_RE.test(s.text))
        .map((s) => s.text.trim())
        .join(lang === "zh" ? "" : " ")
        .trim();
    },
    ({ until }) => Promise.reject(new OutOfQuota(until)),
  );
}

function readBody(req: IncomingMessage, limit: number): Promise<Buffer | null> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > limit) {
        resolve(null);
        req.destroy();
      } else chunks.push(chunk);
    });
    req.on("end", () => resolve(Buffer.concat(chunks)));
    req.on("error", reject);
  });
}

function reply(res: ServerResponse, status: number, body: TranscribeReply | { error: string }): void {
  res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
}

/** POST /api/transcribe?room=&client=&lang= with a WAV body, from a member of the room. */
export async function handleTranscribe(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (req.method !== "POST") return reply(res, 405, { error: "POST only" });
  const roomId = url.searchParams.get("room") ?? "";
  const clientId = url.searchParams.get("client") ?? "";
  const lang = url.searchParams.get("lang") as Lang;
  if (!isMember(roomId, clientId)) return reply(res, 403, { error: "not in this room" });
  if (!LANGS.includes(lang)) return reply(res, 400, { error: "unknown lang" });
  if (!key) return reply(res, 503, { unavailable: true });

  const wav = await readBody(req, MAX_BODY);
  if (!wav) return reply(res, 413, { error: "utterance too long" });

  try {
    const text = await transcribe(wav, lang);
    if (text) postChat(roomId, clientId, text, true);
    reply(res, 200, { ok: true });
  } catch (error) {
    if (error instanceof OutOfQuota) return reply(res, 429, { paused: true, until: error.until });
    console.warn((error as Error).message);
    reply(res, 502, { error: "transcription failed" });
  }
}
