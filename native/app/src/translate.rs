//! Live subtitles' translation on this device, like the web's useQuickTranslations: the engine's [`Translator`] for
//! the direction your language needs, kept only while subtitles can show (it holds a few hundred MB), and what it has
//! translated so far. Its answers come back on the UI thread through the bridge.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use engine::{Pair, Translator, TranslatorEvent};

// Translations kept, so a line shown again isn't translated again
const MAX_KEPT: usize = 200;
// As many as the engine's Translator keeps waiting
const MAX_WAITING: usize = 6;

struct Running {
    pair: Pair,
    translator: Translator,
    ready: bool,
    done: HashMap<String, String>,
    /// Oldest first, for dropping past MAX_KEPT
    order: VecDeque<String>,
    asked: HashSet<String>,
}

#[derive(Default)]
pub struct LiveTranslation {
    running: Option<Running>,
}

impl LiveTranslation {
    /// Starts the translator for `want`, or stops it for None. `on_event` brings its answers to the UI thread.
    pub fn sync(
        &mut self,
        want: Option<Pair>,
        data_dir: Option<&Path>,
        on_event: impl Fn(Pair, TranslatorEvent) + Send + Clone + 'static,
    ) {
        if self.running.as_ref().map(|r| r.pair) == want {
            return;
        }
        self.running = None;
        let (Some(pair), Some(data_dir)) = (want, data_dir) else { return };
        let translator = Translator::start(data_dir, pair, move |event| on_event(pair, event));
        self.running = Some(Running {
            pair,
            translator,
            ready: false,
            done: HashMap::new(),
            order: VecDeque::new(),
            asked: HashSet::new(),
        });
    }

    /// Whether lines can be translated: loaded, and not failed
    pub fn ready(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.ready)
    }

    /// The translation of `text` if it's done; asks for it if it hasn't been asked for yet.
    pub fn lookup(&mut self, text: &str) -> Option<String> {
        let running = self.running.as_mut().filter(|r| r.ready)?;
        if let Some(done) = running.done.get(text) {
            return Some(done.clone());
        }
        if running.asked.insert(text.to_string()) {
            running.translator.translate(text.to_string());
        }
        None
    }

    /// Handles what the translator said; returns whether shown lines may change.
    pub fn event(&mut self, pair: Pair, event: TranslatorEvent) -> bool {
        let Some(running) = self.running.as_mut().filter(|r| r.pair == pair) else { return false };
        match event {
            TranslatorEvent::Ready => running.ready = true,
            TranslatorEvent::Translated { text, translation } => {
                running.asked.remove(&text);
                // More waiting than the translator keeps: some were dropped, and are asked again if still shown
                if running.asked.len() > MAX_WAITING {
                    running.asked.clear();
                }
                if running.order.len() >= MAX_KEPT
                    && let Some(oldest) = running.order.pop_front()
                {
                    running.done.remove(&oldest);
                }
                running.order.push_back(text.clone());
                running.done.insert(text, translation);
            }
            // Lines show as said, like before the model was downloaded; not started again for the same direction
            TranslatorEvent::Failed => running.ready = false,
        }
        true
    }
}
