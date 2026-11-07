// Turns what you say into chat messages: the browser's own recognition where it works, otherwise the
// server's. Once the browser's fails it stays on the server for the rest of the page's life

import type { Lang, SpeechServerMessage } from "../../../../shared/protocol";
import { BrowserRecognizer } from "./browserRecognizer";
import { RECOGNITION_LANG } from "./lang";
import { SpeechSocket } from "./speechSocket";
import { WhisperRecorder } from "./whisperRecorder";

/** Something the person should know about transcription; the UI words it in their language. */
export type SpeechNotice = { kind: "unavailable" } | { kind: "paused"; until: number };

export interface TranscriberHooks {
  /** A line the browser recognized, to post as spoken chat. */
  sendText(text: string): void;
  /** The server path's socket URL; null when not in a room yet. */
  speechUrl(): string | null;
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
  private pausedUntil = 0;
  private resumeTimer?: number;
  private speech: SpeechSocket;

  constructor(private hooks: TranscriberHooks) {
    this.speech = new SpeechSocket(hooks.speechUrl, (msg) => this.onServer(msg));
  }

  /** Runs on `track` in `lang`, or stops when `track` is null. Cheap to call on every state change. */
  sync(track: MediaStreamTrack | null, lang: Lang): void {
    const key = track && `${track.id}/${lang}`;
    if (key === this.running) return;
    this.running = key;
    if (track && !this.unavailable) return this.start(track, lang);
    this.stop();
    this.speech.close();
  }

  private start(track: MediaStreamTrack, lang: Lang): void {
    this.stop();
    if (serverOnly) {
      this.speech.connect();
      // Paused while out of quota: nothing goes out until it's back
      let sending = false;
      this.engine = new WhisperRecorder(track, {
        start: () => {
          sending = Date.now() >= this.pausedUntil;
          if (sending) this.speech.start(lang);
        },
        audio: (pcm) => {
          if (sending) this.speech.audio(pcm);
        },
        end: (speech) => {
          if (!sending) return;
          if (speech) this.speech.end();
          else this.speech.cancel();
        },
      });
      return;
    }

    // While the browser recognizes, the server path's recorder only listens: what you said while the
    // browser stayed silent is kept, and sent to the server if it turns out the browser isn't working
    let unheard: Int16Array[][] = [];
    let current: Int16Array[] = [];
    let live = true;
    const useServer = (reason: string) => {
      if (!live) return;
      console.info(`browser speech recognition failed (${reason}), using the server`);
      serverOnly = true;
      const missed = unheard;
      this.start(track, lang);
      for (const utterance of missed) {
        this.speech.start(lang);
        utterance.forEach((pcm) => this.speech.audio(pcm));
        this.speech.end();
      }
    };
    const browser = new BrowserRecognizer(
      RECOGNITION_LANG[lang],
      (text) => {
        unheard = [];
        this.hooks.sendText(text);
      },
      useServer,
    );
    const watchdog = new WhisperRecorder(track, {
      start: () => (current = []),
      audio: (pcm) => current.push(pcm),
      end: (speech) => {
        if (!speech) return;
        unheard.push(current);
        if (unheard.length >= SILENT_UTTERANCES) useServer("no results");
      },
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

  private onServer(msg: SpeechServerMessage): void {
    if (msg.type === "unavailable") {
      this.unavailable = true;
      this.stop();
      this.speech.close();
      this.hooks.onNotice({ kind: "unavailable" });
    } else if (msg.type === "paused" && typeof msg.until === "number") {
      this.pausedUntil = msg.until;
      this.hooks.onNotice({ kind: "paused", until: msg.until });
      clearTimeout(this.resumeTimer);
      this.resumeTimer = window.setTimeout(() => this.hooks.onNotice(null), msg.until - Date.now());
    }
  }
}
