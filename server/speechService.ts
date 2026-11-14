// The speech service: SenseVoice and Silero (speechModels.ts) in a Railway service of their own, so their ~1 GB is
// held only while someone uses speech. With nobody connected it sends nothing, and Railway puts it to sleep until the
// web service connects again. It's reached only over the project's private network, from transcribe.ts, which has
// already checked the member and their limits: each connection here is one client's /ws/speech, passed through.

import { randomUUID, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { WebSocketServer, type RawData } from "ws";
import { Utterance } from "./speechLines.js";
import { speech, type Speech } from "./speechModels.js";
import type { Lang } from "../shared/protocol.js";

/** web → speech: a client's messages, already checked. Its 16-bit audio comes as binary frames. */
export type SpeechUpstreamMessage = { type: "start"; lang: Lang; live: boolean } | { type: "end" } | { type: "cancel" };
/** speech → web */
export type SpeechDownstreamMessage =
  | { type: "unavailable" }
  | { type: "live"; id: string; text: string }
  | { type: "line"; text: string; liveId?: string };

// Both services get it from the Railway config; a local run leaves it empty on both sides
const SECRET = Buffer.from(`Bearer ${process.env.SPEECH_SECRET ?? ""}`);
// Pings keep the service awake through a long silence in a call: Railway counts only what the service sends
const PING_MS = 30_000;
// Decodes waiting on the one recognizer: past this it's too far behind, and new utterances are ignored
const MAX_WAITING = 20;

let models: Speech | null | undefined;
void speech.then((loaded) => (models = loaded));

function authorized(header: string | undefined): boolean {
  const got = Buffer.from(header ?? "");
  return got.length === SECRET.length && timingSafeEqual(got, SECRET);
}

function int16ToFloat(data: Buffer): Float32Array {
  const samples = new Float32Array(data.length >> 1);
  for (let i = 0; i < samples.length; i++) samples[i] = data.readInt16LE(i * 2) / 32768;
  return samples;
}

const server = createServer((req, res) => {
  // Ready once the models are: a new deployment takes no connections before then
  const ok = req.url === "/healthz" && models;
  res.writeHead(ok ? 200 : 503).end();
});
const wss = new WebSocketServer({ noServer: true });

server.on("upgrade", (req, socket, head) => {
  if (!authorized(req.headers.authorization)) return socket.destroy();
  wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws));
});

wss.on("connection", (ws) => {
  const send = (msg: SpeechDownstreamMessage) => {
    if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(msg));
  };
  // Until the models are loaded, what comes in waits: the connection that woke the service brings the call's first
  // utterance, which would otherwise go unrecognized. The web service bounds it by the member's audio limit.
  let early: [RawData, boolean][] | null = [];
  void speech.then((loaded) => {
    if (!loaded) {
      send({ type: "unavailable" });
      return ws.close(1000);
    }
    const waited = early ?? [];
    early = null;
    for (const [data, isBinary] of waited) onMessage(data, isBinary);
  });
  const ping = setInterval(() => ws.ping(), PING_MS);
  let utterance: Utterance | null = null;

  const start = (live: boolean) => {
    // Too far behind: this utterance goes unrecognized
    if (!models || models.decoder.waiting() > MAX_WAITING) return;
    utterance = new Utterance(
      models.vad(live),
      models.decoder,
      live,
      {
        live: (id, text) => send({ type: "live", id, text }),
        line: (text, liveId, { seconds, waitedMs }) => {
          // Timings only, never what was said
          console.info(`speech line audio=${seconds.toFixed(1)}s waited=${waitedMs}ms chars=${text.length} live=${live}`);
          send({ type: "line", text, liveId });
        },
      },
      randomUUID,
    );
  };

  const onMessage = (data: RawData, isBinary: boolean) => {
    if (isBinary) {
      if (utterance && Buffer.isBuffer(data)) utterance.audio(int16ToFloat(data));
      return;
    }
    let msg: SpeechUpstreamMessage;
    try {
      msg = JSON.parse(data.toString());
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null) return;
    utterance?.[msg.type === "end" ? "end" : "cancel"]();
    utterance = null;
    if (msg.type === "start") start(msg.live === true);
  };
  ws.on("message", (data: RawData, isBinary) => (early ? early.push([data, isBinary]) : onMessage(data, isBinary)));
  ws.on("close", () => {
    early = null;
    clearInterval(ping);
    utterance?.cancel();
    utterance = null;
  });
});

// transcribe.ts dials 8090 when it isn't told a port
server.listen(Number(process.env.PORT ?? 8090),() => console.log("speech service listening"));
