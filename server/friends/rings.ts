// Call requests between friends: a ring reaches every open client of the callee until one answers, the caller
// cancels or it times out. A callee with no client open gets it if one opens in time. An answered call gets a
// private room only the two accounts can join.

import { randomBytes } from "node:crypto";
import { USERNAME_RE } from "../../shared/api.js";
import type { CallEndReason } from "../../shared/protocol.js";
import { rateLimit } from "../limits.js";
import { createPrivateRoom, inCall, type Invitee } from "../rooms.js";
import { presence, refresh, send, sendToUser, type UserSocket } from "./hub.js";
import * as store from "./store.js";

const RING_MS = 45_000;
const ringsPerUser = rateLimit(10, 60_000);

interface Ring {
  id: string;
  caller: Invitee;
  callee: Invitee;
  // The client that rang: it alone hears the answer
  from: UserSocket;
  timer: NodeJS.Timeout;
}

const rings = new Map<string, Ring>();

function missedCall(callee: Invitee, caller: Invitee): void {
  store
    .addMissedCall(callee.id, caller.id)
    .then(() => refresh([callee.id]))
    .catch((err: Error) => console.warn("missed call:", err.message));
}

/** Stops the ring on both sides; `missed` stores it for the callee to see later. */
function finish(ring: Ring, reason: CallEndReason, missed: boolean): void {
  if (!rings.delete(ring.id)) return;
  clearTimeout(ring.timer);
  send(ring.from.ws, { type: "call-ended", callId: ring.id, reason });
  sendToUser(ring.callee.id, { type: "call-ended", callId: ring.id, reason });
  if (missed) missedCall(ring.callee, ring.caller);
}

export async function ring(from: UserSocket, to: string): Promise<void> {
  const callId = randomBytes(12).toString("base64url");
  const ended = (reason: CallEndReason) => send(from.ws, { type: "call-ended", callId, reason });
  // One outgoing ring per account: a new one replaces the last
  for (const r of rings.values()) if (r.caller.id === from.userId) finish(r, "cancelled", true);
  if (!ringsPerUser(from.userId)) return ended("unavailable");

  const name = to.trim().toLowerCase();
  const callee = USERNAME_RE.test(name) ? await store.friendByName(from.userId, name) : undefined;
  if (!callee) return ended("unavailable");
  // The caller may have hung up while the database answered
  if (from.ws.readyState !== from.ws.OPEN) return;
  const caller: Invitee = { id: from.userId, username: from.username };
  if (inCall(callee.id)) {
    missedCall(callee, caller);
    return ended("busy");
  }

  const r: Ring = { id: callId, caller, callee, from, timer: setTimeout(() => finish(r, "timeout", true), RING_MS) };
  rings.set(callId, r);
  send(from.ws, { type: "ringing", callId, to: callee.username, presence: presence(callee.id) });
  sendToUser(callee.id, { type: "incoming", callId, from: caller.username });
}

export function cancel(sock: UserSocket, callId: string): void {
  const r = rings.get(callId);
  if (r && r.caller.id === sock.userId) finish(r, "cancelled", true);
}

export function answer(sock: UserSocket, callId: string, accept: boolean): void {
  const r = rings.get(callId);
  if (!r || r.callee.id !== sock.userId) return;
  if (!accept) return finish(r, "declined", false);

  const roomId = createPrivateRoom(r.caller, r.callee);
  if (!roomId) return finish(r, "unavailable", true);
  rings.delete(r.id);
  clearTimeout(r.timer);
  store
    .missedAnswered(r.caller.id, r.callee.id)
    .then(() => refresh([r.caller.id, r.callee.id]))
    .catch((err: Error) => console.warn("missed calls:", err.message));
  send(r.from.ws, { type: "answered", callId, roomId });
  send(sock.ws, { type: "answered", callId, roomId });
  sendToUser(r.callee.id, { type: "call-ended", callId, reason: "answered-elsewhere" }, sock);
}

/** A client opened: show it the rings already going to its account. */
export function connected(sock: UserSocket): void {
  for (const r of rings.values()) if (r.callee.id === sock.userId) send(sock.ws, { type: "incoming", callId: r.id, from: r.caller.username });
}

/** A client closed: its own ring ends. Rings to its account go on, for another client or one that opens later. */
export function disconnected(sock: UserSocket): void {
  for (const r of rings.values()) if (r.from === sock) finish(r, "cancelled", true);
}
