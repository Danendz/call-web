/**
 * The AudioWorklet that runs DeepFilterNet3 on the mic. The model takes 480-sample frames (10 ms at 48 kHz) and the
 * audio graph hands over 128 at a time, so input collects until there's a frame, and output starts a frame behind so
 * it never runs dry.
 */

// The worklet scope's globals, missing from TypeScript's DOM types
declare class AudioWorkletProcessor {
  constructor(options?: { processorOptions?: unknown });
}
declare function registerProcessor(name: string, processor: typeof AudioWorkletProcessor): void;

interface Exports {
  memory: WebAssembly.Memory;
  denoise_init(): number;
  denoise_buffer(): number;
  denoise_process(): void;
}

const FRAME = 480;

class DenoiseProcessor extends AudioWorkletProcessor {
  private readonly wasm: Exports | null;
  private readonly input = new Float32Array(FRAME);
  private inputLength = 0;
  private output = new Float32Array(FRAME * 4);
  // Starts a frame behind: with 128 in and out per call, a frame is always ready before the output runs out
  private outputLength = FRAME;

  constructor(options: { processorOptions: { module: WebAssembly.Module } }) {
    super();
    const instance = new WebAssembly.Instance(options.processorOptions.module, {});
    const wasm = instance.exports as unknown as Exports;
    this.wasm = wasm.denoise_init() ? wasm : null;
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const mic = inputs[0]?.[0];
    const out = outputs[0]?.[0];
    if (!out) return true;
    if (!mic || !this.wasm) {
      // No mic connected yet, or no model: pass through what there is
      if (mic) out.set(mic);
      return true;
    }
    for (let i = 0; i < mic.length; ) {
      const n = Math.min(mic.length - i, FRAME - this.inputLength);
      this.input.set(mic.subarray(i, i + n), this.inputLength);
      this.inputLength += n;
      i += n;
      if (this.inputLength === FRAME) {
        // A view made now: the memory may have grown since the last frame
        const buffer = new Float32Array(this.wasm.memory.buffer, this.wasm.denoise_buffer(), FRAME);
        buffer.set(this.input);
        this.wasm.denoise_process();
        this.output.set(buffer, this.outputLength);
        this.outputLength += FRAME;
        this.inputLength = 0;
      }
    }
    out.set(this.output.subarray(0, out.length));
    this.output.copyWithin(0, out.length, this.outputLength);
    this.outputLength -= out.length;
    return true;
  }
}

registerProcessor("denoise", DenoiseProcessor);
