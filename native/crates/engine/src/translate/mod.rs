//! Live subtitles' translation on this device: Firefox's Bergamot engine with one direction's model, on its own
//! thread. Lines are translated in the order asked; if they come faster than that, the oldest waiting ones are
//! dropped, since newer live text has replaced them.

mod host;
mod model;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

pub use model::Pair;
pub(crate) use model::{dir as model_dir, is_ready as model_ready};

/// Lines waiting their turn; past this the oldest are dropped
const MAX_WAITING: usize = 6;
// Room for the engine's calls; wasmtime keeps WASM's own stack within its default 512 KB of it
const STACK: usize = 8 * 1024 * 1024;

/// What the translator reports, on its own thread.
#[derive(Debug)]
pub enum TranslatorEvent {
    /// Loaded; lines asked for before this are translated now
    Ready,
    Translated {
        text: String,
        translation: String,
    },
    /// The engine didn't load; nothing more comes
    Failed,
}

/// One direction's engine on its own thread. Dropping it stops the thread and frees its memory (a few hundred MB).
pub struct Translator {
    tx: Sender<String>,
}

impl Translator {
    /// Starts loading `pair`'s model from the app's data folder. `on_event` is called on the translator's thread.
    pub fn start(data_dir: &Path, pair: Pair, on_event: impl Fn(TranslatorEvent) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let dir = model::dir(data_dir, pair);
        let spawned = std::thread::Builder::new()
            .name("translate".into())
            .stack_size(STACK)
            .spawn(move || run(&dir, pair, &rx, &on_event));
        if let Err(e) = spawned {
            tracing::warn!("couldn't start the translator: {e}");
        }
        Self { tx }
    }

    /// Asks for `text` to be translated
    pub fn translate(&self, text: String) {
        let _ = self.tx.send(text);
    }
}

fn run(dir: &Path, pair: Pair, rx: &Receiver<String>, on_event: &dyn Fn(TranslatorEvent)) {
    let started = std::time::Instant::now();
    let mut engine = match host::Bergamot::load(dir, pair) {
        Ok(engine) => engine,
        Err(e) => {
            tracing::warn!("translation didn't load: {e:#}");
            on_event(TranslatorEvent::Failed);
            return;
        }
    };
    tracing::info!("translation {pair:?} loaded in {} ms", started.elapsed().as_millis());
    on_event(TranslatorEvent::Ready);
    let mut waiting = VecDeque::new();
    // Ends when the Translator is dropped
    while let Ok(text) = rx.recv() {
        waiting.push_back(text);
        waiting.extend(rx.try_iter());
        while waiting.len() > MAX_WAITING {
            waiting.pop_front();
        }
        while let Some(text) = waiting.pop_front() {
            match engine.translate(&text) {
                Ok(translation) => on_event(TranslatorEvent::Translated { text, translation }),
                Err(e) => {
                    // A trap can leave the engine's memory half changed, so it starts over
                    tracing::warn!("a line didn't translate: {e:#}");
                    match host::Bergamot::load(dir, pair) {
                        Ok(fresh) => engine = fresh,
                        Err(e) => {
                            tracing::warn!("translation didn't load again: {e:#}");
                            on_event(TranslatorEvent::Failed);
                            return;
                        }
                    }
                }
            }
            // Newer lines may have come in meanwhile
            waiting.extend(rx.try_iter());
            while waiting.len() > MAX_WAITING {
                waiting.pop_front();
            }
        }
    }
}
