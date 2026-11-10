import { randomBytes, randomUUID } from "node:crypto";
import { WebSocket } from "ws";
import type { ChatMessage, FullReason, OpenRoom, ServerMessage } from "../shared/protocol.js";
import { rateLimit, type Tier } from "./limits.js";
import { translatorFromEnv } from "./translate.js";
import { iceServersFor } from "./turn.js";

const MAX_PEERS = 2;
// Each room holds its chat in memory; this is far past what two people use.
const MAX_ROOMS = 100;
// Anonymous rooms get only part of that, so signed-in users still find space when someone floods links.
const MAX_ANON_ROOMS = 20;
const MAX_LINK_ROOMS_PER_USER = 3;
const anonRoomsPerIp = rateLimit(5, 60 * 60_000);
// An anonymous call ends after this long; its TURN credentials expire then too (turn.ts).
const ANON_ROOM_MS = 2 * 60 * 60_000;
// A private room made by an answered call that neither side opens in this long is dropped.
const PRIVATE_ROOM_UNUSED_MS = 60_000;
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
  // The account that joined, null for an anonymous member
  userId: string | null;
  ws: WebSocket | null;
  polite: boolean;
  graceTimer?: NodeJS.Timeout;
}

/** An account invited into a private room. */
export interface Invitee {
  id: string;
  username: string;
}

/** Who is opening the room socket: the IP, and the account when the request carries a session. */
export interface Joiner {
  ip: string;
  userId: string | null;
}

interface Room {
  id: string;
  tier: Tier;
  // The account that created a link room, which counts against its limit
  ownerId: string | null;
  // A private room's two accounts: nobody else may join it, whatever they know of its id
  invited?: [Invitee, Invitee];
  expireTimer?: NodeJS.Timeout;
  members: Map<string, Member>;
  chat: ChatMessage[];
  // When both members were first here, and since when one has been alone: drive the call timer.
  callStart?: number;
  aloneSince?: number;
}

const rooms = new Map<string, Room>();
// Told which accounts' presence or open rooms may have changed (friends/hub.ts pushes them a new state).
let roomsChanged: (userIds: string[]) => void = () => {};

export function onRoomsChanged(listener: (userIds: string[]) => void): void {
  roomsChanged = listener;
}

/** `also`: accounts that just lost their seat, no longer among the members. */
function changed(room: Room, ...also: (string | null)[]): void {
  const ids = new Set(also.filter((id) => id !== null));
  for (const m of room.members.values()) if (m.userId) ids.add(m.userId);
  for (const u of room.invited ?? []) ids.add(u.id);
  if (ids.size) roomsChanged([...ids]);
}

function destroy(room: Room, ...also: (string | null)[]): void {
  if (rooms.get(room.id) !== room) return;
  clearTimeout(room.expireTimer);
  rooms.delete(room.id);
  changed(room, ...also);
}
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

/** A new room in the creator's tier, or undefined when that tier (or the server) is out of rooms. */
function createRoom(roomId: string, who: Joiner): Room | undefined {
  if (rooms.size >= MAX_ROOMS) return undefined;
  const all = [...rooms.values()];
  if (who.userId) {
    if (all.filter((r) => r.ownerId === who.userId).length >= MAX_LINK_ROOMS_PER_USER) return undefined;
  } else if (all.filter((r) => r.tier === "anon").length >= MAX_ANON_ROOMS || !anonRoomsPerIp(who.ip)) {
    return undefined;
  }
  const room: Room = { id: roomId, tier: who.userId ? "user" : "anon", ownerId: who.userId, members: new Map(), chat: [] };
  if (room.tier === "anon") room.expireTimer = setTimeout(() => expire(room), ANON_ROOM_MS);
  rooms.set(roomId, room);
  return room;
}

/** A room for an answered call between two accounts, or null when the server is out of rooms. */
export function createPrivateRoom(a: Invitee, b: Invitee): string | null {
  if (rooms.size >= MAX_ROOMS) return null;
  // The id is only an address: join() checks the account, so it needn't be secret, just not taken
  const id = randomBytes(16).toString("base64url");
  const room: Room = { id, tier: "user", ownerId: null, invited: [a, b], members: new Map(), chat: [] };
  room.expireTimer = setTimeout(() => room.members.size === 0 && destroy(room), PRIVATE_ROOM_UNUSED_MS).unref();
  rooms.set(id, room);
  return id;
}

/** Ends an anonymous room's call at its time limit. */
function expire(room: Room): void {
  const ids = [...room.members.values()].map((m) => m.userId);
  for (const m of room.members.values()) {
    clearTimeout(m.graceTimer);
    // "full" stops the client from reconnecting
    send(m.ws, { type: "full", reason: "time-limit" });
    m.ws?.close(4002, "time limit");
  }
  room.members.clear();
  destroy(room, ...ids);
}

