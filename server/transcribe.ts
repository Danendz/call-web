// Speech to chat on the server, for browsers whose own recognition doesn't work (in China it goes to Google), for
// live subtitles from a browser, and for the native app without its on-device model: the client streams each
// utterance over /ws/speech while it's being said, and each line comes back to be posted to the room's chat as if the
// speaker had typed it. When the client asks for live text, the line so far also goes to the other person as it
// grows. The recognizing itself is in the speech service (speechService.ts), which sleeps while nobody uses it: here
// the member and their limits are checked, and each socket is passed through to one of its own there.

import WebSocket from "ws";
import {
  LANGS,
  MAX_CHAT_LENGTH,
  MAX_LIVE_LENGTH,
  MAX_UTTERANCE_S,
  TRANSCRIBE_SAMPLE_RATE,
  type Lang,
  type SpeechClientMessage,
  type SpeechServerMessage,
} from "../shared/protocol.js";
import { rateLimit } from "./limits.js";
import { memberByToken, postChat, relayToPeer, tierOf } from "./rooms.js";
import type { SpeechDownstreamMessage, SpeechUpstreamMessage } from "./speechService.js";

// The speech service on the private network; a local run has it on this machine
const SPEECH_URL = `ws://${process.env.SPEECH_HOST || "localhost"}:${process.env.SPEECH_PORT || 8090}`;
const SPEECH_AUTH = `Bearer ${process.env.SPEECH_SECRET ?? ""}`;
// Waking the speech service from sleep takes a few seconds
const CONNECT_TIMEOUT_MS = 30_000;
// 16-bit mono at the agreed rate; audio past this is dropped
const MAX_UTTERANCE_BYTES = MAX_UTTERANCE_S * TRANSCRIBE_SAMPLE_RATE * 2;
// The client sends ~25 frames a second; a cap on the count keeps tiny frames from costing more than their bytes
const MAX_UTTERANCE_FRAMES = 2000;
// Nobody talks for longer than they're in the call, so more audio than time is someone spending the server's CPU.
// An anonymous room's IP gets an hour of audio a day.
const secondsPerMember = rateLimit(90, 60_000);
const anonSecondsPerIp = rateLimit(60 * 60, 24 * 60 * 60_000);

function send(ws: WebSocket, msg: SpeechServerMessage): void {
  if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(msg));
}

function cut(text: string, max: number): string {
  return text.length > max ? text.slice(0, max) : text;
}

/** A /ws/speech socket, already upgraded. Only a member of the room holding its token may use it. */
export function onSpeechConnection(ws: WebSocket, roomId: string, token: string, ip: string): void {
  const clientId = memberByToken(roomId, token);
  if (!clientId) return ws.close(4003, "not in this room");

  // The seat may have been given up since the socket opened
  const member = () => memberByToken(roomId, token) === clientId;
  let utterance: { bytes: number; frames: number } | null = null;
  // Of the last utterance: its lines come back after it ended
  let lang: Lang = "en";

  // Opened now, so a sleeping speech service is awake by the first utterance. What the client sends meanwhile waits,
  // bound by the member's audio limit.
  const upstream = new WebSocket(SPEECH_URL, { headers: { authorization: SPEECH_AUTH }, handshakeTimeout: CONNECT_TIMEOUT_MS });
  const waiting: (Buffer | string)[] = [];
  const forward = (data: Buffer | string) => {
    if (upstream.readyState === upstream.OPEN) upstream.send(data);
    else if (upstream.readyState === upstream.CONNECTING) waiting.push(data);
  };
  const command = (msg: SpeechUpstreamMessage) => forward(JSON.stringify(msg));

  upstream.on("open", () => {
    for (const data of waiting) upstream.send(data);
    waiting.length = 0;
  });
  upstream.on("message", (data, isBinary) => {
    if (isBinary) return;
    let msg: SpeechDownstreamMessage;
    try {
      msg = JSON.parse(data.toString());
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null || !member()) return;
    if (msg.type === "unavailable") send(ws, msg);
    else if (msg.type === "live" && typeof msg.id === "string" && typeof msg.text === "string")
      relayToPeer(roomId, clientId, { type: "chat-live", id: msg.id, text: cut(msg.text, MAX_LIVE_LENGTH), lang });
    else if (msg.type === "line" && typeof msg.text === "string")
      postChat(roomId, clientId, cut(msg.text, MAX_CHAT_LENGTH), true, typeof msg.liveId === "string" ? msg.liveId : undefined);
  });
  // A refused connection has only a code, no message
  upstream.on("error", (err: NodeJS.ErrnoException) => console.warn("speech service:", err.code ?? err.message));
  // The client reconnects with its next utterance, and that dials the speech service again: after a redeploy, or
  // while it was down
  upstream.on("close", () => ws.close(1011, "speech service closed"));
  ws.on("close", () => upstream.close());

  const audio = (data: Buffer) => {
    if (!utterance || utterance.frames >= MAX_UTTERANCE_FRAMES) return;
    // 16-bit samples: an odd byte at the end is half of one
    const bytes = Math.min(data.length, MAX_UTTERANCE_BYTES - utterance.bytes) & ~1;
    if (bytes <= 0) return;
    const seconds = bytes / (TRANSCRIBE_SAMPLE_RATE * 2);
    if (!secondsPerMember(clientId, seconds)) return;
    if (tierOf(roomId) !== "user" && !anonSecondsPerIp(ip, seconds)) return;
    utterance.bytes += bytes;
    utterance.frames++;
    forward(data.subarray(0, bytes));
  };

  ws.on("message", (data, isBinary) => {
    if (isBinary) {
      if (Buffer.isBuffer(data)) audio(data);
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
        if (!LANGS.includes(msg.lang)) {
          command({ type: "cancel" });
          utterance = null;
          break;
        }
        command({ type: "start", lang: msg.lang, live: msg.live === true });
        utterance = { bytes: 0, frames: 0 };
        lang = msg.lang;
        break;
      case "cancel":
        command({ type: "cancel" });
        utterance = null;
        break;
      case "end":
        if (!member()) return ws.close(4003, "not in this room");
        command({ type: "end" });
        utterance = null;
        break;
    }
  });
}
