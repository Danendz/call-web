import { TRANSCRIBE_SAMPLE_RATE } from "../../../../shared/protocol";

/**
 * Turns the mic's samples into the 16-bit PCM the server transcribes, a chunk at a time. Each output
 * sample averages the input samples it covers, which also filters what's above the new rate; what a
 * chunk ends partway through is kept for the next one.
 */
export class Downsampler {
  private ratio: number;
  private carry = new Float32Array(0);
  private at = 0; // where the next output sample starts in carry, in input samples

  constructor(rate: number) {
    this.ratio = Math.max(1, rate / TRANSCRIBE_SAMPLE_RATE);
  }

  push(samples: Float32Array): Int16Array {
    const input = new Float32Array(this.carry.length + samples.length);
    input.set(this.carry);
    input.set(samples, this.carry.length);
    const out = new Int16Array(Math.max(0, Math.floor((input.length - this.at) / this.ratio)));
    let n = 0;
    for (; n < out.length; n++) {
      const start = Math.floor(this.at);
      const end = Math.max(start + 1, Math.floor(this.at + this.ratio));
      if (end > input.length) break;
      let sum = 0;
      for (let j = start; j < end; j++) sum += input[j];
      out[n] = Math.max(-1, Math.min(1, sum / (end - start))) * 0x7fff;
      this.at += this.ratio;
    }
    const used = Math.floor(this.at);
    this.carry = input.slice(used);
    this.at -= used;
    return out.subarray(0, n);
  }
}
