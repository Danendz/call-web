// Friendships, blocks and missed calls in the database. Callers check rate limits and input shapes first.

import type { TransactionSql } from "postgres";
import type { MissedCall } from "../../shared/protocol.js";
import { sql } from "../db/index.js";
import type { Invitee } from "../rooms.js";

const MAX_OUTGOING_REQUESTS = 20;
const MAX_FRIENDS = 200;
const MAX_BLOCKS = 500;
// Missed calls past this many (oldest first) are dropped; the screen shows only the unseen ones.
const MAX_MISSED_CALLS = 100;
const MAX_MISSED_SHOWN = 50;

export type FriendResult = "ok" | "too-many";

function db() {
  if (!sql) throw new Error("no database");
  return sql;
}

/** A friendship row's key: the smaller id first. */
function pair(a: string, b: string): [string, string] {
  return a < b ? [a, b] : [b, a];
}

export async function userByName(username: string): Promise<Invitee | undefined> {
  const [user] = await db()<Invitee[]>`SELECT id, username FROM users WHERE username = ${username}`;
  return user;
}

/**
 * Sends a friend request, or accepts theirs when they already sent one. A user who blocked you never sees it in
 * their incoming list, and nothing tells you so.
 */
export async function request(me: string, target: Invitee): Promise<FriendResult> {
  if (target.id === me) return "ok";
  const [a, b] = pair(me, target.id);
  return db().begin(async (tx) => {
    // Asking someone you blocked means you changed your mind
    await tx`DELETE FROM blocks WHERE blocker = ${me} AND blocked = ${target.id}`;
    const [row] = await tx<{ status: string; requestedBy: string }[]>`
      SELECT status, requested_by AS "requestedBy" FROM friendships WHERE user_a = ${a} AND user_b = ${b} FOR UPDATE`;
    if (row?.status === "accepted" || row?.requestedBy === me) return "ok";
    if (await friendCount(tx, me) >= MAX_FRIENDS) return "too-many";
    if (row) {
      await tx`UPDATE friendships SET status = 'accepted' WHERE user_a = ${a} AND user_b = ${b}`;
      return "ok";
    }
    const [{ n }] = await tx<{ n: number }[]>`
      SELECT count(*)::int AS n FROM friendships WHERE requested_by = ${me} AND status = 'pending'`;
    if (n >= MAX_OUTGOING_REQUESTS) return "too-many";
    await tx`INSERT INTO friendships (user_a, user_b, status, requested_by) VALUES (${a}, ${b}, 'pending', ${me})`;
    return "ok";
  });
}

async function friendCount(tx: TransactionSql, me: string): Promise<number> {
  const [{ n }] = await tx<{ n: number }[]>`
    SELECT count(*)::int AS n FROM friendships WHERE (user_a = ${me} OR user_b = ${me}) AND status = 'accepted'`;
  return n;
}

/** Accepts their pending request, if there is one. */
export async function accept(me: string, target: Invitee): Promise<FriendResult> {
  const [a, b] = pair(me, target.id);
  return db().begin(async (tx) => {
    if (await friendCount(tx, me) >= MAX_FRIENDS) return "too-many";
    await tx`
      UPDATE friendships SET status = 'accepted'
      WHERE user_a = ${a} AND user_b = ${b} AND status = 'pending' AND requested_by = ${target.id}`;
    return "ok";
  });
}

/** Unfriends, declines their request or withdraws yours: whichever row there is goes. */
export async function remove(me: string, target: Invitee): Promise<void> {
  const [a, b] = pair(me, target.id);
  await db()`DELETE FROM friendships WHERE user_a = ${a} AND user_b = ${b}`;
}

