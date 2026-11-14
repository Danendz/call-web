// Cuts the mic track into utterances for the server's recognition, handing each one out while it's being
// said. Loudness against the room's noise floor says when someone is talking; a pause ends the utterance.
// It listens to the call's own track, so echo cancellation keeps the other person's voice from the
// speakers out of it

import { MAX_UTTERANCE_S } from "../../../../shared/protocol";
import { Downsampler } from "./pcm";

const CHUNK = 2048;
// Hands the mic's samples to the page in chunks; inline so it ships in the bundle, not from another host
const WORKLET = `
class Capture extends AudioWorkletProcessor {
  buf = new Float32Array(${CHUNK});
  n = 0;
  process(inputs) {
    const ch = inputs[0][0];
    if (ch) for (let i = 0; i < ch.length; i++) {
      this.buf[this.n++] = ch[i];
      if (this.n === ${CHUNK}) {
        this.port.postMessage(this.buf, [this.buf.buffer]);
        this.buf = new Float32Array(${CHUNK});
        this.n = 0;
      }
    }
    return true;
  }
}
registerProcessor("capture", Capture);
`;

// Louder than the noise floor by this much is speech, and never quieter than MIN_SPEECH_DB
const ABOVE_FLOOR_DB = 12;
const MIN_SPEECH_DB = -50;
// Loud chunks in a row that start an utterance (~90ms), so a click doesn't
const START_CHUNKS = 2;
// Kept from before the start, so the first syllable isn't cut
const PREROLL_MS = 300;
const END_SILENCE_MS = 800;
// Shorter than this of actual speech is a cough or a bump, not worth a request
const MIN_VOICED_MS = 300;

/** Where an utterance goes as it's said: 16-bit PCM at the server's rate, then whether it was speech. */
export interface UtteranceSink {
  start(): void;
  audio(pcm: Int16Array): void;
  end(speech: boolean): void;
}

export class UtteranceRecorder {
  private ctx: AudioContext;
  private chunkMs: number;
  private floorDb = -60;
  private loudRun = 0;
  private preroll: Float32Array[] = [];
  private utterance: Downsampler | null = null;
  private utteranceMs = 0;
  private voicedMs = 0;
  private silentMs = 0;
  private stopped = false;

  constructor(
    track: MediaStreamTrack,
    private sink: UtteranceSink,
  ) {
    this.ctx = new AudioContext();
    this.chunkMs = (CHUNK / this.ctx.sampleRate) * 1000;
    void this.connect(track);
  }

  stop(): void {
    this.stopped = true;
    void this.ctx.close();
    // Cut off mid-sentence (muted, language changed): what was said so far is dropped
    if (this.utterance) this.sink.end(false);
    this.utterance = null;
  }

  private async connect(track: MediaStreamTrack): Promise<void> {
    const url = URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" }));
    try {
      await this.ctx.audioWorklet.addModule(url);
    } finally {
      URL.revokeObjectURL(url);
    }
    if (this.stopped) return;
    const node = new AudioWorkletNode(this.ctx, "capture");
    node.port.onmessage = (e: MessageEvent<Float32Array>) => this.onChunk(e.data);
    // Some browsers only run a node that leads to the speakers; silenced, so nothing is heard
    const mute = this.ctx.createGain();
    mute.gain.value = 0;
    this.ctx.createMediaStreamSource(new MediaStream([track])).connect(node).connect(mute).connect(this.ctx.destination);
    await this.ctx.resume();
  }

  private onChunk(samples: Float32Array): void {
    let sum = 0;
    for (const s of samples) sum += s * s;
    const db = 10 * Math.log10(sum / samples.length + 1e-12);
    const loud = db > Math.max(this.floorDb + ABOVE_FLOOR_DB, MIN_SPEECH_DB);

    if (!this.utterance) {
      // Drops to quiet at once, creeps up to steady noise slowly
      if (!loud) this.floorDb = db < this.floorDb ? db : this.floorDb + (db - this.floorDb) * 0.05;
      this.preroll.push(samples);
      if (this.preroll.length * this.chunkMs > PREROLL_MS) this.preroll.shift();
      this.loudRun = loud ? this.loudRun + 1 : 0;
      if (this.loudRun >= START_CHUNKS) {
        this.utterance = new Downsampler(this.ctx.sampleRate);
        this.utteranceMs = this.preroll.length * this.chunkMs;
        this.voicedMs = this.loudRun * this.chunkMs;
        this.silentMs = 0;
        this.sink.start();
        for (const chunk of this.preroll) this.sink.audio(this.utterance.push(chunk));
        this.preroll = [];
      }
      return;
    }

    this.sink.audio(this.utterance.push(samples));
    this.utteranceMs += this.chunkMs;
    if (loud) {
      this.voicedMs += this.chunkMs;
      this.silentMs = 0;
    } else this.silentMs += this.chunkMs;
    if (this.silentMs >= END_SILENCE_MS || this.utteranceMs >= MAX_UTTERANCE_S * 1000 - 500) this.finish();
  }

  private finish(): void {
    this.utterance = null;
    this.loudRun = 0;
    this.sink.end(this.voicedMs >= MIN_VOICED_MS);
  }
}
