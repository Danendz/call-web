// Messages exchanged over the /ws socket. Shared by server and web client.

export const MAX_CHAT_LENGTH = 2000;
export const ROOM_ID_RE = /^[A-Za-z0-9_-]{6,32}$/;

export interface ChatMessage {
  id: string;
  from: string; // clientId of the sender
  text: string;
  ts: number;
}

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
  | { type: "chat"; text: string }
  | { type: "leave" }
  | { type: "ping" };

export type ServerMessage =
  | {
      type: "joined";
      clientId: string;
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
  | { type: "pong" }
  | { type: "error"; message: string };
