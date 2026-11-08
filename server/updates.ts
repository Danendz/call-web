// Native app updates: /updates/<file> streams Velopack's release feed and packages from the private Railway bucket
// that CI uploads to. Served from here, not from the bucket's host, because this host is known to work from China.

import { createHash, createHmac } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream as WebReadableStream } from "node:stream/web";
import { clientIp, concurrencyLimit, rateLimit } from "./limits.js";

// Velopack's names: releases.win.json, CallDanya-1.2.3-win-full.nupkg, CallDanya-win-Setup.exe; Android's
// (packaging/android/release.sh): android.json, call-danya-1.2.3.apk, call-danya.apk; the on-device speech model's
// files (uploaded by hand): sensevoice-small-int8-2024-07-17.onnx. Nothing with a slash, so a request can only name a
// file at the bucket's root.
const FILE_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

// An update check is one request, an update a few; a package is ~20 MB, so a few at a time per IP is plenty.
const requestsPerIp = rateLimit(60, 10 * 60_000);
const downloadsPerIp = concurrencyLimit(3);

const CONTENT_TYPES: Record<string, string> = {
  ".json": "application/json",
  ".nupkg": "application/zip",
  ".zip": "application/zip",
  ".exe": "application/vnd.microsoft.portable-executable",
  ".pkg": "application/octet-stream",
  ".apk": "application/vnd.android.package-archive",
};

type Bucket = { endpoint: URL; name: string; region: string; keyId: string; secret: string };

function bucketFromEnv(): Bucket | undefined {
  const { RELEASES_ENDPOINT, RELEASES_BUCKET, RELEASES_REGION, RELEASES_KEY_ID, RELEASES_SECRET } = process.env;
  if (!RELEASES_ENDPOINT || !RELEASES_BUCKET || !RELEASES_KEY_ID || !RELEASES_SECRET) return undefined;
  return {
    endpoint: new URL(RELEASES_ENDPOINT),
    name: RELEASES_BUCKET,
    region: RELEASES_REGION || "auto",
    keyId: RELEASES_KEY_ID,
    secret: RELEASES_SECRET,
  };
}

const bucket = bucketFromEnv();

/** Handles /updates/*; returns false for any other path. */
export function serveUpdates(req: IncomingMessage, res: ServerResponse, pathname: string): boolean {
  if (!pathname.startsWith("/updates/")) return false;
  const file = pathname.slice("/updates/".length);
  const ip = clientIp(req);
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405, { allow: "GET, HEAD" }).end();
  } else if (!bucket || !FILE_RE.test(file)) {
    res.writeHead(404).end();
  } else if (!requestsPerIp(ip) || !downloadsPerIp.acquire(ip)) {
    res.writeHead(429, { "retry-after": "60" }).end();
  } else {
    const abort = new AbortController();
    res.on("close", () => {
      downloadsPerIp.release(ip);
      abort.abort();
    });
    proxy(bucket, file, req, res, abort.signal).catch((err) => {
      // A download the client gave up on isn't an error
      if (abort.signal.aborted) return;
      console.error("updates:", err instanceof Error ? err.message : err);
      if (!res.headersSent) res.writeHead(502).end();
      else res.destroy();
    });
  }
  return true;
}

async function proxy(
  bucket: Bucket,
  file: string,
  req: IncomingMessage,
  res: ServerResponse,
  signal: AbortSignal,
): Promise<void> {
  const url = new URL(`/${file}`, `${bucket.endpoint.protocol}//${bucket.name}.${bucket.endpoint.host}`);
  // A byte range lets the app resume the ~240 MB speech model after a dropped connection; it isn't signed, which S3
  // allows for headers left out of SignedHeaders
  const range = req.headers.range;
  const upstream = await fetch(url, {
    method: req.method,
    headers: {
      ...signedHeaders(bucket, req.method ?? "GET", url),
      ...(typeof range === "string" && /^bytes=\d+-\d*$/.test(range) && { range }),
    },
    signal,
  });
  // A 403 is our credentials, not a missing file, so it's thrown and logged
  if (upstream.status === 404 || upstream.status === 416) {
    await upstream.body?.cancel();
    res.writeHead(upstream.status).end();
    return;
  }
  if (!upstream.ok) {
    await upstream.body?.cancel();
    throw new Error(`bucket answered ${upstream.status} for ${file}`);
  }

  const ext = file.slice(file.lastIndexOf("."));
  const headers: Record<string, string> = {
    "content-type": CONTENT_TYPES[ext] ?? "application/octet-stream",
    // A name with a version in it never changes; the feeds and the installers keep one name across releases
    "cache-control": /\d+\.\d+\.\d+/.test(file) ? "public, max-age=31536000, immutable" : "no-cache",
  };
  const length = upstream.headers.get("content-length");
  if (length) headers["content-length"] = length;
  const contentRange = upstream.headers.get("content-range");
  if (upstream.status === 206 && contentRange) headers["content-range"] = contentRange;
  headers["accept-ranges"] = "bytes";
  res.writeHead(upstream.status === 206 ? 206 : 200, headers);
  if (!upstream.body || req.method === "HEAD") {
    res.end();
    return;
  }
  await pipeline(Readable.fromWeb(upstream.body as WebReadableStream), res);
}

/** AWS Signature Version 4 for a GET or HEAD with no body, which is all the S3 API needs from us. */
function signedHeaders(bucket: Bucket, method: string, url: URL): Record<string, string> {
  const amzDate = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");
  const date = amzDate.slice(0, 8);
  const payload = "UNSIGNED-PAYLOAD";
  const signed = "host;x-amz-content-sha256;x-amz-date";
  const canonical = [
    method,
    url.pathname,
    "",
    `host:${url.host}`,
    `x-amz-content-sha256:${payload}`,
    `x-amz-date:${amzDate}`,
    "",
    signed,
    payload,
  ].join("\n");
  const scope = `${date}/${bucket.region}/s3/aws4_request`;
  const toSign = ["AWS4-HMAC-SHA256", amzDate, scope, createHash("sha256").update(canonical).digest("hex")].join("\n");
  const key = [date, bucket.region, "s3", "aws4_request"].reduce<Buffer | string>(
    (k, part) => createHmac("sha256", k).update(part).digest(),
    `AWS4${bucket.secret}`,
  );
  const signature = createHmac("sha256", key).update(toSign).digest("hex");
  return {
    "x-amz-content-sha256": payload,
    "x-amz-date": amzDate,
    authorization: `AWS4-HMAC-SHA256 Credential=${bucket.keyId}/${scope}, SignedHeaders=${signed}, Signature=${signature}`,
  };
}
