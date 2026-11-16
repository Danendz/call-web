// A ring to a friend with no app open waits for one to open, instead of ending at once.

import assert from "node:assert/strict";
import { mock, test } from "node:test";
import type { WebSocket } from "ws";
import type { UserServerMessage } from "../shared/protocol.js";

const alice = { id: "00000000-0000-4000-8000-00000000000a", username: "alice" };
const bob = { id: "00000000-0000-4000-8000-00000000000b", username: "bob" };
const missed: string[] = [];

mock.module("./friends/store.js", {
  namedExports: {
    friendByName: async (_me: string, name: string) => (name === bob.username ? bob : undefined),
    addMissedCall: async (callee: string, caller: string) => void missed.push(`${caller}->${callee}`),
    missedAnswered: async () => {},
    friendIds: async () => [],
    lists: async () => undefined,
  },
});
const { onUserConnection } = await import("./friends/hub.js");

function client(user: typeof alice) {
  const sent: UserServerMessage[] = [];
  const handlers: Record<string, (data?: unknown) => void> = {};
  const ws = {
    readyState: 1,
    OPEN: 1,
    send: (data: string) => sent.push(JSON.parse(data)),
    on: (event: string, fn: (data?: unknown) => void) => (handlers[event] = fn),
    close: () => handlers.close?.(),
  };
  onUserConnection(ws as unknown as WebSocket, { userId: user.id, username: user.username, tokenHash: Buffer.from(user.id) } as never);
  return {
    sent,
    say: (msg: object) => handlers.message?.(Buffer.from(JSON.stringify(msg))),
    close: () => ws.close(),
  };
}

const tick = () => new Promise((r) => setImmediate(r));

test("a ring to an offline friend reaches them when they open the app, and times out into a missed call", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const caller = client(alice);
  caller.say({ type: "ring", to: "bob" });
  await tick();
  const ringing = caller.sent.find((m) => m.type === "ringing");
  assert.deepEqual(ringing && { ...ringing, callId: "" }, { type: "ringing", callId: "", to: "bob", presence: "offline" });
  assert.ok(!caller.sent.some((m) => m.type === "call-ended"));

  const callee = client(bob);
  assert.ok(callee.sent.some((m) => m.type === "incoming" && m.from === "alice"));

  // Closing the app again doesn't end it either: another open in time still rings
  callee.close();
  assert.ok(!caller.sent.some((m) => m.type === "call-ended"));

  mock.timers.tick(45_000);
  await tick();
  assert.ok(caller.sent.some((m) => m.type === "call-ended" && m.reason === "timeout"));
  assert.deepEqual(missed, [`${alice.id}->${bob.id}`]);
  caller.close();
  mock.timers.reset();
});
