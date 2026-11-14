//! What a chat line says to you: the other person's in your language with what they wrote under it, your own as
//! written.

use chrono::{DateTime, Local};

use engine::{ChatMessage, Lang};

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
