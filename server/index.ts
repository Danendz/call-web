import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { WebSocketServer, type WebSocket } from "ws";
import { MAX_CHAT_LENGTH, MAX_STREAM_ID_LENGTH, ROOM_ID_RE, type ClientMessage } from "../shared/protocol.js";
import { clientIp, concurrencyLimit, ownHost, rateLimit, setSecurityHeaders } from "./limits.js";
import { disconnected, join, leave, postChat, relayToPeer, send } from "./rooms.js";
import { onSpeechConnection } from "./transcribe.js";

const PORT = Number(process.env.PORT ?? 8080);
const STATIC_DIR = path.resolve(process.env.STATIC_DIR ?? "dist/web");
const HEARTBEAT_MS = 20_000;

// Per IP: a reconnecting phone opens a few sockets a minute, a page reload one more.
const socketsPerIp = concurrencyLimit(6);
const connectsPerIp = rateLimit(30, 60_000);
// Per member: typing fast is a few messages in 10 s; a renegotiation sends a few dozen ICE candidates.
const chatPerMember = rateLimit(20, 10_000);
const signalsPerMember = rateLimit(200, 10_000);
const screenshotsPerMember = rateLimit(5, 10_000);
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
};

async function serveStatic(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  setSecurityHeaders(req, res);
  if (url.pathname === "/healthz") {
    res.writeHead(200, { "content-type": "text/plain" }).end("ok");
    return;
  }

  let filePath = path.join(STATIC_DIR, path.normalize(decodeURIComponent(url.pathname)));
  if (!filePath.startsWith(STATIC_DIR)) {
    res.writeHead(400).end();
    return;
  }
  const isFile = await stat(filePath).then((s) => s.isFile(), () => false);
  // SPA fallback: /, /r/<id> and anything unknown get index.html.
  if (!isFile) filePath = path.join(STATIC_DIR, "index.html");

  try {
    const body = await readFile(filePath);
    const isAsset = filePath.includes(`${path.sep}assets${path.sep}`);
    res.writeHead(200, {
      "content-type": CONTENT_TYPES[path.extname(filePath)] ?? "application/octet-stream",
      "cache-control": isAsset ? "public, max-age=31536000, immutable" : "no-cache",
    });
    res.end(body);
  } catch {
    res.writeHead(404, { "content-type": "text/plain" }).end("Not found (did you run `npm run build`?)");
  }
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
  if ((url.pathname !== "/ws" && !speech) || !ROOM_ID_RE.test(roomId) || !sameOrigin(req)) {
    socket.destroy();
    return;
  }
  const ip = clientIp(req);
  if (!connectsPerIp(ip) || !socketsPerIp.acquire(ip)) {
    socket.end("HTTP/1.1 429 Too Many Requests\r\n\r\n");
    return;
  }
  wss.handleUpgrade(req, socket, head, (ws) => {
    ws.on("close", () => socketsPerIp.release(ip));
    alive.add(ws);
    ws.on("pong", () => alive.add(ws));
    if (speech) onSpeechConnection(ws, roomId, url.searchParams.get("token") ?? "");
    else onConnection(ws, roomId, url.searchParams.get("clientId"), url.searchParams.get("token"));
  });
});

// Browsers always send Origin on a WebSocket; another site's page must not open our room sockets.
function sameOrigin(req: IncomingMessage): boolean {
  const origin = req.headers.origin;
  if (!origin) return true;
  try {
    return new URL(origin).host === ownHost(req);
  } catch {
    return false;
  }
}

function onConnection(ws: WebSocket, roomId: string, requestedId: string | null, token: string | null): void {
  const clientId = join(roomId, requestedId, token, ws);
  if (!clientId) {
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
        if (!chatPerMember(clientId)) break;
        const text = typeof msg.text === "string" ? msg.text.trim() : "";
        if (text && text.length <= MAX_CHAT_LENGTH) postChat(roomId, clientId, text, msg.spoken === true);
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
        const { camOn, screen } = msg;
        const validScreen = screen === null || (typeof screen === "string" && screen.length <= MAX_STREAM_ID_LENGTH);
        if (typeof camOn === "boolean" && validScreen && mediaPerMember(clientId)) {
          relayToPeer(roomId, clientId, { type: "media", camOn, screen });
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

server.listen(PORT, () => {
  console.log(`listening on :${PORT}, serving ${STATIC_DIR}`);
  if (!process.env.TURN_HOST) console.warn("TURN_HOST not set: only host candidates, calls work on LAN only");
});
