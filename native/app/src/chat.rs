//! The chat's rows and the subtitles, kept on the UI thread. Messages come in as [`ChatEvent`]s and update the list in
//! place: a new message appends a row, a translation rewrites one. What a row says depends on your language, so a
//! language change rewrites them all.

use chrono::{DateTime, Local};

use engine::{ChatEvent, ChatMessage, Lang, Pair, TranslatorEvent};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::path::Path;
use std::rc::Rc;

use crate::subtitles::Subtitles;
use crate::ui::{AppWindow, ChatRow};

pub struct Chat {
    rows: Rc<VecModel<ChatRow>>,
    messages: Vec<ChatMessage>,
    my_id: String,
    lang: Lang,
    subtitles: Subtitles,
}

impl Chat {
    pub fn new(ui: &AppWindow, lang: Lang) -> Self {
        let rows = Rc::new(VecModel::default());
        ui.set_chat(ModelRc::from(rows.clone()));
        Self { rows, messages: Vec::new(), my_id: String::new(), lang, subtitles: Subtitles::new(ui, lang) }
    }

    pub fn event(&mut self, ui: &AppWindow, event: ChatEvent) {
        match event {
            ChatEvent::Reset { my_id, messages } => {
                self.my_id = my_id;
                self.messages = messages;
                self.rows.set_vec(self.messages.iter().map(|m| self.row(m)).collect::<Vec<_>>());
                self.subtitles.clear();
            }
            ChatEvent::Added(message) => {
                self.rows.push(self.row(&message));
                self.messages.push(message);
                if let Some(message) = self.messages.last() {
                    self.subtitles.added(message, &self.messages, &self.my_id, self.lang);
                }
            }
            ChatEvent::Live { id, text } => {
                self.subtitles.live(id, text, &self.messages, &self.my_id, self.lang);
                return;
            }
            // The bridge flies these; they're never part of the chat
            ChatEvent::Reaction { .. } => return,
            ChatEvent::Translated { id, translation } => {
                let Some(index) = self.messages.iter().position(|m| m.id == id) else { return };
                let Some(message) = self.messages.get_mut(index) else { return };
                message.translation = Some(translation);
                let message = message.clone();
                self.rows.set_row_data(index, self.row(&message));
                self.subtitles.refresh(&self.messages, &self.my_id, self.lang);
            }
        }
        let others = self.messages.iter().filter(|m| m.from != self.my_id).count();
        ui.set_others_count(i32::try_from(others).unwrap_or(i32::MAX));
    }

    /// Starts or stops translating subtitles here, for `want`'s direction
    pub fn sync_translation(
        &mut self,
        want: Option<Pair>,
        data_dir: Option<&Path>,
        on_event: impl Fn(Pair, TranslatorEvent) + Send + Clone + 'static,
    ) {
        self.subtitles.sync_translation(want, data_dir, on_event);
    }

    pub fn translation_event(&mut self, pair: Pair, event: TranslatorEvent) {
        self.subtitles.translation_event(pair, event, &self.messages, &self.my_id, self.lang);
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
        self.subtitles.refresh(&self.messages, &self.my_id, self.lang);
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
pub fn readable<'a>(m: &'a ChatMessage, my_id: &str, lang: Lang) -> (&'a str, Option<&'a str>) {
    let Some(t) = m.translation.as_ref().filter(|_| m.from != my_id) else { return (&m.text, None) };
    let (code, text) = match lang {
        Lang::Zh => ("zh", &t.text.zh),
        Lang::En => ("en", &t.text.en),
    };
    if t.lang == code && !has_other_script(&m.text, lang) { (&m.text, None) } else { (text, Some(&m.text)) }
}

/// One language per message comes back, so "yep 你好" is labelled one of the two; it still needs translating when part
/// of it is in the other script. For a Chinese reader a stray "OK" or brand name doesn't count, a real share of English
/// words does. Kept the same as the web's readable.ts.
pub fn has_other_script(text: &str, lang: Lang) -> bool {
    let han = text.chars().filter(|&c| is_han(c)).count();
    if lang == Lang::En {
        return han > 0;
    }
    // All-caps words are abbreviations like OK or USB, which a Chinese reader knows
    let words =
        text.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| w.chars().any(|c| c.is_ascii_lowercase())).count();
    words > 0 && words as f64 / (words + han) as f64 >= 0.3
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{2ffff}')
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
            live_id: None,
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

    #[test]
    fn mixed_languages_are_translated_whichever_label_they_got() {
        assert_eq!(readable(&message("them", "yep 你好", "en"), "me", Lang::En), ("hello", Some("yep 你好")));
        assert_eq!(readable(&message("them", "yep 你好", "zh"), "me", Lang::Zh), ("你好", Some("yep 你好")));
    }

    #[test]
    fn a_stray_abbreviation_in_chinese_stays_as_written() {
        assert_eq!(readable(&message("them", "OK 好的", "zh"), "me", Lang::Zh), ("OK 好的", None));
        assert_eq!(
            readable(&message("them", "我今天用了 Zoom 开会", "zh"), "me", Lang::Zh),
            ("我今天用了 Zoom 开会", None)
        );
    }
}
