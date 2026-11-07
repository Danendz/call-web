import { randomBytes, randomUUID } from "node:crypto";
import { WebSocket } from "ws";
import type { ChatMessage, ServerMessage } from "../shared/protocol.js";
import { translatorFromEnv } from "./translate.js";
import { iceServersFor } from "./turn.js";

const MAX_PEERS = 2;
// Each room holds its chat in memory; this is far past what two people use.
const MAX_ROOMS = 100;
const MAX_CHAT_HISTORY = 200;
// How long a dropped socket keeps its seat, so a network switch resumes the call instead of ending it.
const RESUME_GRACE_MS = 20_000;
const CLIENT_ID_RE = /^[0-9a-f-]{36}$/;
// A rejoin within this gap (a reload, a dropped network) continues the call timer; after it, a new call starts.
const CALL_GAP_MS = 2 * 60_000;

interface Member {
  clientId: string;
  // Secret that proves a reconnect is this member. clientId is not one: the other person sees it in chat.
  token: string;
  ws: WebSocket | null;
  polite: boolean;
  graceTimer?: NodeJS.Timeout;
}

interface Room {
  id: string;
  members: Map<string, Member>;
  chat: ChatMessage[];
  // When both members were first here, and since when one has been alone: drive the call timer.
  callStart?: number;
  aloneSince?: number;
}

const rooms = new Map<string, Room>();
const translate = translatorFromEnv();
if (!translate) console.warn("GROQ_API_KEY and LLM_API_KEY not set: chat messages are not translated");

export function send(ws: WebSocket | null, msg: ServerMessage): void {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

/** Marks the room as having both members; keeps the running timer unless they were apart too long. */
function startCall(room: Room): number {
  const now = Date.now();
  if (room.callStart === undefined || (room.aloneSince !== undefined && now - room.aloneSince > CALL_GAP_MS)) room.callStart = now;
  room.aloneSince = undefined;
  return now - room.callStart;
}

function otherMember(room: Room, clientId: string): Member | undefined {
  for (const m of room.members.values()) if (m.clientId !== clientId) return m;
  return undefined;
}

/** Returns the joined clientId, or null if the room (or the server) is full. */
export function join(roomId: string, requestedId: string | null, token: string | null, ws: WebSocket): string | null {
  let room = rooms.get(roomId);
  if (!room) {
    if (rooms.size >= MAX_ROOMS) {
      send(ws, { type: "full" });
      return null;
    }
    room = { id: roomId, members: new Map(), chat: [] };
    rooms.set(roomId, room);
  }

  const seat = requestedId ? room.members.get(requestedId) : undefined;
  const existing = seat && token && seat.token === token ? seat : undefined;
  if (existing) {
    clearTimeout(existing.graceTimer);
    if (existing.ws && existing.ws !== ws) existing.ws.terminate();
    existing.ws = ws;
    const peerPresent = otherMember(room, existing.clientId) !== undefined;
    send(ws, {
      type: "joined",
      clientId: existing.clientId,
      token: existing.token,
      resumed: true,
      peerPresent,
      polite: existing.polite,
      iceServers: iceServersFor(roomId),
      chat: room.chat,
      ...(peerPresent && room.callStart !== undefined && { callElapsedMs: Date.now() - room.callStart }),
    });
    return existing.clientId;
  }

  if (room.members.size >= MAX_PEERS) {
    // A seat held only for resume (e.g. someone reloaded the page) yields to a fresh join.
    const ghost = [...room.members.values()].find((m) => m.ws === null);
    if (!ghost) {
      send(ws, { type: "full" });
      return null;
    }
    leave(roomId, ghost.clientId);
    if (!rooms.has(roomId)) rooms.set(roomId, room);
  }

  // Keeping the old id (seat expired, then reconnected) lets the chat history still show as ours.
  const clientId = requestedId && CLIENT_ID_RE.test(requestedId) && !room.members.has(requestedId) ? requestedId : randomUUID();
  const peer = otherMember(room, clientId);
  // The newcomer is the polite peer in perfect negotiation; the one already waiting is impolite.
  const member: Member = { clientId, token: randomBytes(32).toString("base64url"), ws, polite: peer !== undefined };
  room.members.set(clientId, member);
  const callElapsedMs = peer ? startCall(room) : undefined;

  send(ws, {
    type: "joined",
    clientId,
    token: member.token,
    resumed: false,
    peerPresent: peer !== undefined,
    polite: member.polite,
    iceServers: iceServersFor(roomId),
    chat: room.chat,
    ...(callElapsedMs !== undefined && { callElapsedMs }),
  });
  if (peer) {
    peer.polite = false;
    send(peer.ws, { type: "peer-joined", polite: false, callElapsedMs: callElapsedMs ?? 0 });
  }
  return clientId;
}

export function leave(roomId: string, clientId: string): void {
  const room = rooms.get(roomId);
  const member = room?.members.get(clientId);
  if (!room || !member) return;
  clearTimeout(member.graceTimer);
  room.members.delete(clientId);
  const peer = otherMember(room, clientId);
  if (peer) send(peer.ws, { type: "peer-left" });
  if (room.members.size === 1) room.aloneSince = Date.now();
  if (room.members.size === 0) {
    // join() can put an emptied room back (evicting a ghost seat); it starts a new call then
    room.callStart = room.aloneSince = undefined;
    rooms.delete(roomId);
  }
}

/** Socket closed without an explicit leave: hold the seat for a while in case the client reconnects. */
export function disconnected(roomId: string, clientId: string, ws: WebSocket): void {
  const member = rooms.get(roomId)?.members.get(clientId);
  if (!member || member.ws !== ws) return; // already replaced by a resumed socket
  member.ws = null;
  member.graceTimer = setTimeout(() => leave(roomId, clientId), RESUME_GRACE_MS);
}

export function relayToPeer(roomId: string, clientId: string, msg: ServerMessage): void {
  const room = rooms.get(roomId);
  if (!room) return;
  send(otherMember(room, clientId)?.ws ?? null, msg);
}

/** The clientId of the room member holding this token. */
export function memberByToken(roomId: string, token: string): string | null {
  if (!token) return null;
  for (const m of rooms.get(roomId)?.members.values() ?? []) if (m.token === token) return m.clientId;
  return null;
}

/** Posts the message as written, then its translation once it's back (kept in the history for a rejoin). */
export function postChat(roomId: string, clientId: string, text: string, spoken = false): void {
  const room = rooms.get(roomId);
  if (!room) return;
  const message: ChatMessage = { id: randomUUID(), from: clientId, text, ts: Date.now(), ...(spoken && { spoken }) };
  room.chat.push(message);
  if (room.chat.length > MAX_CHAT_HISTORY) room.chat.shift();
  for (const m of room.members.values()) send(m.ws, { type: "chat", message });

  void translate?.(text).then((translation) => {
    if (!translation) return;
    message.translation = translation;
    const current = rooms.get(roomId);
    for (const m of current?.members.values() ?? []) send(m.ws, { type: "chat-translation", id: message.id, translation });
  });
}
