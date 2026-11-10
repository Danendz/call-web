import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { WebSocketServer, type WebSocket } from "ws";
import {
  LANGS,
  MAX_CHAT_LENGTH,
  MAX_LIVE_ID_LENGTH,
  MAX_LIVE_LENGTH,
  MAX_STREAM_ID_LENGTH,
  ROOM_ID_RE,
  type ClientMessage,
} from "../shared/protocol.js";
import { clientIp, concurrencyLimit, rateLimit, setSecurityHeaders, type Tier } from "./limits.js";
import { disconnected, join, leave, postChat, relayToPeer, send, tierOf, type Joiner } from "./rooms.js";
import { onSpeechConnection } from "./transcribe.js";
import { serveUpdates } from "./updates.js";
import { migrate, sql } from "./db/index.js";
import { serveApi } from "./auth/routes.js";
import { sessionUser } from "./auth/sessions.js";
import { onUserConnection } from "./friends/hub.js";
import { sameOrigin } from "./http.js";

const PORT = Number(process.env.PORT ?? 8080);
const STATIC_DIR = path.resolve(process.env.STATIC_DIR ?? "dist/web");
const HEARTBEAT_MS = 20_000;

// A reconnecting phone opens a few sockets a minute, a page reload one more. Checked before the session lookup.
const connectsPerIp = rateLimit(30, 60_000);
// Anonymous sockets get less than a signed-in user, whose sockets also count against the account. A signed-in
// client in a call holds three (/ws/user, /ws, /ws/speech), and an account may be open on a few devices.
const anonConnectsPerIp = rateLimit(10, 60_000);
const connectsPerUser = rateLimit(30, 60_000);
const anonSocketsPerIp = concurrencyLimit(4);
const userSocketsPerIp = concurrencyLimit(8);
const socketsPerUser = concurrencyLimit(10);
// Per member: typing fast is a few messages in 10 s; a renegotiation sends a few dozen ICE candidates.
// Each chat message is translated by a paid model, so an anonymous room gets half.
const chatPerMember: Record<Tier, (key: string) => boolean> = { anon: rateLimit(10, 10_000), user: rateLimit(20, 10_000) };
const signalsPerMember = rateLimit(200, 10_000);
const screenshotsPerMember = rateLimit(5, 10_000);
// Live text is sent a few times a second while speaking
const livePerMember = rateLimit(80, 10_000);
// A media update per camera toggle or share start/stop; nobody clicks those 30 times in 10 s.
const mediaPerMember = rateLimit(30, 10_000);

const CONTENT_TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".json": "application/json",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
};

async function serveStatic(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  setSecurityHeaders(req, res);
  if (url.pathname === "/healthz") {
    res.writeHead(200, { "content-type": "text/plain" }).end("ok");
    return;
  }
  if (serveUpdates(req, res, url.pathname)) return;
  if (serveApi(req, res, url.pathname)) return;

  let filePath = path.join(STATIC_DIR, path.normalize(decodeURIComponent(url.pathname)));
  if (!filePath.startsWith(STATIC_DIR)) {
    res.writeHead(400).end();
    return;
  }
  // SPA fallback: /, /r/<id> and anything unknown get index.html.
  if (!(await isFile(filePath))) filePath = path.join(STATIC_DIR, "index.html");

  try {
    // A brotli copy made at build time (scripts/precompress.ts), for a browser that takes it
    const brotli = /\bbr\b/.test(String(req.headers["accept-encoding"] ?? "")) && (await isFile(`${filePath}.br`));
    const body = await readFile(brotli ? `${filePath}.br` : filePath);
    const isAsset = filePath.includes(`${path.sep}assets${path.sep}`);
    res.writeHead(200, {
      "content-type": CONTENT_TYPES[path.extname(filePath)] ?? "application/octet-stream",
      "cache-control": isAsset ? "public, max-age=31536000, immutable" : "no-cache",
      ...(brotli && { "content-encoding": "br", vary: "accept-encoding" }),
    });
    res.end(body);
  } catch {
    res.writeHead(404, { "content-type": "text/plain" }).end("Not found (did you run `npm run build`?)");
  }
}

function isFile(file: string): Promise<boolean> {
  return stat(file).then(
    (s) => s.isFile(),
    () => false,
  );
}

const server = createServer((req, res) => {
  serveStatic(req, res).catch((err) => {
    console.error(err);
    res.writeHead(500).end();
  });
});

const wss = new WebSocketServer({ noServer: true, maxPayload: 64 * 1024 });
const alive = new WeakSet<WebSocket>();