function refuse(ws: WebSocket, reason?: FullReason): null {
  send(ws, { type: "full", ...(reason && { reason }) });
  return null;
}

/** Whether the account holds a seat in any room (a dropped one too, which may be resumed). */
export function inCall(userId: string): boolean {
  for (const room of rooms.values()) for (const m of room.members.values()) if (m.userId === userId) return true;
  return false;
}

/** Private rooms the account isn't in while the other account still is: the Rooms tab. */
export function openRooms(userId: string): OpenRoom[] {
  const open: OpenRoom[] = [];
  for (const room of rooms.values()) {
    const other = room.invited?.find((u) => u.id !== userId);
    if (!other || !room.invited?.some((u) => u.id === userId)) continue;
    const seated = [...room.members.values()].map((m) => m.userId);
    if (!seated.includes(userId) && seated.includes(other.id)) open.push({ roomId: room.id, with: other.username });
  }
  return open;
}

export function tierOf(roomId: string): Tier | undefined {
  return rooms.get(roomId)?.tier;
}

/** Returns the joined clientId, or null if the room (or the server) is full. */
export function join(roomId: string, requestedId: string | null, token: string | null, ws: WebSocket, who: Joiner): string | null {
  const room = rooms.get(roomId) ?? createRoom(roomId, who);
  if (!room) return refuse(ws);
  if (room.invited && !room.invited.some((u) => u.id === who.userId)) return refuse(ws, "not-invited");

  const seat = requestedId ? room.members.get(requestedId) : undefined;
  const existing = seat && token && seat.token === token && (!room.invited || seat.userId === who.userId) ? seat : undefined;
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
      iceServers: iceServersFor(roomId, room.tier),
      chat: room.chat,
      ...(peerPresent && room.callStart !== undefined && { callElapsedMs: Date.now() - room.callStart }),
    });
    return existing.clientId;
  }

  let evicted: string | null = null;
  // In a private room each account has one seat: joining from another device takes it over.
  const mine = room.invited && [...room.members.values()].find((m) => m.userId === who.userId);
  if (mine) {
    send(mine.ws, { type: "full", reason: "elsewhere" });
    mine.ws?.close(1000);
    removeMember(room, mine);
  }

  if (room.members.size >= MAX_PEERS) {
    // A seat held only for resume (e.g. someone reloaded the page) yields to a fresh join.
    const ghost = [...room.members.values()].find((m) => m.ws === null);
    if (!ghost) return refuse(ws);
    removeMember(room, ghost);
    evicted = ghost.userId;
  }

  // Keeping the old id (seat expired, then reconnected) lets the chat history still show as ours.
  const clientId = requestedId && CLIENT_ID_RE.test(requestedId) && !room.members.has(requestedId) ? requestedId : randomUUID();
  const peer = otherMember(room, clientId);
  // The newcomer is the polite peer in perfect negotiation; the one already waiting is impolite.
  const member: Member = { clientId, token: randomBytes(32).toString("base64url"), userId: who.userId, ws, polite: peer !== undefined };
  room.members.set(clientId, member);
  const callElapsedMs = peer ? startCall(room) : undefined;

  send(ws, {
    type: "joined",
    clientId,
    token: member.token,
    resumed: false,
    peerPresent: peer !== undefined,
    polite: member.polite,
    iceServers: iceServersFor(roomId, room.tier),
    chat: room.chat,
    ...(callElapsedMs !== undefined && { callElapsedMs }),
  });
  if (peer) {
    peer.polite = false;
    send(peer.ws, { type: "peer-joined", polite: false, callElapsedMs: callElapsedMs ?? 0 });
  }
  changed(room, evicted);
  return clientId;
}

/** Frees a seat, leaving the room in place even when it's now empty. */
function removeMember(room: Room, member: Member): void {
  clearTimeout(member.graceTimer);
  room.members.delete(member.clientId);
  const peer = otherMember(room, member.clientId);
  if (peer) send(peer.ws, { type: "peer-left" });
  if (room.members.size === 1) room.aloneSince = Date.now();
  if (room.members.size === 0) room.callStart = room.aloneSince = undefined;
}

export function leave(roomId: string, clientId: string): void {
  const room = rooms.get(roomId);
  const member = room?.members.get(clientId);
  if (!room || !member) return;
  removeMember(room, member);
  if (room.members.size === 0) destroy(room, member.userId);
  else changed(room, member.userId);
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
export function postChat(roomId: string, clientId: string, text: string, spoken = false, liveId?: string): void {
  const room = rooms.get(roomId);
  if (!room) return;
  const message: ChatMessage = {
    id: randomUUID(),
    from: clientId,
    text,
    ts: Date.now(),
    ...(spoken && { spoken }),
    ...(liveId !== undefined && { liveId }),
  };
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
