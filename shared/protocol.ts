// Messages exchanged over the /ws socket. Shared by server and web client.

export const MAX_CHAT_LENGTH = 2000;
export const ROOM_ID_RE = /^[A-Za-z0-9_-]{6,32}$/;

// The two languages the chat is translated between, also what each side's speech is recognized in.
export const LANGS = ["zh", "en"] as const;
export type Lang = (typeof LANGS)[number];

// Speech sent to /api/transcribe: 16-bit mono WAV at this rate, one utterance of at most this long.
export const TRANSCRIBE_SAMPLE_RATE = 16_000;
export const MAX_UTTERANCE_S = 30;

export interface Translation {
  lang: string; // ISO 639-1 code of the original message
  text: Record<Lang, string>;
}

export interface ChatMessage {
  id: string;
  from: string; // clientId of the sender
  text: string;
  ts: number;
  spoken?: boolean; // transcribed from the sender's speech
  translation?: Translation; // arrives after the message, in a chat-translation
}

export type TranscribeReply =
  | { ok: true }
  // Every speech model is out of quota until this epoch ms
  | { paused: true; until: number }
  // The server has no speech model configured
  | { unavailable: true };

export interface IceServerConfig {
  urls: string[];
  username?: string;
  credential?: string;
}

// Opaque WebRTC payloads relayed verbatim to the other peer.
export type SignalPayload =
  | { kind: "description"; description: { type: string; sdp?: string } }
  | { kind: "candidate"; candidate: unknown };

export type ClientMessage =
  | { type: "signal"; payload: SignalPayload }
  | { type: "chat"; text: string; spoken?: boolean }
  | { type: "leave" }
  | { type: "ping" };

export type ServerMessage =
  | {
      type: "joined";
      clientId: string;
      // Secret for resuming this seat and for /api/transcribe; never shown to the other member
      token: string;
      // true when this socket replaced the same client's dropped socket
      resumed: boolean;
      peerPresent: boolean;
      polite: boolean;
      iceServers: IceServerConfig[];
      chat: ChatMessage[];
    }
  | { type: "peer-joined"; polite: boolean }
  | { type: "peer-left" }
  | { type: "full" }
  | { type: "signal"; payload: SignalPayload }
  | { type: "chat"; message: ChatMessage }
  | { type: "chat-translation"; id: string; translation: Translation }
  | { type: "pong" }
  | { type: "error"; message: string };
