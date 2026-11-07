// Turns what you say into chat messages: the browser's own recognition where it works, otherwise the
// server's. Once the browser's fails it stays on the server for the rest of the page's life

import type { Lang, TranscribeReply } from "../../../../shared/protocol";
import { BrowserRecognizer } from "./browserRecognizer";
import { RECOGNITION_LANG } from "./lang";
import { WhisperRecorder } from "./whisperRecorder";

/** Something the person should know about transcription; the UI words it in their language. */
export type SpeechNotice = { kind: "unavailable" } | { kind: "paused"; until: number };

export interface TranscriberHooks {
  /** A line the browser recognized, to post as spoken chat. */
  sendText(text: string): void;
  /** Where the server path uploads to; null when not in a room yet. */
  uploadUrl(lang: Lang): string | null;
  /** Something the person should know about transcription, or null to clear it. */
  onNotice(notice: SpeechNotice | null): void;
}

// Chinese browsers that have SpeechRecognition but get nothing back from it, without any error
const SILENT_RECOGNITION_UA = /Quark|UCBrowser|MicroMessenger|MQQBrowser|baiduboxapp|HuaweiBrowser|MiuiBrowser/i;
// Utterances the server path heard with nothing from the browser's recognition: that many in a row and
// the browser's isn't working, even though it never said so
const SILENT_UTTERANCES = 3;

// ?stt=whisper skips the browser's recognition, to try the server path in a browser that has its own
let serverOnly =
  new URLSearchParams(location.search).get("stt") === "whisper" ||
  !BrowserRecognizer.supported() ||
  SILENT_RECOGNITION_UA.test(navigator.userAgent);

export class Transcriber {
  private engine: { stop(): void } | null = null;
  // What it's running for (track and language), so sync() only restarts when that changes
  private running: string | null = null;
  private unavailable = false;
  private uploads = Promise.resolve();
  private pausedUntil = 0;
  private resumeTimer?: number;

  constructor(private hooks: TranscriberHooks) {}

  /** Runs on `track` in `lang`, or stops when `track` is null. Cheap to call on every state change. */
  sync(track: MediaStreamTrack | null, lang: Lang): void {
    const key = track && `${track.id}/${lang}`;
    if (key === this.running) return;
    this.running = key;
    if (track && !this.unavailable) this.start(track, lang);
    else this.stop();
  }

  private start(track: MediaStreamTrack, lang: Lang): void {
    this.stop();
    if (serverOnly) {
      this.engine = new WhisperRecorder(track, (wav) => this.upload(wav, lang));
      return;
    }

    // While the browser recognizes, the server path's recorder only listens: what you said while the
    // browser stayed silent is kept, and sent to the server if it turns out the browser isn't working
    let unheard: Blob[] = [];
    let live = true;
    const useServer = (reason: string) => {
      if (!live) return;
      console.info(`browser speech recognition failed (${reason}), using the server`);
      serverOnly = true;
      const missed = unheard;
      this.start(track, lang);
      missed.forEach((wav) => this.upload(wav, lang));
    };
    const browser = new BrowserRecognizer(
      RECOGNITION_LANG[lang],
      (text) => {
        unheard = [];
        this.hooks.sendText(text);
      },
      useServer,
    );
    const watchdog = new WhisperRecorder(track, (wav) => {
      unheard.push(wav);
      if (unheard.length >= SILENT_UTTERANCES) useServer("no results");
    });
    this.engine = {
      stop() {
        live = false;
        browser.stop();
        watchdog.stop();
      },
    };
  }

  private stop(): void {
    this.engine?.stop();
    this.engine = null;
  }

  // One at a time, so lines reach the chat in the order they were said
  private upload(wav: Blob, lang: Lang): void {
    this.uploads = this.uploads.then(async () => {
      const url = this.hooks.uploadUrl(lang);
      if (!url || Date.now() < this.pausedUntil) return;
      try {
        const res = await fetch(url, { method: "POST", body: wav, headers: { "content-type": "audio/wav" } });
        this.onReply((await res.json()) as TranscribeReply);
      } catch {
        // offline or the server restarting: this line is lost, the next one tries again
      }
    });
  }

  private onReply(reply: TranscribeReply): void {
    if ("unavailable" in reply) {
      this.unavailable = true;
      this.stop();
      this.hooks.onNotice({ kind: "unavailable" });
    } else if ("paused" in reply) {
      this.pausedUntil = reply.until;
      this.hooks.onNotice({ kind: "paused", until: reply.until });
      clearTimeout(this.resumeTimer);
      this.resumeTimer = window.setTimeout(() => this.hooks.onNotice(null), reply.until - Date.now());
    }
  }
}
