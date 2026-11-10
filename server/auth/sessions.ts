// Sessions: an opaque random token held by the client, of which the database keeps only a SHA-256.

import { randomBytes } from "node:crypto";
import type { IncomingMessage } from "node:http";
import type { ClientKind } from "../../shared/api.js";
import { sql } from "../db/index.js";
import { cookie } from "../http.js";
import { sha256 } from "./password.js";

// __Host-: only ever set by this host over HTTPS, for the whole site, so a subdomain can't plant or read it.
export const SESSION_COOKIE = "__Host-session";
const SESSION_DAYS = 90;
// Pushed forward at most once a day, not on every request.
const EXTEND_AFTER_MS = 24 * 60 * 60_000;
// A reinstall or a cleared browser leaves an old session behind; past this many the oldest go.
const MAX_SESSIONS_PER_USER = 20;
const TOKEN_RE = /^[A-Za-z0-9_-]{43}$/;

export interface SessionUser {
  userId: string;
  username: string;
  tokenHash: Buffer;
}

function requireDb() {
  if (!sql) throw new Error("no database");
  return sql;
}

export async function createSession(userId: string, client: ClientKind): Promise<string> {
  const db = requireDb();
  const token = randomBytes(32).toString("base64url");
  await db`
    INSERT INTO sessions (token_hash, user_id, client_kind, expires_at)
    VALUES (${sha256(token)}, ${userId}, ${client}, now() + make_interval(days => ${SESSION_DAYS}))`;
  await db`
    DELETE FROM sessions WHERE token_hash IN (
      SELECT token_hash FROM sessions WHERE user_id = ${userId} ORDER BY created_at DESC OFFSET ${MAX_SESSIONS_PER_USER})`;
  return token;
}

/** The session cookie, or the native client's bearer token. */
function tokenOf(req: IncomingMessage): string | undefined {
  const auth = req.headers.authorization;
  const token = auth?.startsWith("Bearer ") ? auth.slice(7) : cookie(req, SESSION_COOKIE);
  return token && TOKEN_RE.test(token) ? token : undefined;
}

/** Who sent the request, or null when it carries no live session (or accounts are off). */
export async function sessionUser(req: IncomingMessage): Promise<SessionUser | null> {
  const token = tokenOf(req);
  if (!sql || !token) return null;
  const tokenHash = sha256(token);
  const [row] = await sql<{ userId: string; username: string; expiresAt: Date }[]>`
    SELECT u.id AS "userId", u.username, s.expires_at AS "expiresAt"
    FROM sessions s JOIN users u ON u.id = s.user_id
    WHERE s.token_hash = ${tokenHash} AND s.expires_at > now()`;
  if (!row) return null;
  if (row.expiresAt.getTime() - Date.now() < SESSION_DAYS * 24 * 60 * 60_000 - EXTEND_AFTER_MS) {
    await sql`UPDATE sessions SET expires_at = now() + make_interval(days => ${SESSION_DAYS}) WHERE token_hash = ${tokenHash}`;
  }
  return { userId: row.userId, username: row.username, tokenHash };
}

export async function revokeSession(tokenHash: Buffer): Promise<void> {
  await requireDb()`DELETE FROM sessions WHERE token_hash = ${tokenHash}`;
}

/** Signs the user out everywhere, except the session `keep` when given. */
export async function revokeSessions(userId: string, keep?: Buffer): Promise<void> {
  const db = requireDb();
  if (keep) await db`DELETE FROM sessions WHERE user_id = ${userId} AND token_hash <> ${keep}`;
  else await db`DELETE FROM sessions WHERE user_id = ${userId}`;
}

export function sessionCookie(token: string): string {
  // Browsers cap a cookie's life at 400 days; the session's own expiry in the database is what counts.
  return `${SESSION_COOKIE}=${token}; Path=/; Max-Age=34560000; HttpOnly; Secure; SameSite=Strict`;
}

export const CLEAR_SESSION_COOKIE = `${SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict`;