/** Ends any friendship or request, and hides their future requests and rings. */
export async function block(me: string, target: Invitee): Promise<FriendResult> {
  if (target.id === me) return "ok";
  const [a, b] = pair(me, target.id);
  return db().begin(async (tx) => {
    const [{ n }] = await tx<{ n: number }[]>`SELECT count(*)::int AS n FROM blocks WHERE blocker = ${me}`;
    if (n >= MAX_BLOCKS) return "too-many";
    await tx`DELETE FROM friendships WHERE user_a = ${a} AND user_b = ${b}`;
    await tx`INSERT INTO blocks (blocker, blocked) VALUES (${me}, ${target.id}) ON CONFLICT DO NOTHING`;
    return "ok";
  });
}

export async function unblock(me: string, target: Invitee): Promise<void> {
  await db()`DELETE FROM blocks WHERE blocker = ${me} AND blocked = ${target.id}`;
}

/** The friend's account when the two are friends, which is what lets one ring the other. */
export async function friendByName(me: string, username: string): Promise<Invitee | undefined> {
  const [friend] = await db()<Invitee[]>`
    SELECT u.id, u.username FROM users u
    JOIN friendships f ON f.status = 'accepted'
      AND ((f.user_a = ${me} AND f.user_b = u.id) OR (f.user_b = ${me} AND f.user_a = u.id))
    WHERE u.username = ${username}`;
  return friend;
}

export async function friendIds(me: string): Promise<string[]> {
  const rows = await db()<{ id: string }[]>`
    SELECT CASE WHEN user_a = ${me} THEN user_b ELSE user_a END AS id
    FROM friendships WHERE (user_a = ${me} OR user_b = ${me}) AND status = 'accepted'`;
  return rows.map((r) => r.id);
}

export interface Lists {
  username: string;
  friends: Invitee[];
  incoming: string[];
  outgoing: string[];
  blocked: string[];
  missed: MissedCall[];
}

/** What the friends screen lists, apart from presence and open rooms (which live in memory). */
export async function lists(me: string): Promise<Lists | undefined> {
  const [user] = await db()<{ username: string }[]>`SELECT username FROM users WHERE id = ${me}`;
  if (!user) return undefined;
  const rows = await db()<{ id: string; username: string; status: string; requestedBy: string }[]>`
    SELECT u.id, u.username, f.status, f.requested_by AS "requestedBy"
    FROM friendships f JOIN users u ON u.id = CASE WHEN f.user_a = ${me} THEN f.user_b ELSE f.user_a END
    WHERE (f.user_a = ${me} OR f.user_b = ${me})
      AND NOT EXISTS (SELECT 1 FROM blocks WHERE blocker = ${me} AND blocked = u.id)
    ORDER BY u.username`;
  const blocked = await db()<{ username: string }[]>`
    SELECT u.username FROM blocks b JOIN users u ON u.id = b.blocked WHERE b.blocker = ${me} ORDER BY u.username`;
  const missed = await db()<{ from: string; at: Date }[]>`
    SELECT u.username AS "from", m.at FROM missed_calls m JOIN users u ON u.id = m.caller
    WHERE m.callee = ${me} AND m.seen_at IS NULL
    ORDER BY m.at DESC LIMIT ${MAX_MISSED_SHOWN}`;
  return {
    username: user.username,
    friends: rows.filter((r) => r.status === "accepted").map(({ id, username }) => ({ id, username })),
    incoming: rows.filter((r) => r.status === "pending" && r.requestedBy !== me).map((r) => r.username),
    outgoing: rows.filter((r) => r.status === "pending" && r.requestedBy === me).map((r) => r.username),
    blocked: blocked.map((r) => r.username),
    missed: missed.map((r) => ({ from: r.from, at: r.at.getTime() })),
  };
}

export async function addMissedCall(callee: string, caller: string): Promise<void> {
  await db()`INSERT INTO missed_calls (callee, caller) VALUES (${callee}, ${caller})`;
  await db()`
    DELETE FROM missed_calls WHERE id IN (
      SELECT id FROM missed_calls WHERE callee = ${callee} ORDER BY at DESC OFFSET ${MAX_MISSED_CALLS})`;
}

export async function missedSeen(me: string): Promise<void> {
  await db()`UPDATE missed_calls SET seen_at = now() WHERE callee = ${me} AND seen_at IS NULL`;
}
