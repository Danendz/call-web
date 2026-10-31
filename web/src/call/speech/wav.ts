import { TRANSCRIBE_SAMPLE_RATE } from "../../../../shared/protocol";

/** Mono samples at `rate` as a 16-bit WAV at the rate the server transcribes. */
export function encodeWav(samples: Float32Array, rate: number): Blob {
  const pcm = downsample(samples, rate, TRANSCRIBE_SAMPLE_RATE);
  const view = new DataView(new ArrayBuffer(44 + pcm.length * 2));
  const text = (at: number, s: string) => [...s].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  view.setUint32(4, 36 + pcm.length * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, TRANSCRIBE_SAMPLE_RATE, true);
  view.setUint32(28, TRANSCRIBE_SAMPLE_RATE * 2, true); // bytes per second
  view.setUint16(32, 2, true); // bytes per frame
  view.setUint16(34, 16, true); // bits per sample
  text(36, "data");
  view.setUint32(40, pcm.length * 2, true);
  pcm.forEach((s, i) => view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, s)) * 0x7fff, true));
  return new Blob([view.buffer], { type: "audio/wav" });
}

// Averages each output sample over the input samples it covers, which also filters what's above the new rate
function downsample(samples: Float32Array, from: number, to: number): Float32Array {
  if (from <= to) return samples;
  const ratio = from / to;
  const out = new Float32Array(Math.floor(samples.length / ratio));
  for (let i = 0; i < out.length; i++) {
    const start = Math.floor(i * ratio);
    const end = Math.min(samples.length, Math.floor((i + 1) * ratio));
    let sum = 0;
    for (let j = start; j < end; j++) sum += samples[j];
    out[i] = sum / (end - start);
  }
  return out;
}
