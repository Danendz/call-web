//! The chat's rows and the subtitle, kept on the UI thread. Messages come in as [`ChatEvent`]s and update the list in
//! place: a new message appends a row, a translation rewrites one. What a row says depends on your language, so a
//! language change rewrites them all.

use chrono::{DateTime, Local};
use engine::{ChatEvent, ChatMessage, Lang};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

use crate::ui::{AppWindow, ChatRow};

pub struct Chat {
    rows: Rc<VecModel<ChatRow>>,
    messages: Vec<ChatMessage>,
    my_id: String,
    lang: Lang,
    /// The message the subtitle shows, to follow its translation
    subtitle_id: Option<String>,
}

impl Chat {
    pub fn new(ui: &AppWindow, lang: Lang) -> Self {
        let rows = Rc::new(VecModel::default());
        ui.set_chat(ModelRc::from(rows.clone()));
        Self { rows, messages: Vec::new(), my_id: String::new(), lang, subtitle_id: None }
    }

    pub fn event(&mut self, ui: &AppWindow, event: ChatEvent) {
        match event {
            ChatEvent::Reset { my_id, messages } => {
                self.my_id = my_id;
                self.messages = messages;
                self.rows.set_vec(self.messages.iter().map(|m| self.row(m)).collect::<Vec<_>>());
                // Lines from before you joined aren't shown as subtitles
                self.subtitle_id = None;
            }
            ChatEvent::Added(message) => {
                self.rows.push(self.row(&message));
                if message.spoken == Some(true) && message.from != self.my_id {
                    self.subtitle_id = Some(message.id.clone());
                    self.show_subtitle(ui, &message);
                }
                self.messages.push(message);
            }
            ChatEvent::Translated { id, translation } => {
                let Some(index) = self.messages.iter().position(|m| m.id == id) else { return };
                let Some(message) = self.messages.get_mut(index) else { return };
                message.translation = Some(translation);
                let message = message.clone();
                self.rows.set_row_data(index, self.row(&message));
                if self.subtitle_id.as_deref() == Some(id.as_str()) {
                    self.show_subtitle(ui, &message);
                }
            }
        }
        let others = self.messages.iter().filter(|m| m.from != self.my_id).count();
        ui.set_others_count(i32::try_from(others).unwrap_or(i32::MAX));
    }

    pub fn set_lang(&mut self, lang: Lang) {
        if lang == self.lang {
            return;
        }
        self.lang = lang;
        for (index, message) in self.messages.iter().enumerate() {
            if index < self.rows.row_count() {
                self.rows.set_row_data(index, self.row(message));
            }
        }
    }

    fn show_subtitle(&self, ui: &AppWindow, message: &ChatMessage) {
        ui.set_subtitle(readable(message, &self.my_id, self.lang).0.into());
        ui.set_subtitle_serial(ui.get_subtitle_serial().wrapping_add(1));
    }

    fn row(&self, m: &ChatMessage) -> ChatRow {
        let (text, original) = readable(m, &self.my_id, self.lang);
        ChatRow {
            id: SharedString::from(&m.id),
            text: text.into(),
            original: original.unwrap_or_default().into(),
            mine: m.from == self.my_id,
            spoken: m.spoken == Some(true),
            time: local_time(m.ts).into(),
        }
    }
}

/// The web's readable(): the other person's message in your language with what they wrote under it, your own as
/// written, and anything already in your language as is.
fn readable<'a>(m: &'a ChatMessage, my_id: &str, lang: Lang) -> (&'a str, Option<&'a str>) {
    let Some(t) = m.translation.as_ref().filter(|_| m.from != my_id) else { return (&m.text, None) };
    let (code, text) = match lang {
        Lang::Zh => ("zh", &t.text.zh),
        Lang::En => ("en", &t.text.en),
    };
    if t.lang == code { (&m.text, None) } else { (text, Some(&m.text)) }
}

/// "14:05" on this machine's clock.
pub fn local_time(epoch_ms: u64) -> String {
    i64::try_from(epoch_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|at| at.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use engine::{TranslatedText, Translation};

    use super::*;

    fn message(from: &str, text: &str, lang: &str) -> ChatMessage {
        ChatMessage {
            id: "m".into(),
            from: from.into(),
            text: text.into(),
            ts: 0,
            spoken: None,
            translation: Some(Translation {
                lang: lang.into(),
                text: TranslatedText { zh: "你好".into(), en: "hello".into() },
            }),
        }
    }

    #[test]
    fn theirs_is_translated_with_the_original_under_it() {
        let m = message("them", "你好", "zh");
        assert_eq!(readable(&m, "me", Lang::En), ("hello", Some("你好")));
    }

    #[test]
    fn already_in_your_language_or_your_own_stays_as_written() {
        assert_eq!(readable(&message("them", "你好", "zh"), "me", Lang::Zh), ("你好", None));
        assert_eq!(readable(&message("me", "你好", "zh"), "me", Lang::En), ("你好", None));
    }
}
