// Turns what you say into chat messages: the browser's own recognition where it works, otherwise the
// server's. Once the browser's fails it stays on the server for the rest of the page's life. Live subtitles
// always use the server's, the only one that sends the line while it's being said

import type { Lang, SpeechServerMessage } from "../../../../shared/protocol";
import { BrowserRecognizer } from "./browserRecognizer";
import { RECOGNITION_LANG } from "./lang";
import { SpeechSocket } from "./speechSocket";
import { UtteranceRecorder } from "./utteranceRecorder";

/** Something the person should know about transcription; the UI words it in their language. */
export type SpeechNotice = { kind: "unavailable" };

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

// ?stt=server skips the browser's recognition, to try the server path in a browser that has its own
let serverOnly =
  new URLSearchParams(location.search).get("stt") === "server" ||
  !BrowserRecognizer.supported() ||
  SILENT_RECOGNITION_UA.test(navigator.userAgent);

export class Transcriber {
  private engine: { stop(): void } | null = null;
  // What it's running for (track, language, live), so sync() only restarts when that changes
  private running: string | null = null;
  private unavailable = false;
  private speech: SpeechSocket;

  constructor(private hooks: TranscriberHooks) {
    this.speech = new SpeechSocket(hooks.speechUrl, (msg) => this.onServer(msg));
  }

  /**
   * Runs on `track` in `lang`, or stops when `track` is null; `live` sends the line being said to the other person
   * as it grows. Cheap to call on every state change.
   */
  sync(track: MediaStreamTrack | null, lang: Lang, live: boolean): void {
    const key = track && `${track.id}/${lang}/${live}`;
    if (key === this.running) return;
    this.running = key;
    if (track && !this.unavailable) return this.start(track, lang, live);
    this.stop();
    this.speech.close();
  }

  private start(track: MediaStreamTrack, lang: Lang, live: boolean): void {
    this.stop();
    if (serverOnly || live) {
      this.speech.connect();
      this.engine = new UtteranceRecorder(track, {
        start: () => this.speech.start(lang, live),
        audio: (pcm) => this.speech.audio(pcm),
        end: (speech) => (speech ? this.speech.end() : this.speech.cancel()),
      });
      return;
    }

    // While the browser recognizes, the server path's recorder only listens: what you said while the
    // browser stayed silent is kept, and sent to the server if it turns out the browser isn't working
    let unheard: Int16Array[][] = [];
    let current: Int16Array[] = [];
    let running = true;
    const useServer = (reason: string) => {
      if (!running) return;
      console.info(`browser speech recognition failed (${reason}), using the server`);
      serverOnly = true;
      const missed = unheard;
      this.start(track, lang, false);
      for (const utterance of missed) {
        this.speech.start(lang, false);
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
    const watchdog = new UtteranceRecorder(track, {
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
        running = false;
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
    }
  }
}
