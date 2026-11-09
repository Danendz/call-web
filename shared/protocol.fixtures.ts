// One example of every message shape in protocol.ts, with and without its optional fields. The native client's
// Rust types are written by hand; `npm run fixtures` writes these to fixtures.json and a Rust test parses each
// one, so a change to the messages or constants that the Rust side doesn't follow fails its tests. Typecheck keeps
// these honest.

import {
  LANGS,
  MAX_CHAT_LENGTH,
  MAX_STREAM_ID_LENGTH,
  MAX_UTTERANCE_S,
  ROOM_ID_RE,
  TRANSCRIBE_SAMPLE_RATE,
  type ClientMessage,
  type ServerMessage,
  type SpeechClientMessage,
  type SpeechServerMessage,
} from "./protocol.js";

export const constants = {
  MAX_CHAT_LENGTH,
  MAX_STREAM_ID_LENGTH,
  ROOM_ID_RE: ROOM_ID_RE.source,
  LANGS,
  TRANSCRIBE_SAMPLE_RATE,
  MAX_UTTERANCE_S,
};

const chatMessage = { id: "m1", from: "c1", text: "你好", ts: 1_700_000_000_000 };
const translation = { lang: "zh", text: { zh: "你好", en: "Hello" } };

export const clientMessages = [
  { type: "signal", payload: { kind: "description", description: { type: "offer", sdp: "v=0\r\n" } } },
  { type: "signal", payload: { kind: "description", description: { type: "rollback" } } },
  {
    type: "signal",
    payload: { kind: "candidate", candidate: { candidate: "candidate:1 1 udp 1 1.2.3.4 5 typ host", sdpMid: "0", sdpMLineIndex: 0 } },
  },
  { type: "chat", text: "hi" },
  { type: "chat", text: "hi", spoken: true },
  { type: "leave" },
  { type: "ping" },
  { type: "screenshot" },
  { type: "media", camOn: true, screen: null },
  { type: "media", camOn: false, screen: "{6f1c2c9e-0000-4000-8000-000000000000}" },
  { type: "media", camOn: true, screen: null, speakerOff: true },
] satisfies ClientMessage[];

export const serverMessages = [
  {
    type: "joined",
    clientId: "c1",
    token: "secret",
    resumed: false,
    peerPresent: false,
    polite: false,
    iceServers: [{ urls: ["stun:turn.danendz.com:3478"] }, { urls: ["turn:turn.danendz.com:3478"], username: "u", credential: "p" }],
    chat: [],
  },
  {
    type: "joined",
    clientId: "c1",
    token: "secret",
    resumed: true,
    peerPresent: true,
    polite: true,
    iceServers: [],
    chat: [chatMessage, { ...chatMessage, id: "m2", spoken: true, translation }],
    callElapsedMs: 61_000,
  },
  { type: "peer-joined", polite: true, callElapsedMs: 0 },
  { type: "peer-left" },
  { type: "full" },
  { type: "signal", payload: { kind: "description", description: { type: "answer", sdp: "v=0\r\n" } } },
  { type: "signal", payload: { kind: "candidate", candidate: null } },
  { type: "chat", message: chatMessage },
  { type: "chat-translation", id: "m1", translation },
  { type: "pong" },
  { type: "screenshot" },
  { type: "media", camOn: true, screen: "s1" },
  { type: "media", camOn: true, screen: null, speakerOff: true },
  { type: "error", message: "bad message" },
] satisfies ServerMessage[];

export const speechClientMessages = [
  { type: "start", lang: "zh" },
  { type: "end" },
  { type: "cancel" },
] satisfies SpeechClientMessage[];

export const speechServerMessages = [{ type: "paused", until: 1_700_000_000_000 }, { type: "unavailable" }] satisfies SpeechServerMessage[];
