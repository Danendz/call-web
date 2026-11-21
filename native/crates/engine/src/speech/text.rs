//! The text of a recognized line, as the chat shows it.

/// The line to post, or None when it's only punctuation or a filler: in the chat a lone "嗯" is noise even when it
/// was really said, since it was heard anyway.
pub(super) fn clean(text: &str) -> Option<String> {
    let words: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    (!words.is_empty() && !fillers(&words)).then(|| text.trim().to_owned())
}

/// Whether `words` is fillers and nothing else, like the server's `FILLER_RE`.
fn fillers(words: &[char]) -> bool {
    let Some((first, rest)) = words.split_first() else { return true };
    if "嗯啊哦呃额唔".contains(*first) && fillers(rest) {
        return true;
    }
    for fixed in [['h', 'u', 'h'], ['m', 'h', 'm']] {
        if let Some(rest) = words.strip_prefix(&fixed[..])
            && fillers(rest)
        {
            return true;
        }
    }
    // "mm", "hmm", "uhh", "umm", "ahh", "ohh": a letter, then one or more of another
    for (letter, repeated) in [('m', 'm'), ('h', 'm'), ('u', 'h'), ('u', 'm'), ('a', 'h'), ('o', 'h')] {
        if *first != letter {
            continue;
        }
        let run = rest.iter().take_while(|&&c| c == repeated).count();
        if (1..=run).any(|n| rest.get(n..).is_some_and(fillers)) {
            return true;
        }
    }
    false
}

/// Lines said one after another, as one: Chinese and Japanese are written without spaces between words.
pub(super) fn join<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    let mut all = String::new();
    for text in texts {
        let cjk_boundary = all.chars().next_back().is_some_and(cjk) || text.chars().next().is_some_and(cjk);
        if !all.is_empty() && !cjk_boundary {
            all.push(' ');
        }
        all.push_str(text);
    }
    all
}

fn cjk(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}')
}

#[cfg(test)]
mod tests {
    use super::{clean, join};

    #[test]
    fn fillers_and_words() {
        for filler in ["嗯。", "Mm-hmm.", "Uhh, um", "Hmm?", "啊哦", "mhm"] {
            assert_eq!(clean(filler), None, "{filler}");
        }
        for words in ["嗯，好的。", "Hmm, maybe.", "Oh no", "um, yes", "Ahead"] {
            assert_eq!(clean(words).as_deref(), Some(words), "{words}");
        }
    }

    #[test]
    fn lines_join_with_spaces_only_between_words_that_take_them() {
        assert_eq!(join(["Hello.", "How are you?"]), "Hello. How are you?");
        assert_eq!(join(["你好。", "我是"]), "你好。我是");
        assert_eq!(join(["我用", "iPhone"]), "我用iPhone");
    }
}
