import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { WebSocketServer, type WebSocket } from "ws";
import { NET_TEST_SIDES, NET_TEST_TARGETS, type NetTestBoard, type NetTestResult, type NetTestSide } from "../shared/nettest.js";
import { MAX_CHAT_LENGTH, ROOM_ID_RE, type ClientMessage } from "../shared/protocol.js";
import { disconnected, join, leave, postChat, relayToPeer, send } from "./rooms.js";

const PORT = Number(process.env.PORT ?? 8080);
const STATIC_DIR = path.resolve(process.env.STATIC_DIR ?? "dist/web");
const HEARTBEAT_MS = 20_000;

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

// Latest /nettest result from each side, kept in memory only (a redeploy clears it).
const netTestBoard: NetTestBoard = {};

async function handleNetTest(req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method === "POST") {
    const body = await readBody(req, 16 * 1024);
    const report = body === null ? null : parseNetTestReport(body);
    if (!report) {
      res.writeHead(400).end();
      return;
    }
    netTestBoard[report.side] = { ...report, at: Date.now() };
  }
  res.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" }).end(JSON.stringify(netTestBoard));
}

function parseNetTestReport(body: string): { side: NetTestSide; results: NetTestResult[] } | null {
  try {
    const raw = JSON.parse(body);
    if (!NET_TEST_SIDES.includes(raw.side) || !Array.isArray(raw.results)) return null;
    const ms = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 && v < 60_000 ? Math.round(v) : null);
    const results = NET_TEST_TARGETS.map(({ id }) => {
      const r = raw.results.find((x: { id?: unknown }) => x?.id === id);
      return { id, minMs: ms(r?.minMs), medianMs: ms(r?.medianMs), failed: ms(r?.failed) ?? 0 };
    });
    return { side: raw.side, results };
  } catch {
    return null;
  }
}

function readBody(req: IncomingMessage, limit: number): Promise<string | null> {
  return new Promise((resolve) => {
    let size = 0;
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > limit) {
        resolve(null);
        req.destroy();
      } else chunks.push(chunk);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString()));
    req.on("error", () => resolve(null));
  });
}

async function serveStatic(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  if (url.pathname === "/healthz") {
    res.writeHead(200, { "content-type": "text/plain" }).end("ok");
    return;
  }
  if (url.pathname === "/api/nettest") return handleNetTest(req, res);

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
  if (url.pathname !== "/ws" || !ROOM_ID_RE.test(roomId)) {
    socket.destroy();
    return;
  }
  wss.handleUpgrade(req, socket, head, (ws) => onConnection(ws, roomId, url.searchParams.get("clientId")));
});

function onConnection(ws: WebSocket, roomId: string, requestedId: string | null): void {
  const clientId = join(roomId, requestedId, ws);
  if (!clientId) {
    ws.close(4001, "room full");
    return;
  }

  alive.add(ws);
  ws.on("pong", () => alive.add(ws));
  ws.on("close", () => disconnected(roomId, clientId, ws));
  ws.on("message", (raw) => {
    let msg: ClientMessage;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    switch (msg.type) {
      case "signal":
        relayToPeer(roomId, clientId, { type: "signal", payload: msg.payload });
        break;
      case "chat": {
        const text = typeof msg.text === "string" ? msg.text.trim() : "";
        if (text && text.length <= MAX_CHAT_LENGTH) postChat(roomId, clientId, text);
        break;
      }
      case "leave":
        leave(roomId, clientId);
        ws.close(1000);
        break;
      case "ping":
        send(ws, { type: "pong" });
        break;
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
