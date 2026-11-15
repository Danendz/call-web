//! The app's Bergamot against the engine's own output in Node (Firefox's glue), line for line, and how fast it is. Needs
//! the models downloaded: `BERGAMOT_DATA_DIR=<the app's data folder> cargo test --release -p engine --test translate --
//! --ignored --nocapture`; the app downloads them there from Settings. On Android, build it with `cargo ndk -t
//! arm64-v8a -P 29 test --release -p engine --test translate --no-run`, push the binary and the models with `adb` and
//! run it there.
//!
//! The app runs the engine built natively, whose matrix multiplications (NEON, or ruy and intgemm on x86) round
//! differently from WASM's and pick other words on some lines, so it only has to translate every line and match most
//! of them; a wrong vocabulary or broken segmentation matches none. With `--features wasm-translate` it checks the
//! WebAssembly build through wasmtime (`host.rs`) instead, which must match every line: it caught an embind bug once (a
//! shared_ptr's method called on the pointer instead of the object), which still translated, just badly.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use engine::{Pair, Translator, TranslatorEvent};

#[test]
#[ignore = "needs the translation models downloaded"]
fn matches_firefoxs_engine() {
    let dir = std::path::PathBuf::from(std::env::var("BERGAMOT_DATA_DIR").expect("BERGAMOT_DATA_DIR"));
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("bergamot_reference.json")).expect("reference");
    for (pair, key) in [(Pair::EnZh, "en-zh"), (Pair::ZhEn, "zh-en")] {
        let (tx, rx) = mpsc::channel();
        let translator = Translator::start(&dir, pair, move |e| {
            let _ = tx.send(e);
        });
        let lines = reference[key].as_array().expect("lines");
        let mut different = Vec::new();
        let started = Instant::now();
        let mut loaded = Duration::ZERO;
        for line in lines {
            let (src, want) = (line[0].as_str().expect("src"), line[1].as_str().expect("out"));
            translator.translate(src.to_string());
            let got = loop {
                match rx.recv_timeout(Duration::from_secs(60)).expect("no answer") {
                    TranslatorEvent::Ready => {
                        loaded = started.elapsed();
                        continue;
                    }
                    TranslatorEvent::Translated { translation, .. } => break translation,
                    TranslatorEvent::Failed => panic!("{key} didn't load from {}", dir.display()),
                }
            };
            assert!(!got.trim().is_empty(), "{key}: {src:?} translated to nothing");
            if got != want {
                different.push(format!("{src}\n  got:  {got}\n  want: {want}"));
            }
        }
        let translating = started.elapsed() - loaded;
        println!(
            "{key}: loaded in {} ms, {} lines in {} ms ({:.1} ms a line), {} of them different",
            loaded.as_millis(),
            lines.len(),
            translating.as_millis(),
            translating.as_secs_f64() * 1000.0 / lines.len() as f64,
            different.len()
        );
        let allowed = if cfg!(feature = "wasm-translate") { 0 } else { lines.len() / 2 };
        assert!(
            different.len() <= allowed,
            "{key}: {} of {} differ:\n{}",
            different.len(),
            lines.len(),
            different.join("\n")
        );
    }
}
