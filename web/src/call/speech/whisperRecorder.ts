// Cuts the mic track into utterances for server transcription. Loudness against the room's noise
// floor says when someone is talking; a pause ends the utterance. It listens to the call's own track,
// so echo cancellation keeps the other person's voice from the speakers out of it

import { MAX_UTTERANCE_S } from "../../../../shared/protocol";
import { encodeWav } from "./wav";

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

export class WhisperRecorder {
  private ctx: AudioContext;
  private chunkMs: number;
  private floorDb = -60;
  private loudRun = 0;
  private preroll: Float32Array[] = [];
  private utterance: Float32Array[] | null = null;
  private voicedMs = 0;
  private silentMs = 0;
  private stopped = false;

  constructor(
    track: MediaStreamTrack,
    private onUtterance: (wav: Blob) => void,
  ) {
    this.ctx = new AudioContext();
    this.chunkMs = (CHUNK / this.ctx.sampleRate) * 1000;
    void this.connect(track);
  }

  stop(): void {
    this.stopped = true;
    void this.ctx.close();
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
        this.utterance = this.preroll;
        this.preroll = [];
        this.voicedMs = this.loudRun * this.chunkMs;
        this.silentMs = 0;
      }
      return;
    }

    this.utterance.push(samples);
    if (loud) {
      this.voicedMs += this.chunkMs;
      this.silentMs = 0;
    } else this.silentMs += this.chunkMs;
    if (this.silentMs >= END_SILENCE_MS || this.utterance.length * this.chunkMs >= MAX_UTTERANCE_S * 1000 - 500) this.finish();
  }

  private finish(): void {
    const chunks = this.utterance!;
    this.utterance = null;
    this.loudRun = 0;
    if (this.voicedMs < MIN_VOICED_MS) return;
    const all = new Float32Array(chunks.length * CHUNK);
    chunks.forEach((c, i) => all.set(c, i * CHUNK));
    this.onUtterance(encodeWav(all, this.ctx.sampleRate));
  }
}
