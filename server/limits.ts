// Abuse limits: the service is for two people, so anything well past what a call does is cut off.

import type { IncomingMessage, ServerResponse } from "node:http";

/** The caller's IP. Railway's edge sets X-Real-IP; without it (a local run) the socket's address. */
export function clientIp(req: IncomingMessage): string {
  const real = req.headers["x-real-ip"];
  return (typeof real === "string" && real) || req.socket.remoteAddress || "";
}

/** At most `max` hits per key in each `windowMs`. Returns false once the key is over. */
export function rateLimit(max: number, windowMs: number): (key: string) => boolean {
  const windows = new Map<string, { count: number; resetAt: number }>();
  setInterval(() => {
    const now = Date.now();
    for (const [key, w] of windows) if (w.resetAt <= now) windows.delete(key);
  }, windowMs).unref();

  return (key) => {
    const now = Date.now();
    let w = windows.get(key);
    if (!w || w.resetAt <= now) {
      w = { count: 0, resetAt: now + windowMs };
      windows.set(key, w);
    }
    return ++w.count <= max;
  };
}

/** Counts what each key holds open (sockets per IP), refusing past `max`. */
export function concurrencyLimit(max: number) {
  const open = new Map<string, number>();
  return {
    acquire(key: string): boolean {
      const n = open.get(key) ?? 0;
      if (n >= max) return false;
      open.set(key, n + 1);
      return true;
    },
    release(key: string): void {
      const n = (open.get(key) ?? 1) - 1;
      if (n <= 0) open.delete(key);
      else open.set(key, n);
    },
  };
}

/** The page's own origin: X-Forwarded-Host on Railway, Host locally. */
export function ownHost(req: IncomingMessage): string {
  const forwarded = req.headers["x-forwarded-host"];
  return (typeof forwarded === "string" && forwarded) || req.headers.host || "";
}

const HOST_RE = /^[A-Za-z0-9.-]+(:\d+)?$/;

/**
 * Locks the page down to itself: scripts, styles, sockets and fetches only from this server, never
 * framed by another site (it asks for camera and mic), and the room link isn't sent on as a referrer.
 * blob: scripts are the inline AudioWorklet in speech/whisperRecorder.ts.
 */
export function setSecurityHeaders(req: IncomingMessage, res: ServerResponse): void {
  const host = ownHost(req);
  // 'self' covers same-origin wss: in current browsers; the explicit host is for older WebViews.
  const sockets = HOST_RE.test(host) ? ` wss://${host}` : "";
  res.setHeader(
    "content-security-policy",
    [
      "default-src 'self'",
      // 'wasm-unsafe-eval' compiles WebAssembly only (not JS eval): the mic's noise suppression runs as WASM
      "script-src 'self' blob: 'wasm-unsafe-eval'",
      "style-src 'self'",
      `connect-src 'self'${sockets}`,
      "img-src 'self' data: blob:",
      "media-src 'self' blob: mediastream:",
      "font-src 'self'",
      "object-src 'none'",
      "base-uri 'none'",
      "form-action 'none'",
      "frame-ancestors 'none'",
    ].join("; "),
  );
  res.setHeader("strict-transport-security", "max-age=31536000");
  res.setHeader("x-content-type-options", "nosniff");
  res.setHeader("x-frame-options", "DENY");
  res.setHeader("referrer-policy", "no-referrer");
  res.setHeader(
    "permissions-policy",
    "camera=(self), microphone=(self), display-capture=(self), geolocation=(), payment=(), usb=()",
  );
}
