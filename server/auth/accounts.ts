// Accounts and invite codes in the database. Callers check rate limits and input shapes first.

import { timingSafeEqual } from "node:crypto";
import { INVITE_TTL_DAYS, MAX_UNUSED_INVITES } from "../../shared/api.js";
import { sql } from "../db/index.js";
import { DUMMY_HASH, hashPassword, newCode, normalizeCode, sha256, verifyPassword } from "./password.js";

const INVITE_GROUPS = 4; // 80 bits
const RECOVERY_GROUPS = 6; // 120 bits

class UsernameTaken extends Error {}

function db() {
  if (!sql) throw new Error("no database");
  return sql;
}

function codeHash(code: string): Buffer {
  return sha256(normalizeCode(code));
}

export type RegisterResult = { userId: string; recoveryCode: string } | "invalid-invite" | "username-taken";

/** Uses up the invite and creates the account, both or neither. */
export async function register(username: string, password: string, invite: string): Promise<RegisterResult> {
  const inviteHash = codeHash(invite);
  // Checked before hashing the password, so a made-up code costs us nothing and says nothing about usernames
  const [valid] = await db()`
    SELECT 1 FROM invites
    WHERE code_hash = ${inviteHash} AND used_at IS NULL AND created_at > now() - make_interval(days => ${INVITE_TTL_DAYS})`;
  if (!valid) return "invalid-invite";

  const recoveryCode = newCode(RECOVERY_GROUPS);
  const passwordHash = await hashPassword(password);
  try {
    return await db().begin(async (tx) => {
      // The row lock makes a second redeem of the same code wait here, then find it used
      const [used] = await tx`
        UPDATE invites SET used_at = now()
        WHERE code_hash = ${inviteHash} AND used_at IS NULL AND created_at > now() - make_interval(days => ${INVITE_TTL_DAYS})
        RETURNING code_hash`;
      if (!used) return "invalid-invite" as const;
      const [user] = await tx<{ id: string }[]>`
        INSERT INTO users (username, password_hash, recovery_hash)
        VALUES (${username}, ${passwordHash}, ${codeHash(recoveryCode).toString("hex")})
        ON CONFLICT (username) DO NOTHING
        RETURNING id`;
      // Throwing rolls back, so the invite stays unused
      if (!user) throw new UsernameTaken();
      await tx`UPDATE invites SET used_by = ${user.id} WHERE code_hash = ${inviteHash}`;
      return { userId: user.id, recoveryCode };
    });
  } catch (err) {
    if (err instanceof UsernameTaken) return "username-taken";
    throw err;
  }
}

/** The user's id when the password is right. Takes as long for a username that doesn't exist. */
export async function login(username: string, password: string): Promise<string | null> {
  const [user] = await db()<{ id: string; passwordHash: string }[]>`
    SELECT id, password_hash AS "passwordHash" FROM users WHERE username = ${username}`;
  const ok = await verifyPassword(password, user?.passwordHash ?? DUMMY_HASH);
  return user && ok ? user.id : null;
}

export async function checkPassword(userId: string, password: string): Promise<boolean> {
  const [user] = await db()<{ passwordHash: string }[]>`SELECT password_hash AS "passwordHash" FROM users WHERE id = ${userId}`;
  return verifyPassword(password, user?.passwordHash ?? DUMMY_HASH);
}

export async function setPassword(userId: string, password: string): Promise<void> {
  const passwordHash = await hashPassword(password);
  await db()`UPDATE users SET password_hash = ${passwordHash} WHERE id = ${userId}`;
}

/** With the right recovery code, sets the new password and a new recovery code (the old one is spent). */
export async function recover(username: string, recoveryCode: string, password: string): Promise<{ userId: string; recoveryCode: string } | null> {
  const [user] = await db()<{ id: string; recoveryHash: string }[]>`
    SELECT id, recovery_hash AS "recoveryHash" FROM users WHERE username = ${username}`;
  const given = codeHash(recoveryCode);
  const stored = Buffer.from(user?.recoveryHash ?? "", "hex");
  if (!user || stored.length !== given.length || !timingSafeEqual(stored, given)) return null;

  const next = newCode(RECOVERY_GROUPS);
  const passwordHash = await hashPassword(password);
  // Only if the code is still the one checked: two recoveries at once can't both win
  const [updated] = await db()`
    UPDATE users SET password_hash = ${passwordHash}, recovery_hash = ${codeHash(next).toString("hex")}
    WHERE id = ${user.id} AND recovery_hash = ${user.recoveryHash}
    RETURNING id`;
  return updated ? { userId: user.id, recoveryCode: next } : null;
}

export async function deleteAccount(userId: string): Promise<void> {
  await db()`DELETE FROM users WHERE id = ${userId}`;
}

export async function unusedInvites(userId: string): Promise<number> {
  const [{ n }] = await db()<{ n: number }[]>`
    SELECT count(*)::int AS n FROM invites
    WHERE created_by = ${userId} AND used_at IS NULL AND created_at > now() - make_interval(days => ${INVITE_TTL_DAYS})`;
  return n;
}

/** A new code, or null when the user already holds the most unused ones. `userId` null: made by scripts/invite.ts. */
export async function createInvite(userId: string | null): Promise<{ code: string; expiresAt: number } | null> {
  const code = newCode(INVITE_GROUPS);
  const created = await db().begin(async (tx) => {
    if (userId) {
      // Serializes one user's invite creations, so two at once can't both pass the count
      await tx`SELECT 1 FROM users WHERE id = ${userId} FOR UPDATE`;
      const [{ n }] = await tx<{ n: number }[]>`
        SELECT count(*)::int AS n FROM invites
        WHERE created_by = ${userId} AND used_at IS NULL AND created_at > now() - make_interval(days => ${INVITE_TTL_DAYS})`;
      if (n >= MAX_UNUSED_INVITES) return null;
    }
    const [row] = await tx<{ createdAt: Date }[]>`
      INSERT INTO invites (code_hash, created_by) VALUES (${codeHash(code)}, ${userId}) RETURNING created_at AS "createdAt"`;
    return row;
  });
  return created && { code, expiresAt: created.createdAt.getTime() + INVITE_TTL_DAYS * 24 * 60 * 60_000 };
}

// Expired sessions and invite codes would otherwise stay in the tables for good.
if (sql) {
  const cleanup = sql;
  setInterval(() => {
    Promise.all([
      cleanup`DELETE FROM sessions WHERE expires_at < now()`,
      cleanup`DELETE FROM invites WHERE used_at IS NULL AND created_at < now() - make_interval(days => ${INVITE_TTL_DAYS})`,
    ]).catch((err: Error) => console.warn("cleanup:", err.message));
  }, 60 * 60_000).unref();
}