server.on("upgrade", (req, socket, head) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  const roomId = url.searchParams.get("room") ?? "";
  const speech = url.pathname === "/ws/speech";
  // The signed-in client's own socket: friends, presence and call requests
  const userSocket = url.pathname === "/ws/user";
  const roomSocket = url.pathname === "/ws" || speech;
  if (!(userSocket || (roomSocket && ROOM_ID_RE.test(roomId))) || !sameOrigin(req)) {
    socket.destroy();
    return;
  }
  const ip = clientIp(req);
  const tooMany = () => socket.end("HTTP/1.1 429 Too Many Requests\r\n\r\n");
  if (!connectsPerIp(ip)) return tooMany();
  // The web client's session cookie comes with the upgrade; the native client sends its bearer token
  sessionUser(req).then(
    (user) => {
      if (socket.destroyed) return;
      const userId = user?.userId ?? null;
      if (userSocket && !user) return void socket.end("HTTP/1.1 401 Unauthorized\r\n\r\n");
      if (!(userId ? connectsPerUser(userId) : anonConnectsPerIp(ip))) return tooMany();
      const release = holdSocket(ip, userId);
      if (!release) return tooMany();
      wss.handleUpgrade(req, socket, head, (ws) => {
        ws.on("close", release);
        alive.add(ws);
        ws.on("pong", () => alive.add(ws));
        if (speech) onSpeechConnection(ws, roomId, url.searchParams.get("token") ?? "", ip);
        else if (!userSocket) onConnection(ws, roomId, url.searchParams.get("clientId"), url.searchParams.get("token"), { ip, userId });
        else if (user) onUserConnection(ws, user);
      });
    },
    (err: unknown) => {
      console.error(err);
      socket.destroy();
    },
  );
});

/** Takes a socket slot for this IP (and account), returning its release, or null when they hold too many. */
function holdSocket(ip: string, userId: string | null): (() => void) | null {
  if (!userId) return anonSocketsPerIp.acquire(ip) ? () => anonSocketsPerIp.release(ip) : null;
  if (!userSocketsPerIp.acquire(ip)) return null;
  if (!socketsPerUser.acquire(userId)) {
    userSocketsPerIp.release(ip);
    return null;
  }
  return () => {
    userSocketsPerIp.release(ip);
    socketsPerUser.release(userId);
  };
}

function onConnection(ws: WebSocket, roomId: string, requestedId: string | null, token: string | null, who: Joiner): void {
  const clientId = join(roomId, requestedId, token, ws, who);
  const tier = tierOf(roomId);
  if (!clientId || !tier) {
    ws.close(4001, "room full");
    return;
  }

  ws.on("close", () => disconnected(roomId, clientId, ws));
  ws.on("message", (raw) => {
    let msg: ClientMessage;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    // Valid JSON can still be null or a number; reading .type off null would crash the whole server.
    if (typeof msg !== "object" || msg === null) return;
    switch (msg.type) {
      case "signal":
        if (signalsPerMember(clientId)) relayToPeer(roomId, clientId, { type: "signal", payload: msg.payload });
        break;
      case "chat": {
        if (!chatPerMember[tier](clientId)) break;
        const text = typeof msg.text === "string" ? msg.text.trim() : "";
        const liveId = typeof msg.liveId === "string" && msg.liveId.length <= MAX_LIVE_ID_LENGTH ? msg.liveId : undefined;
        if (text && text.length <= MAX_CHAT_LENGTH) postChat(roomId, clientId, text, msg.spoken === true, liveId);
        break;
      }
      case "chat-live": {
        const { id, text, lang } = msg;
        const valid =
          typeof id === "string" &&
          id.length > 0 &&
          id.length <= MAX_LIVE_ID_LENGTH &&
          typeof text === "string" &&
          text.length <= MAX_LIVE_LENGTH &&
          LANGS.includes(lang);
        if (valid && livePerMember(clientId)) relayToPeer(roomId, clientId, { type: "chat-live", id, text, lang });
        break;
      }
      case "leave":
        leave(roomId, clientId);
        ws.close(1000);
        break;
      case "ping":
        send(ws, { type: "pong" });
        break;
      case "screenshot":
        if (screenshotsPerMember(clientId)) relayToPeer(roomId, clientId, { type: "screenshot" });
        break;
      case "media": {
        const { camOn, screen, speakerOff = false } = msg;
        const validScreen = screen === null || (typeof screen === "string" && screen.length <= MAX_STREAM_ID_LENGTH);
        if (typeof camOn === "boolean" && validScreen && typeof speakerOff === "boolean" && mediaPerMember(clientId)) {
          relayToPeer(roomId, clientId, { type: "media", camOn, screen, speakerOff });
        }
        break;
      }
    }
  });
}

// Drop sockets that stopped answering pings (e.g. phone switched networks without a clean close).
setInterval(() => {
  for (const ws of wss.clients) {
    if (!alive.has(ws)) {
      ws.terminate();
      continue;
    }
    alive.delete(ws);
    ws.ping();
  }
}, HEARTBEAT_MS);

await migrate();
server.listen(PORT, () => {
  console.log(`listening on :${PORT}, serving ${STATIC_DIR}`);
  if (!process.env.TURN_HOST) console.warn("TURN_HOST not set: only host candidates, calls work on LAN only");
  if (!sql) console.warn("DATABASE_URL not set: accounts and friends are off, only room links work");
});
