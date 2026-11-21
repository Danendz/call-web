// Cuts one utterance from /ws/speech into chat lines and, with live subtitles, the text of the line being said.
// Silero VAD says where speech is: a short pause in it ends a line, and an utterance with no speech in it at all (a
// door, the room's noise) posts nothing. With live subtitles the line so far is decoded again as it grows, and each
// result replaces the last on the other side. A line alone is often a second or two, too little for SenseVoice to
// read well, so lines are only shown live: once the speaker pauses longer, the sentence they make is decoded again as
// a whole, and that's what goes to the chat.

import { TRANSCRIBE_SAMPLE_RATE } from "../shared/protocol.js";

/** Where speech is in the audio. */
export interface Vad {
  accept(samples: Float32Array): void;
  /**
   * Where speech ended since the last call, in samples since the utterance started; null when it didn't. A pause or
   * running too long ends it. The VAD says so only once the pause is long enough, by which time the next line may
   * have started.
   */
  ended(): number | null;
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
// Kept past where the VAD says speech ended: most of the pause it waited for. SenseVoice reads a line better with
// some silence after it, but the next line can start within the pause's last tenth of a second.
const KEEP_AFTER_SPEECH_S = 0.3;
// A pause this long ends a sentence, and its lines go to the chat as one
const SENTENCE_PAUSE_S = 1;
// A sentence this long goes to the chat even without a pause: SenseVoice reads up to about 30 s at once
const MAX_SENTENCE_S = 20;
// A line that's only a filler: in the chat it's noise even when it was really said, since it was heard anyway
const FILLER_RE = /^(?:嗯|啊|哦|呃|额|唔|mm+|hm+|uh+|um+|ah+|oh+|huh|mhm)+$/;

function clean(text: string): string {
  const words = text.toLowerCase().replace(/[\p{P}\p{S}\s]+/gu, "");
  // Only punctuation: SenseVoice's reading of a breath or a cough
  return !words || FILLER_RE.test(words) ? "" : text.trim();
}

// Chinese and Japanese text is written without spaces between words
const CJK_RE = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\u3000-\u303f\uff00-\uffef]/u;

function join(texts: string[]): string {
  return texts.reduce((all, t) => (!all ? t : CJK_RE.test(all.at(-1)!) || CJK_RE.test(t[0]) ? all + t : `${all} ${t}`), "");
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
  // Where `chunks` starts, in samples since the utterance started
  private start = 0;
  // `samples` when live text was last decoded
  private decodedAt = 0;
  // Shows the sentence being said, its finished lines and the line so far
  private liveId: string | null = null;
  // Counts finished lines, so a live result for one that has since finished is dropped
  private lineNo = 0;
  // The finished lines not in the chat yet: their audio, and their text as each comes back. `speechEnd` is where the
  // last one's speech ended, in samples since the utterance started.
  private sentence: { audio: Float32Array[]; samples: number; texts: string[]; speechEnd: number } | null = null;
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
    const end = this.vad.ended();
    if (end !== null) this.finishLine(end, true);
    if (!this.vad.speaking()) {
      if (this.sentence && this.start + this.samples - this.sentence.speechEnd >= SENTENCE_PAUSE_S * RATE)
        this.finishSentence();
      while (this.chunks.length > 1 && this.samples - this.chunks[0].length >= KEEP_BEFORE_SPEECH_S * RATE) {
        const dropped = this.chunks.shift()!.length;
        this.samples -= dropped;
        this.start += dropped;
      }
      this.decodedAt = 0;
    } else if (this.live) this.decodeLive();
  }

  /** The speaker paused long enough to end the utterance: whatever speech is left is a line. */
  end(): void {
    if (this.done) return;
    this.vad.flush();
    // Nothing more is coming: it all goes in this line, and the line in the sentence posted now
    if (this.vad.ended() !== null) this.finishLine(Infinity, false);
    this.finishSentence();
    this.stop();
  }

  /** Dropped: what's still unfinished isn't posted, but the lines already finished are. */
  cancel(): void {
    if (this.done) return;
    this.finishSentence();
    this.stop();
  }

  private stop(): void {
    this.done = true;
    this.chunks = [];
    if (this.liveId) this.sink.live(this.liveId, "");
    this.liveId = null;
  }

  /**
   * Ends the line where speech ended; what came after it starts the next one. The line joins the sentence, and with
   * `show` it's decoded alone too, so the live text keeps up until the sentence is done.
   */
  private finishLine(end: number, show: boolean): void {
    const audio = concat(this.chunks, this.samples);
    const cut = Math.min(this.samples, Math.max(0, end - this.start) + Math.round(KEEP_AFTER_SPEECH_S * RATE));
    const speechEnd = Math.min(end, this.start + this.samples);
    this.lineNo++;
    this.chunks = cut < this.samples ? [audio.slice(cut)] : [];
    this.samples -= cut;
    this.start += cut;
    this.decodedAt = 0;

    if (this.sentence && this.sentence.samples + cut > MAX_SENTENCE_S * RATE) this.finishSentence();
    const sentence = (this.sentence ??= { audio: [], samples: 0, texts: [], speechEnd });
    sentence.audio.push(audio.subarray(0, cut));
    sentence.samples += cut;
    sentence.speechEnd = speechEnd;
    if (!this.live || !show) return;
    const id = (this.liveId ??= this.newId());
    const text = this.decoder.decode(audio.subarray(0, cut));
    this.posted = Promise.all([text, this.posted]).then(([text]) => {
      const kept = clean(text);
      if (!kept || this.sentence !== sentence) return;
      sentence.texts.push(kept);
      this.sink.live(id, join(sentence.texts));
    });
  }

  /** Decodes the finished lines again as one, and posts that: it replaces their live text. */
  private finishSentence(): void {
    const sentence = this.sentence;
    if (!sentence) return;
    this.sentence = null;
    const liveId = this.liveId ?? undefined;
    this.liveId = null;
    const seconds = sentence.samples / RATE;
    const text = this.decoder.decode(concat(sentence.audio, sentence.samples));
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
    const sentence = this.sentence;
    const id = (this.liveId ??= this.newId());
    void this.decoder.decode(concat(this.chunks, this.samples)).then((text) => {
      // Stale: the line has finished since, or its sentence has gone to the chat
      if (lineNo !== this.lineNo || this.liveId !== id) return;
      const kept = clean(text);
      if (kept) this.sink.live(id, join([...(sentence?.texts ?? []), kept]));
    });
  }
}
