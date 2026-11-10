// Who may join a private room: knowing its id must never be enough.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { WebSocket } from "ws";
import type { ServerMessage } from "../shared/protocol.js";
import { createPrivateRoom, join, leave, openRooms } from "./rooms.js";

function fakeSocket() {
  const sent: ServerMessage[] = [];
  const ws = { readyState: 1, send: (data: string) => sent.push(JSON.parse(data)), close() {}, terminate() {} };
  return { ws: ws as unknown as WebSocket, sent };
}

const alice = { id: "00000000-0000-4000-8000-00000000000a", username: "alice" };
const bob = { id: "00000000-0000-4000-8000-00000000000b", username: "bob" };
const ip = "127.0.0.1";

test("only the two invited accounts can join a private room", () => {
  const roomId = createPrivateRoom(alice, bob)!;

  const anon = fakeSocket();
  assert.equal(join(roomId, null, null, anon.ws, { ip, userId: null }), null);
  assert.deepEqual(anon.sent, [{ type: "full", reason: "not-invited" }]);

  const other = fakeSocket();
  assert.equal(join(roomId, null, null, other.ws, { ip, userId: "00000000-0000-4000-8000-00000000000c" }), null);
  assert.deepEqual(other.sent, [{ type: "full", reason: "not-invited" }]);

  assert.ok(join(roomId, null, null, fakeSocket().ws, { ip, userId: alice.id }));
  assert.ok(join(roomId, null, null, fakeSocket().ws, { ip, userId: bob.id }));
});

test("a seat's token resumes it only for the same account", () => {
  const roomId = createPrivateRoom(alice, bob)!;
  const first = fakeSocket();
  const clientId = join(roomId, null, null, first.ws, { ip, userId: alice.id })!;
  const token = (first.sent[0] as { token: string }).token;

  // A leaked token alone doesn't let someone else in
  const thief = fakeSocket();
  assert.equal(join(roomId, clientId, token, thief.ws, { ip, userId: null }), null);

  const again = fakeSocket();
  assert.equal(join(roomId, clientId, token, again.ws, { ip, userId: alice.id }), clientId);
  assert.equal((again.sent[0] as { resumed: boolean }).resumed, true);
});

test("joining from a second device takes over the account's seat", () => {
  const roomId = createPrivateRoom(alice, bob)!;
  const laptop = fakeSocket();
  join(roomId, null, null, laptop.ws, { ip, userId: alice.id });
  join(roomId, null, null, fakeSocket().ws, { ip, userId: bob.id });

  const phone = fakeSocket();
  assert.ok(join(roomId, null, null, phone.ws, { ip, userId: alice.id }));
  assert.deepEqual(laptop.sent.at(-1), { type: "full", reason: "elsewhere" });
  assert.equal((phone.sent[0] as { peerPresent: boolean }).peerPresent, true);
});

test("the Rooms tab lists a room the other person is still in, until both leave", () => {
  const roomId = createPrivateRoom(alice, bob)!;
  const a = join(roomId, null, null, fakeSocket().ws, { ip, userId: alice.id })!;
  const b = join(roomId, null, null, fakeSocket().ws, { ip, userId: bob.id })!;

  // Rooms from the other tests are still open
  const listed = (userId: string) => openRooms(userId).filter((r) => r.roomId === roomId);

  leave(roomId, b);
  assert.deepEqual(listed(bob.id), [{ roomId, with: "alice" }]);
  assert.deepEqual(listed(alice.id), []);

  leave(roomId, a);
  assert.deepEqual(listed(bob.id), []);
  // The id now names no private room: joining it makes a plain link room
  const late = fakeSocket();
  join(roomId, null, null, late.ws, { ip, userId: alice.id });
  assert.equal(late.sent[0].type, "joined");
});
