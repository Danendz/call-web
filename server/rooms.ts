import { randomUUID } from "node:crypto";
import { WebSocket } from "ws";
import type { ChatMessage, ServerMessage } from "../shared/protocol.js";
import { iceServersFor } from "./turn.js";

const MAX_PEERS = 2;
const MAX_CHAT_HISTORY = 200;
// How long a dropped socket keeps its seat, so a network switch resumes the call instead of ending it.
const RESUME_GRACE_MS = 20_000;
const CLIENT_ID_RE = /^[0-9a-f-]{36}$/;

interface Member {
  clientId: string;
  ws: WebSocket | null;
  polite: boolean;
  graceTimer?: NodeJS.Timeout;
}

interface Room {
  id: string;
  members: Map<string, Member>;
  chat: ChatMessage[];
}

const rooms = new Map<string, Room>();

export function send(ws: WebSocket | null, msg: ServerMessage): void {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function otherMember(room: Room, clientId: string): Member | undefined {
  for (const m of room.members.values()) if (m.clientId !== clientId) return m;
  return undefined;
}

/** Returns the joined clientId, or null if the room is full. */
export function join(roomId: string, requestedId: string | null, ws: WebSocket): string | null {
  let room = rooms.get(roomId);
  if (!room) {
    room = { id: roomId, members: new Map(), chat: [] };
    rooms.set(roomId, room);
  }

  const existing = requestedId ? room.members.get(requestedId) : undefined;
  if (existing) {
    clearTimeout(existing.graceTimer);
    if (existing.ws && existing.ws !== ws) existing.ws.terminate();
    existing.ws = ws;
    send(ws, {
      type: "joined",
      clientId: existing.clientId,
      resumed: true,
      peerPresent: otherMember(room, existing.clientId) !== undefined,
      polite: existing.polite,
      iceServers: iceServersFor(roomId),
      chat: room.chat,
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

  const clientId = requestedId && CLIENT_ID_RE.test(requestedId) ? requestedId : randomUUID();
  const peer = otherMember(room, clientId);
  // The newcomer is the polite peer in perfect negotiation; the one already waiting is impolite.
  const member: Member = { clientId, ws, polite: peer !== undefined };
  room.members.set(clientId, member);

  send(ws, {
    type: "joined",
    clientId,
    resumed: false,
    peerPresent: peer !== undefined,
    polite: member.polite,
    iceServers: iceServersFor(roomId),
    chat: room.chat,
  });
  if (peer) {
    peer.polite = false;
    send(peer.ws, { type: "peer-joined", polite: false });
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
  if (room.members.size === 0) rooms.delete(roomId);
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

export function postChat(roomId: string, clientId: string, text: string): void {
  const room = rooms.get(roomId);
  if (!room) return;
  const message: ChatMessage = { id: randomUUID(), from: clientId, text, ts: Date.now() };
  room.chat.push(message);
  if (room.chat.length > MAX_CHAT_HISTORY) room.chat.shift();
  for (const m of room.members.values()) send(m.ws, { type: "chat", message });
}
