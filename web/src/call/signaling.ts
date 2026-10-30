import type { ClientMessage, ServerMessage } from "../../../shared/protocol";

export type SocketStatus = "connecting" | "open" | "reconnecting" | "closed";

const PING_EVERY_MS = 10_000;
// No pong for this long means the socket is dead even if the browser hasn't noticed (common after wifi -> 4G).
const DEAD_AFTER_MS = 25_000;
const MAX_BACKOFF_MS = 8_000;

/** WebSocket to the room with heartbeat and automatic reconnect. Reconnects reuse clientId so the server resumes our seat. */
export class Signaling {
  private ws: WebSocket | null = null;
  private clientId: string | null = null;
  private stopped = false;
  private attempt = 0;
  private lastPong = 0;
  private pingTimer?: number;
  private retryTimer?: number;

  constructor(
    private roomId: string,
    private onMessage: (msg: ServerMessage) => void,
    private onStatus: (status: SocketStatus) => void,
  ) {}

  connect(): void {
    this.stopped = false;
    this.open();
  }

  setClientId(id: string): void {
    this.clientId = id;
  }

  send(msg: ClientMessage): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  /** Reconnect right away, e.g. after the OS reports the network came back. */
  kick(): void {
    if (this.stopped || this.ws?.readyState === WebSocket.OPEN) return;
    clearTimeout(this.retryTimer);
    this.attempt = 0;
    this.dropSocket();
    this.open();
  }

  close(leave: boolean): void {
    if (leave) this.send({ type: "leave" });
    this.stopped = true;
    clearTimeout(this.retryTimer);
    this.dropSocket();
    this.onStatus("closed");
  }

  private open(): void {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams({ room: this.roomId });
    if (this.clientId) params.set("clientId", this.clientId);
    const ws = new WebSocket(`${proto}//${location.host}/ws?${params}`);
    this.ws = ws;
    this.onStatus(this.attempt === 0 ? "connecting" : "reconnecting");

    ws.onopen = () => {
      this.attempt = 0;
      this.lastPong = Date.now();
      this.onStatus("open");
      this.pingTimer = window.setInterval(() => {
        if (Date.now() - this.lastPong > DEAD_AFTER_MS) return this.lost();
        this.send({ type: "ping" });
      }, PING_EVERY_MS);
    };
    ws.onmessage = (ev) => {
      let msg: ServerMessage;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      this.lastPong = Date.now();
      if (msg.type === "pong") return;
      if (msg.type === "full") this.stopped = true;
      this.onMessage(msg);
    };
    ws.onclose = () => this.lost();
  }

  private lost(): void {
    this.dropSocket();
    if (this.stopped) return;
    this.onStatus("reconnecting");
    const delay = Math.min(1000 * 2 ** this.attempt++, MAX_BACKOFF_MS);
    this.retryTimer = window.setTimeout(() => this.open(), delay);
  }

  private dropSocket(): void {
    clearInterval(this.pingTimer);
    const ws = this.ws;
    this.ws = null;
    if (!ws) return;
    ws.onopen = ws.onmessage = ws.onclose = null;
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close();
  }
}
