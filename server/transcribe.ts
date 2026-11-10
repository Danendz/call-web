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
import { rateLimit, type Tier } from "./limits.js";
import { memberByToken, postChat, tierOf } from "./rooms.js";

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
// Whisper's own guesses that a segment is made up, at the cutoffs Whisper itself uses: noise, a low
// average log-probability of its tokens (unsure), or text that compresses too well (a phrase on repeat)
const NO_SPEECH_PROB = 0.6;
const MIN_AVG_LOGPROB = -1;
const MAX_COMPRESSION_RATIO = 2.4;
// A line that's only a filler: in the chat it's noise even when it was really said, since it was heard anyway
const FILLER_RE = /^(?:嗯|啊|哦|呃|额|唔|mm+|hm+|uh+|um+|ah+|oh+|huh|mhm)+$/;
// What Whisper says most often when it hears only noise (video endings). Said on purpose it's usually
// longer, so as a whole line it's dropped when the audio was short or Whisper wasn't sure
const THANKS_RE =
  /^(?:(?:thank you|thanks)(?: (?:so much|very much|all|everyone|for watching|for listening))? ?)+$|^(?:谢谢|感谢)+(?:大家|你|您)?(?:的)?(?:观看|收看|收听)?$/;
// Preroll and the closing pause are ~1.1s of this, so it's about 1.4s of speech
const SHORT_AUDIO_S = 2.5;
const SURE_AVG_LOGPROB = -0.5;
// One utterance every couple of seconds of nonstop talking; past that someone is spending the quota.
// An anonymous room gets half, and an hour of audio a day per IP.
const uploadsPerMember: Record<Tier, (key: string) => boolean> = { anon: rateLimit(15, 60_000), user: rateLimit(30, 60_000) };
const anonSecondsPerIp = rateLimit(60 * 60, 24 * 60 * 60_000);
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

interface Segment {
  text: string;
  no_speech_prob?: number;
  avg_logprob?: number;
  compression_ratio?: number;
}

/** Why a segment or the whole line was dropped; only these go to the log, never the text. */
type Dropped = Record<"noise" | "unsure" | "repeat" | "credits" | "filler" | "thanks", number>;

function keep(segments: Segment[], lang: Lang, seconds: number, dropped: Dropped): string {
  const kept = segments.filter((s) => {
    const reason =
      (s.no_speech_prob ?? 0) >= NO_SPEECH_PROB
        ? "noise"
        : (s.avg_logprob ?? 0) < MIN_AVG_LOGPROB
          ? "unsure"
          : (s.compression_ratio ?? 0) > MAX_COMPRESSION_RATIO
            ? "repeat"
            : HALLUCINATION_RE.test(s.text)
              ? "credits"
              : null;
    if (reason) dropped[reason]++;
    return !reason;
  });
  const text = kept
    .map((s) => s.text.trim())
    .join(lang === "zh" ? "" : " ")
    .trim();
  const words = text.toLowerCase().replace(/[\p{P}\p{S}\s]+/gu, " ").trim();
  if (FILLER_RE.test(words.replace(/ /g, ""))) {
    if (words) dropped.filler++;
    return "";
  }
  const sure = kept.every((s) => (s.avg_logprob ?? 0) >= SURE_AVG_LOGPROB);
  if (THANKS_RE.test(words) && (seconds < SHORT_AUDIO_S || !sure)) {
    dropped.thanks++;
    return "";
  }
  return text;
}

async function transcribe(
  wav: Buffer,
  lang: Lang,
  seconds: number,
): Promise<{ text: string; model: string; dropped: Dropped }> {
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
      const reply = (await res.json()) as { segments?: Segment[] };
      const dropped: Dropped = { noise: 0, unsure: 0, repeat: 0, credits: 0, filler: 0, thanks: 0 };
      const text = keep(Array.isArray(reply.segments) ? reply.segments : [], lang, seconds, dropped);
      return { text, model, dropped };
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
export function onSpeechConnection(ws: WebSocket, roomId: string, token: string, ip: string): void {
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
    const seconds = pcm.length / (TRANSCRIBE_SAMPLE_RATE * 2);
    const transcribed = transcribe(wavFrom(pcm), lang, seconds).then(
      (result) => {
        // Timings and drop counts only, never what was said. span against audio shows how far the stream fell behind
        const dropped = Object.entries(result.dropped)
          .filter(([, n]) => n)
          .map(([reason, n]) => `${reason}:${n}`)
          .join(",");
        console.info(
          `transcribe lang=${lang} audio=${seconds.toFixed(1)}s span=${endedAt - startedAt}ms whisper=${Date.now() - endedAt}ms model=${result.model} chars=${result.text.length} dropped=${dropped || "-"}`,
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
        const tier = tierOf(roomId) ?? "anon";
        const pcm = Buffer.concat(done.chunks);
        if (!uploadsPerMember[tier](clientId)) break;
        if (tier === "anon" && !anonSecondsPerIp(ip, pcm.length / (TRANSCRIBE_SAMPLE_RATE * 2))) break;
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
