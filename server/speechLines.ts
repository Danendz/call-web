// Cuts one utterance from /ws/speech into chat lines and, with live subtitles, the text of the line being said.
// Silero VAD says where speech is: a pause in it ends a line, so a long stretch of talk becomes a few short lines
// rather than one, and an utterance with no speech in it at all (a door, the room's noise) posts nothing. With live
// subtitles the line so far is decoded again as it grows, and each result replaces the last on the other side.

import { TRANSCRIBE_SAMPLE_RATE } from "../shared/protocol.js";

/** Where speech is in the audio. */
export interface Vad {
  accept(samples: Float32Array): void;
  /** Speech ended since the last call: a pause ended it, or it ran too long. */
  ended(): boolean;
  /** Ends the speech still going on, so the next ended() says so. */
  flush(): void;
  /** Someone is speaking right now. */
  speaking(): boolean;
}

export interface Decoder {
  /** The text in `samples`. One decode runs at a time, in the order they were asked for. */
  decode(samples: Float32Array): Promise<string>;
  /** Decodes asked for and not done yet, anyone's. */
  waiting(): number;
}

export interface LineSink {
  /** The line being said so far; empty text takes it back. */
  live(id: string, text: string): void;
  /** A finished line, which replaces the live text `liveId` when there was one. */
  line(text: string, liveId: string | undefined, info: { seconds: number; waitedMs: number }): void;
}

const RATE = TRANSCRIBE_SAMPLE_RATE;
// Live text is decoded again after this much new audio; less often once the line is long, since every decode is of
// the whole line so far and takes longer
const LIVE_EVERY_S = 0.5;
const LIVE_EVERY_LONG_S = 1;
const LONG_LINE_S = 5;
// Audio kept from before the VAD says speech started, which it says a little late: without it, the first syllable
// is cut
const KEEP_BEFORE_SPEECH_S = 0.5;
// A line that's only a filler: in the chat it's noise even when it was really said, since it was heard anyway
const FILLER_RE = /^(?:嗯|啊|哦|呃|额|唔|mm+|hm+|uh+|um+|ah+|oh+|huh|mhm)+$/;

function clean(text: string): string {
  const words = text.toLowerCase().replace(/[\p{P}\p{S}\s]+/gu, "");
  return FILLER_RE.test(words) ? "" : text.trim();
}

function concat(chunks: Float32Array[], length: number): Float32Array {
  const out = new Float32Array(length);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

export class Utterance {
  // The audio since the last line ended
  private chunks: Float32Array[] = [];
  private samples = 0;
  // `samples` when live text was last decoded
  private decodedAt = 0;
  private liveId: string | null = null;
  // Counts finished lines, so a live result for one that has since finished is dropped
  private lineNo = 0;
  private done = false;
  private posted: Promise<void> = Promise.resolve();

  constructor(
    private vad: Vad,
    private decoder: Decoder,
    private live: boolean,
    private sink: LineSink,
    private newId: () => string,
  ) {}

  audio(samples: Float32Array): void {
    if (this.done) return;
    this.vad.accept(samples);
    this.chunks.push(samples);
    this.samples += samples.length;
    if (this.vad.ended()) this.finishLine();
    if (!this.vad.speaking()) {
      while (this.chunks.length > 1 && this.samples - this.chunks[0].length >= KEEP_BEFORE_SPEECH_S * RATE) {
        this.samples -= this.chunks.shift()!.length;
      }
      this.decodedAt = 0;
    } else if (this.live) this.decodeLive();
  }

  /** The speaker paused long enough to end the utterance: whatever speech is left is a line. */
  end(): void {
    if (this.done) return;
    this.vad.flush();
    if (this.vad.ended()) this.finishLine();
    this.stop();
  }

  /** Dropped: what's still unfinished isn't posted. */
  cancel(): void {
    if (!this.done) this.stop();
  }

  private stop(): void {
    this.done = true;
    this.chunks = [];
    if (this.liveId) this.sink.live(this.liveId, "");
    this.liveId = null;
  }

  private finishLine(): void {
    const liveId = this.liveId ?? undefined;
    const seconds = this.samples / RATE;
    const text = this.decoder.decode(concat(this.chunks, this.samples));
    this.liveId = null;
    this.lineNo++;
    this.chunks = [];
    this.samples = this.decodedAt = 0;
    const cutAt = Date.now();
    this.posted = Promise.all([text, this.posted]).then(([text]) => {
      const kept = clean(text);
      if (kept) this.sink.line(kept, liveId, { seconds, waitedMs: Date.now() - cutAt });
      else if (liveId) this.sink.live(liveId, "");
    });
  }

  private decodeLive(): void {
    const every = this.samples > LONG_LINE_S * RATE ? LIVE_EVERY_LONG_S : LIVE_EVERY_S;
    // Behind: a decode now would only wait behind the others, and be stale when it came back
    if (this.samples - this.decodedAt < every * RATE || this.decoder.waiting() > 0) return;
    this.decodedAt = this.samples;
    const lineNo = this.lineNo;
    const id = (this.liveId ??= this.newId());
    void this.decoder.decode(concat(this.chunks, this.samples)).then((text) => {
      if (lineNo !== this.lineNo || this.done) return;
      const kept = clean(text);
      if (kept) this.sink.live(id, kept);
    });
  }
}
