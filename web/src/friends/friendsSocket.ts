import type { CallEndReason, UserClientMessage, UserServerMessage } from "../../../shared/protocol";
import { loadSession } from "../account/session";

const PING_EVERY_MS = 20_000;
// No pong for this long means the socket is dead even if the browser hasn't noticed.
const DEAD_AFTER_MS = 45_000;
const MAX_BACKOFF_MS = 15_000;

export type FriendsLists = Extract<UserServerMessage, { type: "state" }>;

export interface FriendsState {
  connected: boolean;
  lists: FriendsLists | null;
  /** Your ring; callId is null until the server says it's ringing. */
  outgoing: { to: string; callId: string | null } | null;
  incoming: { callId: string; from: string }[];
  /** How your last ring ended, until dismissed. */
  notice: { reason: CallEndReason; with: string } | null;
}

/** The signed-in page's /ws/user socket: friends with presence, and rings both ways. Reconnects on its own. */
export class FriendsSocket {
  private state: FriendsState = { connected: false, lists: null, outgoing: null, incoming: [], notice: null };
  private listeners = new Set<() => void>();
  private ws: WebSocket | null = null;
  private stopped = true;
  private attempt = 0;
  private lastPong = 0;
  private pingTimer?: number;
  private retryTimer?: number;

  /** `onAnswered`: a call you made or took was answered; join this room. */
  constructor(private onAnswered: (roomId: string) => void) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = () => this.state;

  private set(patch: Partial<FriendsState>): void {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    clearTimeout(this.retryTimer);
    this.drop();
    this.set({ connected: false, outgoing: null, incoming: [] });
  }

  ring(to: string): void {
    if (this.send({ type: "ring", to })) this.set({ outgoing: { to, callId: null }, notice: null });
  }

  cancel(): void {
    const callId = this.state.outgoing?.callId;
    if (callId) this.send({ type: "cancel", callId });
    this.set({ outgoing: null });
  }

  answer(callId: string, accept: boolean): void {
    this.send({ type: "answer", callId, accept });
    this.set({ incoming: this.state.incoming.filter((c) => c.callId !== callId) });
  }

  missedSeen(): void {
    this.send({ type: "missed-seen" });
  }

  dismissNotice(): void {
    this.set({ notice: null });
  }

  private send(msg: UserClientMessage): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  private open(): void {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/ws/user`);
    this.ws = ws;
    let opened = false;
    ws.onopen = () => {
      opened = true;
      this.attempt = 0;
      this.lastPong = Date.now();
      this.set({ connected: true });
      this.pingTimer = window.setInterval(() => {
        if (Date.now() - this.lastPong > DEAD_AFTER_MS) return this.lost(true);
        this.send({ type: "ping" });
      }, PING_EVERY_MS);
    };
    ws.onmessage = (ev) => {
      let msg: UserServerMessage;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      this.lastPong = Date.now();
      this.onMessage(msg);
    };
    ws.onclose = (ev) => {
      // 4401: this session was signed out (here or from another device)
      if (ev.code === 4401) {
        this.stop();
        void loadSession();
        return;
      }
      this.lost(opened);
    };
  }

  private onMessage(msg: UserServerMessage): void {
    switch (msg.type) {
      case "state":
        this.set({ lists: msg });
        break;
      case "ringing":
        if (this.state.outgoing?.to === msg.to) this.set({ outgoing: { to: msg.to, callId: msg.callId } });
        break;
      case "incoming":
        if (!this.state.incoming.some((c) => c.callId === msg.callId)) this.set({ incoming: [...this.state.incoming, msg] });
        break;
      case "answered":
        this.set({ outgoing: null, incoming: this.state.incoming.filter((c) => c.callId !== msg.callId) });
        this.onAnswered(msg.roomId);
        break;
      case "call-ended": {
        const out = this.state.outgoing;
        // A refusal before "ringing" carries a callId we haven't seen yet
        if (out && (out.callId === msg.callId || out.callId === null)) {
          this.set({ outgoing: null, notice: msg.reason === "cancelled" ? null : { reason: msg.reason, with: out.to } });
        } else {
          this.set({ incoming: this.state.incoming.filter((c) => c.callId !== msg.callId) });
        }
        break;
      }
    }
  }

  private lost(wasOpen: boolean): void {
    this.drop();
    this.set({ connected: false, outgoing: null, incoming: [] });
    if (this.stopped) return;
    // Refused before opening may mean the session ended: check, and the page stops this socket if so
    if (!wasOpen) void loadSession();
    const delay = Math.min(1000 * 2 ** this.attempt++, MAX_BACKOFF_MS);
    this.retryTimer = window.setTimeout(() => this.open(), delay);
  }

  private drop(): void {
    clearInterval(this.pingTimer);
    const ws = this.ws;
    this.ws = null;
    if (!ws) return;
    ws.onopen = ws.onmessage = ws.onclose = null;
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close();
  }
}
