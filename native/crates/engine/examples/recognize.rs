//! Times an on-device speech model on a 16 kHz mono WAV, for checking a phone or a slow machine:
//!
//!   cargo run --release -p engine --example recognize -- <model folder> <file.wav> [zh|en] [threads]
//!
//! A live model's folder is fed 40 ms at a time, as the mic does, and prints its text as it changes.
//!
//! On a phone, push it with sherpa-onnx's two .so files (`target/sherpa-onnx-prebuilt/jniLibs/arm64-v8a/`) and run it
//! with `LD_LIBRARY_PATH=.`.

// A dev tool: bad arguments just stop it
#![allow(clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use engine::LiveModel;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, Wave};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(wav)) = (args.next(), args.next()) else {
        eprintln!("usage: recognize <model folder> <file.wav> [zh|en] [threads]");
        std::process::exit(2);
    };
    if Path::new(&dir).join("encoder.int8.onnx").exists() {
        live(&dir, &wav);
        return;
    }
    let lang = args.next().unwrap_or_else(|| "auto".into());
    let threads = args.next().and_then(|t| t.parse().ok()).unwrap_or(2);
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(format!("{dir}/model.int8.onnx")),
        language: Some(lang),
        use_itn: true,
    };
    config.model_config.tokens = Some(format!("{dir}/tokens.txt"));
    config.model_config.num_threads = threads;
    let started = Instant::now();
    let recognizer = OfflineRecognizer::create(&config).unwrap_or_else(|| panic!("no model in {dir}"));
    println!("loaded in {:?}", started.elapsed());
    let wave = Wave::read(&wav).unwrap_or_else(|| panic!("can't read {wav}"));
    let seconds = wave.num_samples() as f32 / wave.sample_rate() as f32;
    // The first run warms up; the others are what an utterance costs
    for _ in 0..3 {
        let started = Instant::now();
        let stream = recognizer.create_stream();
        stream.accept_waveform(wave.sample_rate(), wave.samples());
        recognizer.decode(&stream);
        let text = stream.get_result().map(|r| r.text).unwrap_or_default();
        println!("{seconds:.1}s of audio in {:?}: {text}", started.elapsed());
    }
}

fn live(dir: &str, wav: &str) {
    let started = Instant::now();
    let model = Arc::new(LiveModel::load(Path::new(dir)).unwrap_or_else(|e| panic!("{e}")));
    println!("loaded in {:?}", started.elapsed());
    let wave = Wave::read(wav).unwrap_or_else(|| panic!("can't read {wav}"));
    let mut utterance = model.start();
    let started = Instant::now();
    for (i, chunk) in wave.samples().chunks(640).enumerate() {
        if let Some(text) = utterance.push(chunk) {
            println!("{:.2}s of audio, {:?} in: {text}", (i + 1) as f32 * 0.04, started.elapsed());
        }
    }
    let finishing = Instant::now();
    let line = utterance.finish().unwrap_or_default();
    println!("finished in {:?}: {line}", finishing.elapsed());
}
