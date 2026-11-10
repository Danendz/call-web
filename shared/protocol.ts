// Messages exchanged over the /ws socket. Shared by server and web client.

export const MAX_CHAT_LENGTH = 2000;
// WebRTC stream ids are UUIDs (Firefox wraps them in braces); anything longer is not one.
export const MAX_STREAM_ID_LENGTH = 64;
export const ROOM_ID_RE = /^[A-Za-z0-9_-]{6,32}$/;

// The two languages the chat is translated between, also what each side's speech is recognized in.
export const LANGS = ["zh", "en"] as const;
export type Lang = (typeof LANGS)[number];

// Speech streamed over /ws/speech: 16-bit little-endian mono PCM at this rate, one utterance of at most this long.
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

// /ws/speech?room=&token= carries one utterance at a time: "start", its audio as binary frames while it's
// being said, then "end" to transcribe it or "cancel" to drop it. Lines reach the chat in the order they ended.
export type SpeechClientMessage = { type: "start"; lang: Lang } | { type: "end" } | { type: "cancel" };

export type SpeechServerMessage =
  // Every speech model is out of quota until this epoch ms
  | { type: "paused"; until: number }
  // The server has no speech model configured
  | { type: "unavailable" };

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
  | { type: "ping" }
  // Tells the other person you saved a picture of the call; carries nothing
  | { type: "screenshot" }
  | MediaMessage;

// What you're sending, which the video tracks alone can't say: a camera turned off still sends black frames, and a
// stopped share leaves its track in place. `screen` is the id of the stream your shared screen arrives in, null
// when not sharing. `speakerOff`: you muted the other person's sound, so they know you can't hear them; missing from
// older builds, which means false. Sent on every change and whenever the peer connection is (re)made.
export interface MediaMessage {
  type: "media";
  camOn: boolean;
  screen: string | null;
  speakerOff?: boolean;
}

export type FullReason = "time-limit" | "not-invited" | "elsewhere";

export type ServerMessage =
  | {
      type: "joined";
      clientId: string;
      // Secret for resuming this seat and for /ws/speech; never shown to the other member
      token: string;
      // true when this socket replaced the same client's dropped socket
      resumed: boolean;
      peerPresent: boolean;
      polite: boolean;
      iceServers: IceServerConfig[];
      chat: ChatMessage[];
      // How long the call has run, when the other member is here. A duration, not a time: clocks differ.
      callElapsedMs?: number;
    }
  | { type: "peer-joined"; polite: boolean; callElapsedMs: number }
  | { type: "peer-left" }
  // The room can't be joined, and the client stops reconnecting. No reason: two people are already in it (or the
  // server is out of rooms). "time-limit": an anonymous room's call ran out its time. "not-invited": a private room
  // of two other accounts. "elsewhere": the same account joined it from another device.
  | { type: "full"; reason?: FullReason }
  | { type: "signal"; payload: SignalPayload }
  | { type: "chat"; message: ChatMessage }
  | { type: "chat-translation"; id: string; translation: Translation }
  | { type: "pong" }
  | { type: "screenshot" }
  | MediaMessage
  | { type: "error"; message: string };

// /ws/user: one per signed-in client, open while the app is. Carries the friends list with presence, and call
// requests. A call that's answered gets a private room, which only the two accounts can join.

export type Presence = "online" | "in-call" | "offline";

export interface FriendState {
  username: string;
  presence: Presence;
}

export interface MissedCall {
  from: string;
  at: number;
}

// A private room you left that the other person is still in: rejoin it by its id.
export interface OpenRoom {
  roomId: string;
  with: string;
}

// "answered-elsewhere": to the callee's other devices once one of them took the call.
export type CallEndReason = "declined" | "busy" | "offline" | "timeout" | "cancelled" | "unavailable" | "answered-elsewhere";

export type UserClientMessage =
  // Rings every device of a friend. One outgoing ring at a time: a new one cancels the last.
  | { type: "ring"; to: string }
  | { type: "cancel"; callId: string }
  | { type: "answer"; callId: string; accept: boolean }
  // Clears the missed calls in the next state
  | { type: "missed-seen" }
  | { type: "ping" };

export type UserServerMessage =
  // Everything the friends screen shows. Sent on connect and again whenever any of it changes.
  | {
      type: "state";
      username: string;
      friends: FriendState[];
      // Friend requests to you, and yours still waiting
      incoming: string[];
      outgoing: string[];
      blocked: string[];
      missed: MissedCall[];
      rooms: OpenRoom[];
    }
  // Your ring reached at least one of their devices
  | { type: "ringing"; callId: string; to: string }
  | { type: "incoming"; callId: string; from: string }
  // To the caller and to the device that answered: join this room
  | { type: "answered"; callId: string; roomId: string }
  | { type: "call-ended"; callId: string; reason: CallEndReason }
  | { type: "pong" };
