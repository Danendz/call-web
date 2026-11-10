//! The last few spoken lines over the video, like the web's Subtitles: the other person's in your language, your own
//! as said, and what they're saying right now as it comes. A finished line takes the place of its live text. The
//! other language is translated on this device as it comes (`translate.rs`), then by the server once a line is
//! finished. Rows are updated in place rather than rebuilt, so each one keeps its own fade; fading after a pause and
//! hiding while the chat is open happen in subtitles.slint.

use std::path::Path;

use engine::{ChatMessage, Lang, Pair, TranslatorEvent};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

use crate::chat::{has_other_script, readable};
use crate::translate::LiveTranslation;
use crate::ui::{AppWindow, SubtitleLine};

const MAX_LINES: usize = 4;
// Lines past the shown ones stay a while, so being pushed out animates
const KEPT: usize = MAX_LINES + 4;

struct Entry {
    /// The live id, or the message id for a line that had no live text
    key: String,
    /// Set once the chat line has come; until then it's live text
    message: Option<String>,
    live: String,
    /// The last translation made here, kept until the next is in, so growing live text doesn't flash the original
    quick: Option<String>,
    /// In the other language with nothing translated yet: it waits the few ms that takes, rather than flash the
    /// original
    pending: bool,
}

pub struct Subtitles {
    rows: Rc<VecModel<SubtitleLine>>,
    entries: Vec<Entry>,
    translation: LiveTranslation,
    lang: Lang,
}

impl Subtitles {
    pub fn new(ui: &AppWindow, lang: Lang) -> Self {
        let rows = Rc::new(VecModel::default());
        ui.set_subtitles(ModelRc::from(rows.clone()));
        Self { rows, entries: Vec::new(), translation: LiveTranslation::default(), lang }
    }

    /// Lines from before you joined aren't shown
    pub fn clear(&mut self) {
        self.entries.clear();
        self.rows.set_vec(Vec::new());
    }

    /// Starts or stops translating here; see [`LiveTranslation::sync`]
    pub fn sync_translation(
        &mut self,
        want: Option<Pair>,
        data_dir: Option<&Path>,
        on_event: impl Fn(Pair, TranslatorEvent) + Send + Clone + 'static,
    ) {
        self.translation.sync(want, data_dir, on_event);
    }

    pub fn translation_event(
        &mut self,
        pair: Pair,
        event: TranslatorEvent,
        messages: &[ChatMessage],
        my_id: &str,
        lang: Lang,
    ) {
        if self.translation.event(pair, event) {
            self.refresh(messages, my_id, lang);
        }
    }

    pub fn added(&mut self, message: &ChatMessage, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        if message.spoken != Some(true) {
            return;
        }
        let key = message.live_id.clone().unwrap_or_else(|| message.id.clone());
        match self.entries.iter().position(|e| e.key == key && e.message.is_none()) {
            Some(index) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.message = Some(message.id.clone());
                    entry.live.clear();
                }
                self.update(index, messages, my_id, lang);
            }
            None => self.push(
                Entry { key, message: Some(message.id.clone()), live: String::new(), quick: None, pending: false },
                messages,
                my_id,
                lang,
            ),
        }
    }

    pub fn live(&mut self, id: String, text: String, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        // Ids come from the other side and may repeat, so a finished line never takes live text again. Its chat line
        // comes after its last live text on the same socket, so nothing is late for it.
        let found = self.entries.iter().position(|e| e.key == id && e.message.is_none());
        match found {
            Some(index) if text.is_empty() => {
                self.entries.remove(index);
                self.rows.remove(index);
                self.mark_shown();
            }
            Some(index) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.live = text;
                }
                self.update(index, messages, my_id, lang);
            }
            None if text.is_empty() => {}
            None => self.push(
                Entry { key: id, message: None, live: text, quick: None, pending: false },
                messages,
                my_id,
                lang,
            ),
        }
    }

    /// A translation came, or the language changed: rewrite the rows from their messages
    pub fn refresh(&mut self, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        if self.lang != lang {
            // Translations into the language before don't count
            self.lang = lang;
            for entry in &mut self.entries {
                entry.quick = None;
            }
        }
        for index in 0..self.entries.len() {
            self.update(index, messages, my_id, lang);
        }
    }

    fn update(&mut self, index: usize, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        let Some(row) = self.row(index, messages, my_id, lang) else { return };
        if self.rows.row_data(index).as_ref() != Some(&row) {
            self.rows.set_row_data(index, row);
        }
    }

    fn push(&mut self, entry: Entry, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        self.entries.push(entry);
        let index = self.entries.len() - 1;
        if let Some(row) = self.row(index, messages, my_id, lang) {
            self.rows.push(row);
        }
        while self.entries.len() > KEPT {
            self.entries.remove(0);
            self.rows.remove(0);
        }
        self.mark_shown();
    }

    /// The newest few lines show
    fn is_shown(&self, index: usize) -> bool {
        index + MAX_LINES >= self.entries.len()
    }

    fn mark_shown(&mut self) {
        for index in 0..self.rows.row_count() {
            let Some(mut row) = self.rows.row_data(index) else { continue };
            let shown = self.is_shown(index) && !self.entries.get(index).is_some_and(|e| e.pending);
            if row.shown != shown {
                row.shown = shown;
                self.rows.set_row_data(index, row);
            }
        }
    }

    fn row(&mut self, index: usize, messages: &[ChatMessage], my_id: &str, lang: Lang) -> Option<SubtitleLine> {
        let shown = self.is_shown(index);
        let entry = self.entries.get(index)?;
        let message = entry.message.as_ref().and_then(|id| messages.iter().find(|m| &m.id == id));
        let (source, text, mine) = match message {
            Some(m) => (m.text.clone(), readable(m, my_id, lang).0.to_string(), m.from == my_id),
            None => (entry.live.clone(), entry.live.clone(), false),
        };
        let needed = !mine && has_other_script(&source, lang);
        let quick = if needed { self.translation.lookup(&source) } else { None };
        let entry = self.entries.get_mut(index)?;
        if quick.is_some() {
            entry.quick = quick;
        }
        let pending = needed && self.translation.ready() && entry.quick.is_none();
        entry.pending = pending;
        let first = entry.quick.clone().unwrap_or_else(|| source.clone());
        Some(SubtitleLine {
            translated: text != source && text != first,
            first: first.into(),
            text: SharedString::from(text),
            mine,
            shown: shown && !pending,
        })
    }
}
