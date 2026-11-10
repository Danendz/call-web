// Password hashing with scrypt from node:crypto, and the secrets that are random enough for a plain SHA-256.

import { createHash, randomBytes, randomInt, scrypt, timingSafeEqual, type ScryptOptions } from "node:crypto";
import { MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH } from "../../shared/api.js";

// 2^15 × 8 × 128 bytes = 32 MiB per hash, about 50–100 ms: slow for a guesser, fine for a login.
const PARAMS = { N: 2 ** 15, r: 8, p: 1 };
const KEY_LENGTH = 32;
const MAXMEM = 64 * 1024 * 1024;

function derive(password: string, salt: Buffer, options: ScryptOptions): Promise<Buffer> {
  return new Promise((resolve, reject) =>
    scrypt(password.normalize("NFKC"), salt, KEY_LENGTH, { ...options, maxmem: MAXMEM }, (err, key) => (err ? reject(err) : resolve(key))),
  );
}

/** "scrypt$N$r$p$salt$hash", so the parameters can be raised later without breaking old hashes. */
export async function hashPassword(password: string): Promise<string> {
  const salt = randomBytes(16);
  const key = await derive(password, salt, PARAMS);
  return ["scrypt", PARAMS.N, PARAMS.r, PARAMS.p, salt.toString("base64"), key.toString("base64")].join("$");
}

export async function verifyPassword(password: string, stored: string): Promise<boolean> {
  const [scheme, N, r, p, salt, hash] = stored.split("$");
  if (scheme !== "scrypt" || !hash) return false;
  const expected = Buffer.from(hash, "base64");
  const key = await derive(password, Buffer.from(salt, "base64"), { N: Number(N), r: Number(r), p: Number(p) });
  return key.length === expected.length && timingSafeEqual(key, expected);
}

// Checked against when the username doesn't exist, so a login takes as long either way.
export const DUMMY_HASH = await hashPassword(randomBytes(16).toString("hex"));

// What a guesser tries first that still passes the length rule.
const COMMON = new Set([
  "1234567890",
  "12345678910",
  "0123456789",
  "1q2w3e4r5t",
  "qwertyuiop",
  "1qaz2wsx3edc",
  "qwerty1234",
  "qwerty12345",
  "password12",
  "password123",
  "password1234",
  "passw0rd123",
  "iloveyou12",
  "abcdefghij",
  "abc1234567",
  "aaaaaaaaaa",
  "1111111111",
  "0000000000",
  "5201314520",
  "woaini1314",
  "a123456789",
  "qq123456789",
  "1234qwerty",
  "letmein123",
  "welcome123",
  "football12",
  "baseball12",
  "sunshine12",
  "princess12",
  "dragon1234",
]);

/** Why the password can't be used, or null when it can. */
export function passwordProblem(username: string, password: string): "weak-password" | null {
  if (password.length < MIN_PASSWORD_LENGTH || password.length > MAX_PASSWORD_LENGTH) return "weak-password";
  const lower = password.toLowerCase();
  if (COMMON.has(lower) || lower.includes(username) || new Set(lower).size < 4) return "weak-password";
  return null;
}

export function sha256(value: string): Buffer {
  return createHash("sha256").update(value).digest();
}

// Crockford base32: no I, L, O or U, so a code read aloud or retyped from a screenshot isn't misread.
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** A random code like "7K2M-Q9XD-…", `groups` groups of 4 (5 bits per character). */
export function newCode(groups: number): string {
  const chars = Array.from({ length: groups * 4 }, () => ALPHABET[randomInt(ALPHABET.length)]);
  return chars.join("").replace(/(.{4})(?=.)/g, "$1-");
}

/** Undoes what retyping does to a code: case, dashes and spaces, and the letters that look like digits. */
export function normalizeCode(code: string): string {
  return code
    .toUpperCase()
    .replace(/[\s-]/g, "")
    .replace(/[IL]/g, "1")
    .replace(/O/g, "0");
}
