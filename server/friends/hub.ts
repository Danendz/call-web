// /ws/user: the signed-in clients that are open, and the friends state each one is shown.

import { randomUUID } from "node:crypto";
import { WebSocket } from "ws";
import type { Presence, UserClientMessage, UserServerMessage } from "../../shared/protocol.js";
import type { SessionUser } from "../auth/sessions.js";
import { rateLimit } from "../limits.js";
import { inCall, onRoomsChanged, openRooms } from "../rooms.js";
import * as rings from "./rings.js";
import * as store from "./store.js";

// Changes that come together (a call starting changes both sides) go out as one state.
const STATE_DELAY_MS = 50;
// Rings and answers are a few a minute; past this a client is misbehaving.
const messagesPerSocket = rateLimit(30, 10_000);

export interface UserSocket {
  key: string;
  ws: WebSocket;
  userId: string;
  username: string;
  tokenHash: Buffer;
}

const online = new Map<string, Set<UserSocket>>();
const pending = new Map<string, NodeJS.Timeout>();

export function send(ws: WebSocket, msg: UserServerMessage): void {
  if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

export function socketsOf(userId: string): UserSocket[] {
  return [...(online.get(userId) ?? [])];
}

export function sendToUser(userId: string, msg: UserServerMessage, except?: UserSocket): void {
  for (const sock of socketsOf(userId)) if (sock !== except) send(sock.ws, msg);
}

function presence(userId: string): Presence {
  if (inCall(userId)) return "in-call";
  return online.has(userId) ? "online" : "offline";
}

/** Sends each of these accounts that's online a fresh state, shortly. */
export function refresh(userIds: Iterable<string>): void {
  for (const id of userIds) {
    if (!online.has(id) || pending.has(id)) continue;
    pending.set(
      id,
      setTimeout(() => {
        pending.delete(id);
        pushState(id).catch((err: Error) => console.warn("friends state:", err.message));
      }, STATE_DELAY_MS),
    );
  }
}

async function pushState(userId: string): Promise<void> {
  const lists = await store.lists(userId);
  if (!lists) return;
  const msg: UserServerMessage = {
    type: "state",
    username: lists.username,
    friends: lists.friends.map((f) => ({ username: f.username, presence: presence(f.id) })),
    incoming: lists.incoming,
    outgoing: lists.outgoing,
    blocked: lists.blocked,
    missed: lists.missed,
    rooms: openRooms(userId),
  };
  sendToUser(userId, msg);
}

/** These accounts went online, offline, into or out of a call: they and their friends get a new state. */
export async function presenceChanged(userIds: string[]): Promise<void> {
  const ids = new Set(userIds);
  for (const id of userIds) for (const friend of await store.friendIds(id)) ids.add(friend);
  refresh(ids);
}

onRoomsChanged((ids) => {
  presenceChanged(ids).catch((err: Error) => console.warn("presence:", err.message));
});

/** Closes the account's open clients, except the one on session `keep`: they were signed out. */
export function closeSessions(userId: string, keep?: Buffer): void {
  for (const sock of socketsOf(userId)) if (!keep || !sock.tokenHash.equals(keep)) sock.ws.close(4401, "signed out");
}

export function closeSession(tokenHash: Buffer): void {
  for (const set of online.values()) for (const sock of set) if (sock.tokenHash.equals(tokenHash)) sock.ws.close(4401, "signed out");
}

/** A /ws/user socket, already upgraded, of a signed-in client. */
export function onUserConnection(ws: WebSocket, user: SessionUser): void {
  const sock: UserSocket = { key: randomUUID(), ws, userId: user.userId, username: user.username, tokenHash: user.tokenHash };
  const set = online.get(user.userId) ?? new Set();
  const cameOnline = set.size === 0;
  set.add(sock);
  online.set(user.userId, set);
  if (cameOnline) presenceChanged([user.userId]).catch((err: Error) => console.warn("presence:", err.message));
  else refresh([user.userId]);
  rings.connected(sock);

  ws.on("message", (raw) => {
    if (!messagesPerSocket(sock.key)) return;
    let msg: UserClientMessage;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    // Valid JSON can still be null or a number; reading .type off null would crash the whole server.
    if (typeof msg !== "object" || msg === null) return;
    switch (msg.type) {
      case "ring":
        if (typeof msg.to === "string") rings.ring(sock, msg.to).catch((err: Error) => console.warn("ring:", err.message));
        break;
      case "cancel":
        if (typeof msg.callId === "string") rings.cancel(sock, msg.callId);
        break;
      case "answer":
        if (typeof msg.callId === "string" && typeof msg.accept === "boolean") rings.answer(sock, msg.callId, msg.accept);
        break;
      case "missed-seen":
        store
          .missedSeen(sock.userId)
          .then(() => refresh([sock.userId]))
          .catch((err: Error) => console.warn("missed calls:", err.message));
        break;
      case "ping":
        send(ws, { type: "pong" });
        break;
    }
  });

  ws.on("close", () => {
    set.delete(sock);
    const wentOffline = set.size === 0 && online.get(user.userId) === set;
    if (wentOffline) online.delete(user.userId);
    rings.disconnected(sock);
    if (wentOffline) presenceChanged([user.userId]).catch((err: Error) => console.warn("presence:", err.message));
  });
}
