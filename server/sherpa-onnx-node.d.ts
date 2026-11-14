// The part of sherpa-onnx-node the server uses; the package ships JSDoc only, no types.

declare module "sherpa-onnx-node" {
  interface Waveform {
    samples: Float32Array;
    sampleRate: number;
  }

  export class OfflineStream {
    acceptWaveform(waveform: Waveform): void;
  }

  export interface OfflineRecognizerConfig {
    featConfig: { sampleRate: number; featureDim: number };
    modelConfig: {
      senseVoice: { model: string; language: string; useInverseTextNormalization: number };
      tokens: string;
      numThreads: number;
      debug: number;
    };
  }

  export class OfflineRecognizer {
    static createAsync(config: OfflineRecognizerConfig): Promise<OfflineRecognizer>;
    createStream(): OfflineStream;
    decodeAsync(stream: OfflineStream): Promise<{ text: string }>;
  }

  export interface VadConfig {
    sileroVad: {
      model: string;
      threshold: number;
      minSilenceDuration: number;
      minSpeechDuration: number;
      windowSize: number;
      maxSpeechDuration: number;
    };
    sampleRate: number;
    numThreads: number;
    debug: number;
  }

  export class Vad {
    constructor(config: VadConfig, bufferSizeInSeconds: number);
    acceptWaveform(samples: Float32Array): void;
    isEmpty(): boolean;
    isDetected(): boolean;
    pop(): void;
    flush(): void;
  }
}
