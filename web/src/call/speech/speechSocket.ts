// The server path's own socket, kept apart from signaling so a stalled audio frame on a lossy link
// never holds up the call's own messages. Utterances go out while they're being said (see
// SpeechClientMessage); what's sent while the socket is still connecting waits for it to open

import {
  MAX_UTTERANCE_S,
  TRANSCRIBE_SAMPLE_RATE,
  type Lang,
  type SpeechClientMessage,
  type SpeechServerMessage,
} from "../../../../shared/protocol";

// After a drop, the next utterance reconnects; not sooner than this, so a server that's down isn't hammered
const RETRY_MS = 2_000;
// What may wait for the socket to open: the few utterances kept while the browser's recognition was on trial
const MAX_QUEUED_BYTES = 3 * MAX_UTTERANCE_S * TRANSCRIBE_SAMPLE_RATE * 2;

export class SpeechSocket {
  private ws: WebSocket | null = null;
  private queue: (string | Int16Array)[] = [];
  private queuedBytes = 0;
  private retryAt = 0;

  constructor(
    /** The socket's URL; null when not in a room yet. */
    private url: () => string | null,
    private onMessage: (msg: SpeechServerMessage) => void,
  ) {}

  /** Opens it ahead of the first utterance, so that one doesn't wait for the handshake. */
  connect(): void {
    this.socket();
  }

  start(lang: Lang, live: boolean): void {
    this.send(JSON.stringify({ type: "start", lang, ...(live && { live }) } satisfies SpeechClientMessage));
  }

  audio(pcm: Int16Array): void {
    if (pcm.length) this.send(pcm);
  }

  end(): void {
    this.send(JSON.stringify({ type: "end" }));
  }

  cancel(): void {
    this.send(JSON.stringify({ type: "cancel" }));
  }

  close(): void {
    this.drop();
    this.retryAt = 0;
  }

  private send(data: string | Int16Array): void {
    const ws = this.socket();
    if (!ws) return;
    if (ws.readyState === WebSocket.OPEN) return ws.send(data);
    const bytes = typeof data === "string" ? data.length : data.byteLength;
    if (this.queuedBytes + bytes > MAX_QUEUED_BYTES) return;
    this.queue.push(data);
    this.queuedBytes += bytes;
  }

  private socket(): WebSocket | null {
    if (this.ws) return this.ws;
    const url = this.url();
    if (!url || Date.now() < this.retryAt) return null;
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}${url}`);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      for (const data of this.queue) ws.send(data);
      this.queue = [];
      this.queuedBytes = 0;
    };
    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return;
      try {
        this.onMessage(JSON.parse(ev.data) as SpeechServerMessage);
      } catch {
        // not ours to read
      }
    };
    // An utterance cut off by the drop is lost: the server only transcribes what it saw start and end
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.drop();
      this.retryAt = Date.now() + RETRY_MS;
    };
    this.ws = ws;
    return ws;
  }

  private drop(): void {
    const ws = this.ws;
    this.ws = null;
    this.queue = [];
    this.queuedBytes = 0;
    if (!ws) return;
    ws.onopen = ws.onmessage = ws.onclose = null;
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close();
  }
}
