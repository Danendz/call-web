//! The native Bergamot host against the engine's own output in Node (Firefox's glue), line for line. It caught an
//! embind bug once (a shared_ptr's method called on the pointer instead of the object), which still translated, just
//! badly. Needs the models downloaded: `BERGAMOT_DATA_DIR=<the app's data folder> cargo test -p engine --test
//! translate -- --ignored`; the app downloads them
//! there from Settings.
//!
//! With `--features native-translate` it checks the engine built natively (iOS's) instead. Its matrix multiplications
//! are NEON rather than WASM's, which rounds differently and picks other words on some lines, so it only has to
//! translate every line and match most of them; a wrong vocabulary or broken segmentation matches none.

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
        for line in lines {
            let (src, want) = (line[0].as_str().expect("src"), line[1].as_str().expect("out"));
            translator.translate(src.to_string());
            let got = loop {
                match rx.recv_timeout(Duration::from_secs(60)).expect("no answer") {
                    TranslatorEvent::Ready => continue,
                    TranslatorEvent::Translated { translation, .. } => break translation,
                    TranslatorEvent::Failed => panic!("{key} didn't load from {}", dir.display()),
                }
            };
            assert!(!got.trim().is_empty(), "{key}: {src:?} translated to nothing");
            if got != want {
                different.push(format!("{src}\n  got:  {got}\n  want: {want}"));
            }
        }
        println!(
            "{key}: {} lines in {} ms with loading, {} of them different",
            lines.len(),
            started.elapsed().as_millis(),
            different.len()
        );
        let allowed = if cfg!(any(feature = "native-translate", target_os = "ios")) { lines.len() / 2 } else { 0 };
        assert!(
            different.len() <= allowed,
            "{key}: {} of {} differ:\n{}",
            different.len(),
            lines.len(),
            different.join("\n")
        );
    }
}
