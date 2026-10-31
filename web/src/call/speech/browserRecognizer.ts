// The browser's own speech recognition, for browsers that have it and can reach its service. Chrome
// sends the audio to Google, which is blocked in China; that shows up as a "network" error, and the
// caller falls back to server transcription

interface RecognitionResult {
  isFinal: boolean;
  0: { transcript: string };
}

interface Recognition {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onresult: ((e: { resultIndex: number; results: ArrayLike<RecognitionResult> }) => void) | null;
  onerror: ((e: { error: string }) => void) | null;
  onend: (() => void) | null;
  start(): void;
  abort(): void;
}

type RecognitionCtor = new () => Recognition;

const Ctor: RecognitionCtor | undefined =
  (window as unknown as { SpeechRecognition?: RecognitionCtor }).SpeechRecognition ??
  (window as unknown as { webkitSpeechRecognition?: RecognitionCtor }).webkitSpeechRecognition;

// Errors that won't go away by starting again; "no-speech" and "aborted" just restart
const FATAL = new Set(["network", "service-not-allowed", "not-allowed", "language-not-supported", "audio-capture"]);
// Some browsers end straight away when they can't recognize at all; that many quick ends in a row counts as failing
const QUICK_END_MS = 1000;
const MAX_QUICK_ENDS = 5;

export class BrowserRecognizer {
  static supported = (): boolean => Ctor !== undefined;

  private rec: Recognition | null;
  private startedAt = 0;
  private quickEnds = 0;

  constructor(
    lang: string,
    private onText: (text: string) => void,
    private onFail: (reason: string) => void,
  ) {
    const rec = new Ctor!();
    rec.lang = lang;
    rec.continuous = true;
    rec.interimResults = false;
    rec.onresult = (e) => {
      this.quickEnds = 0;
      for (let i = e.resultIndex; i < e.results.length; i++) {
        const text = e.results[i].isFinal ? e.results[i][0].transcript.trim() : "";
        if (text) this.onText(text);
      }
    };
    rec.onerror = (e) => {
      if (FATAL.has(e.error)) this.fail(e.error);
    };
    rec.onend = () => {
      if (!this.rec) return;
      // Recognition stops by itself after a pause or a minute; keep it going while enabled
      this.quickEnds = Date.now() - this.startedAt < QUICK_END_MS ? this.quickEnds + 1 : 0;
      if (this.quickEnds >= MAX_QUICK_ENDS) this.fail("keeps ending");
      else this.begin();
    };
    this.rec = rec;
    this.begin();
  }

  stop(): void {
    const rec = this.rec;
    this.rec = null;
    rec?.abort();
  }

  private begin(): void {
    this.startedAt = Date.now();
    try {
      this.rec?.start();
    } catch (err) {
      this.fail((err as Error).message);
    }
  }

  private fail(reason: string): void {
    if (!this.rec) return;
    this.stop();
    // Later, so a failure while constructing reaches a caller that holds this recognizer by then
    queueMicrotask(() => this.onFail(reason));
  }
}
