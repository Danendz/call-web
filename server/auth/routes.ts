// /api/auth/*, /api/me and /api/invites. Every body is shape-checked and every route rate-limited.

import type { IncomingMessage, ServerResponse } from "node:http";
import {
  MAX_PASSWORD_LENGTH,
  MAX_UNUSED_INVITES,
  USERNAME_RE,
  type ClientKind,
  type InvitesResponse,
  type MeResponse,
  type NewInviteResponse,
  type SessionResponse,
} from "../../shared/api.js";
import { sql } from "../db/index.js";
import { HttpError, readJson, sameOrigin, sendJson, str } from "../http.js";
import { clientIp, rateLimit } from "../limits.js";
import * as accounts from "./accounts.js";
import { passwordProblem } from "./password.js";
import { CLEAR_SESSION_COOKIE, createSession, revokeSession, revokeSessions, sessionCookie, sessionUser, type SessionUser } from "./sessions.js";

const MAX_CODE_LENGTH = 64;
const HOUR = 60 * 60_000;

// Each guess costs a scrypt hash, so these stay low; a shared IP (a carrier NAT) still fits a few people.
const apiPerIp = rateLimit(120, 60_000);
const loginsPerIp = rateLimit(5, 60_000);
const loginsPerUsername = rateLimit(10, HOUR);
const registersPerIp = rateLimit(3, HOUR);
const recoveriesPerIp = rateLimit(3, HOUR);
const recoveriesPerUsername = rateLimit(5, HOUR);
const passwordChecksPerUser = rateLimit(5, HOUR);
const invitesPerUser = rateLimit(10, 24 * HOUR);

type Handler = (req: IncomingMessage, res: ServerResponse, ip: string) => Promise<void>;

function limit(allowed: boolean): void {
  if (!allowed) throw new HttpError(429, "rate-limited");
}

function clientKind(body: Record<string, unknown>): ClientKind {
  const client = body.client;
  if (client !== "web" && client !== "native") throw new HttpError(400, "bad-request");
  return client;
}

/** Lowercased, so "Name" and "name" are one account. */
function username(body: Record<string, unknown>): string {
  return str(body, "username", 64).trim().toLowerCase();
}

function password(body: Record<string, unknown>, field = "password"): string {
  return str(body, field, MAX_PASSWORD_LENGTH);
}

async function signedIn(req: IncomingMessage): Promise<SessionUser> {
  const user = await sessionUser(req);
  if (!user) throw new HttpError(401, "unauthorized");
  return user;
}

/** Starts a session and answers with it: a cookie for the web client, the token itself for the native one. */
async function startSession(res: ServerResponse, userId: string, name: string, client: ClientKind, recoveryCode?: string): Promise<void> {
  const token = await createSession(userId, client);
  const body: SessionResponse = { username: name, ...(client === "native" && { token }), ...(recoveryCode && { recoveryCode }) };
  sendJson(res, 200, body, client === "web" ? { "set-cookie": sessionCookie(token) } : {});
}

