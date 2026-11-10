//! The last few spoken lines over the video, like the web's Subtitles: the other person's in your language, your own
//! as said, and what they're saying right now as it comes. A finished line takes the place of its live text. Rows
//! are updated in place rather than rebuilt, so each one keeps its own fade; fading after a pause and hiding while
//! the chat is open happen in subtitles.slint.

use engine::{ChatMessage, Lang};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

use crate::chat::readable;
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
}

pub struct Subtitles {
    rows: Rc<VecModel<SubtitleLine>>,
    entries: Vec<Entry>,
}

impl Subtitles {
    pub fn new(ui: &AppWindow) -> Self {
        let rows = Rc::new(VecModel::default());
        ui.set_subtitles(ModelRc::from(rows.clone()));
        Self { rows, entries: Vec::new() }
    }

    /// Lines from before you joined aren't shown
    pub fn clear(&mut self) {
        self.entries.clear();
        self.rows.set_vec(Vec::new());
    }

    pub fn added(&mut self, message: &ChatMessage, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        if message.spoken != Some(true) {
            return;
        }
        let key = message.live_id.clone().unwrap_or_else(|| message.id.clone());
        let entry = Entry { key, message: Some(message.id.clone()), live: String::new() };
        match self.entries.iter().position(|e| e.key == entry.key && e.message.is_none()) {
            Some(index) => self.replace(index, entry, messages, my_id, lang),
            None => self.push(entry, messages, my_id, lang),
        }
    }

    pub fn live(&mut self, id: String, text: String, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        // Ids come from the other side and may repeat, so a finished line never takes live text again. Its chat line
        // comes after its last live text on the same socket, so nothing is late for it.
        let found = self.entries.iter().position(|e| e.key == id && e.message.is_none());
        let entry = Entry { key: id, message: None, live: text };
        match found {
            Some(index) if entry.live.is_empty() => {
                self.entries.remove(index);
                self.rows.remove(index);
                self.mark_shown();
            }
            Some(index) => self.replace(index, entry, messages, my_id, lang),
            None if entry.live.is_empty() => {}
            None => self.push(entry, messages, my_id, lang),
        }
    }

    /// A message's translation came, or the language changed: rewrite the rows from their messages
    pub fn refresh(&mut self, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        for (index, entry) in self.entries.iter().enumerate() {
            let row = self.row(entry, self.is_shown(index), messages, my_id, lang);
            if self.rows.row_data(index).as_ref() != Some(&row) {
                self.rows.set_row_data(index, row);
            }
        }
    }

    fn replace(&mut self, index: usize, entry: Entry, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        let row = self.row(&entry, self.is_shown(index), messages, my_id, lang);
        if let Some(slot) = self.entries.get_mut(index) {
            *slot = entry;
            self.rows.set_row_data(index, row);
        }
    }

    fn push(&mut self, entry: Entry, messages: &[ChatMessage], my_id: &str, lang: Lang) {
        self.rows.push(self.row(&entry, true, messages, my_id, lang));
        self.entries.push(entry);
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

    fn mark_shown(&self) {
        for index in 0..self.rows.row_count() {
            let Some(mut row) = self.rows.row_data(index) else { continue };
            if row.shown != self.is_shown(index) {
                row.shown = self.is_shown(index);
                self.rows.set_row_data(index, row);
            }
        }
    }

    fn row(&self, entry: &Entry, shown: bool, messages: &[ChatMessage], my_id: &str, lang: Lang) -> SubtitleLine {
        let message = entry.message.as_ref().and_then(|id| messages.iter().find(|m| &m.id == id));
        let Some(m) = message else {
            let text = SharedString::from(&entry.live);
            return SubtitleLine { first: text.clone(), text, translated: false, mine: false, shown };
        };
        let text = readable(m, my_id, lang).0;
        SubtitleLine {
            first: m.text.as_str().into(),
            text: text.into(),
            translated: text != m.text,
            mine: m.from == my_id,
            shown,
        }
    }
}
