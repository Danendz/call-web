// Small helpers for the JSON endpoints under /api.

import type { IncomingMessage, ServerResponse } from "node:http";
import type { ApiErrorCode } from "../shared/api.js";
import { ownHost } from "./limits.js";

// Every request body is a handful of short strings.
const MAX_BODY_BYTES = 4 * 1024;

export class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly code: ApiErrorCode,
  ) {
    super(code);
  }
}

export function sendJson(res: ServerResponse, status: number, body: unknown, headers: Record<string, string | string[]> = {}): void {
  res.writeHead(status, { "content-type": "application/json", "cache-control": "no-store", ...headers });
  res.end(JSON.stringify(body));
}

/** The body as a JSON object, or HttpError 400. Requiring application/json also makes another site's form preflight. */
export async function readJson(req: IncomingMessage): Promise<Record<string, unknown>> {
  if (!String(req.headers["content-type"] ?? "").startsWith("application/json")) throw new HttpError(415, "bad-request");
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of req as AsyncIterable<Buffer>) {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) throw new HttpError(413, "bad-request");
    chunks.push(chunk);
  }
  let body: unknown;
  try {
    body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new HttpError(400, "bad-request");
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) throw new HttpError(400, "bad-request");
  return body as Record<string, unknown>;
}

/** The field as a string of at most `max` characters, or HttpError 400. */
export function str(body: Record<string, unknown>, field: string, max: number): string {
  const value = body[field];
  if (typeof value !== "string" || value.length > max) throw new HttpError(400, "bad-request");
  return value;
}

export function cookie(req: IncomingMessage, name: string): string | undefined {
  for (const part of String(req.headers.cookie ?? "").split(";")) {
    const eq = part.indexOf("=");
    if (eq > 0 && part.slice(0, eq).trim() === name) return part.slice(eq + 1).trim();
  }
  return undefined;
}

/** False when a browser says the request comes from another site. Native clients send no Origin. */
export function sameOrigin(req: IncomingMessage): boolean {
  const origin = req.headers.origin;
  if (!origin) return true;
  try {
    return new URL(origin).host === ownHost(req);
  } catch {
    return false;
  }
}