const routes: Record<string, Handler> = {
  "POST /api/auth/register": async (req, res, ip) => {
    const body = await readJson(req);
    const name = username(body);
    const pw = password(body);
    const invite = str(body, "invite", MAX_CODE_LENGTH);
    const client = clientKind(body);
    // Checked before the limit: getting the rules wrong shouldn't use up an attempt
    if (!USERNAME_RE.test(name)) throw new HttpError(400, "invalid-username");
    const problem = passwordProblem(name, pw);
    if (problem) throw new HttpError(400, problem);
    limit(registersPerIp(ip));
    const result = await accounts.register(name, pw, invite);
    if (result === "invalid-invite") throw new HttpError(403, "invalid-invite");
    if (result === "username-taken") throw new HttpError(409, "username-taken");
    await startSession(res, result.userId, name, client, result.recoveryCode);
  },

  "POST /api/auth/login": async (req, res, ip) => {
    const body = await readJson(req);
    const name = username(body);
    const pw = password(body);
    const client = clientKind(body);
    limit(loginsPerIp(ip) && loginsPerUsername(name));
    const userId = USERNAME_RE.test(name) ? await accounts.login(name, pw) : null;
    if (!userId) throw new HttpError(401, "invalid-credentials");
    await startSession(res, userId, name, client);
  },

  "POST /api/auth/logout": async (req, res) => {
    const user = await sessionUser(req);
    if (user) await revokeSession(user.tokenHash);
    res.writeHead(204, { "set-cookie": CLEAR_SESSION_COOKIE }).end();
  },

  "POST /api/auth/recover": async (req, res, ip) => {
    const body = await readJson(req);
    const name = username(body);
    const code = str(body, "recoveryCode", MAX_CODE_LENGTH);
    const pw = password(body);
    const client = clientKind(body);
    const problem = passwordProblem(name, pw);
    if (problem) throw new HttpError(400, problem);
    limit(recoveriesPerIp(ip) && recoveriesPerUsername(name));
    const result = USERNAME_RE.test(name) ? await accounts.recover(name, code, pw) : null;
    if (!result) throw new HttpError(403, "invalid-recovery");
    await revokeSessions(result.userId);
    await startSession(res, result.userId, name, client, result.recoveryCode);
  },

  "POST /api/auth/password": async (req, res) => {
    const user = await signedIn(req);
    const body = await readJson(req);
    const current = password(body);
    const next = password(body, "newPassword");
    limit(passwordChecksPerUser(user.userId));
    if (!(await accounts.checkPassword(user.userId, current))) throw new HttpError(403, "invalid-credentials");
    const problem = passwordProblem(user.username, next);
    if (problem) throw new HttpError(400, problem);
    await accounts.setPassword(user.userId, next);
    await revokeSessions(user.userId, user.tokenHash);
    res.writeHead(204).end();
  },

  "POST /api/auth/delete": async (req, res) => {
    const user = await signedIn(req);
    const body = await readJson(req);
    const pw = password(body);
    limit(passwordChecksPerUser(user.userId));
    if (!(await accounts.checkPassword(user.userId, pw))) throw new HttpError(403, "invalid-credentials");
    await accounts.deleteAccount(user.userId);
    res.writeHead(204, { "set-cookie": CLEAR_SESSION_COOKIE }).end();
  },

  "GET /api/me": async (req, res) => {
    const user = await signedIn(req);
    sendJson(res, 200, { username: user.username } satisfies MeResponse);
  },

  "GET /api/invites": async (req, res) => {
    const user = await signedIn(req);
    sendJson(res, 200, { unused: await accounts.unusedInvites(user.userId), max: MAX_UNUSED_INVITES } satisfies InvitesResponse);
  },

  "POST /api/invites": async (req, res) => {
    const user = await signedIn(req);
    limit(invitesPerUser(user.userId));
    const invite = await accounts.createInvite(user.userId);
    if (!invite) throw new HttpError(409, "too-many-invites");
    sendJson(res, 200, invite satisfies NewInviteResponse);
  },
};

/** Handles /api/*; returns false for any other path. */
export function serveApi(req: IncomingMessage, res: ServerResponse, pathname: string): boolean {
  if (!pathname.startsWith("/api/")) return false;
  const ip = clientIp(req);
  const handler = routes[`${req.method} ${pathname}`];
  const run = async () => {
    // A page on another site can't send a cookie here (SameSite=Strict), and this stops anything that slips past
    if (!sameOrigin(req)) throw new HttpError(403, "forbidden");
    limit(apiPerIp(ip));
    if (!handler) throw new HttpError(404, "not-found");
    if (!sql) throw new HttpError(503, "unavailable");
    await handler(req, res, ip);
  };
  run().catch((err: unknown) => {
    if (res.headersSent) return;
    if (err instanceof HttpError) return sendJson(res, err.status, { error: err.code });
    console.error(err);
    sendJson(res, 500, { error: "unavailable" });
  });
  return true;
}
