//! The one rule that splits a message into the words the text rules compare:
//! the person-message rules in `xt_metrics::session_warnings` (corrections,
//! repeated prompts) and the agent-reply admission facts
//! ([`crate::reply_admission`]). Nothing here is kept: words are compared in
//! memory and dropped.

/// One word of a message as the text rules compare it, and where it came
/// from in the original text, as a byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub span: std::ops::Range<usize>,
}

/// Whether a character, once lowercased with curly apostrophes made
/// straight, belongs to a word: a letter, digit, apostrophe or hyphen.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '-' || c == '\u{2019}'
}

/// The words of a message as the text rules compare them: lowercase, curly
/// apostrophes made straight, split on anything that is not a letter, digit,
/// apostrophe or hyphen, with apostrophes and hyphens trimmed from each end.
/// Each keeps the byte range of the original text it came from (every
/// lowercase character maps to the original character it was made from), so
/// a match can be shown in the message as written.
pub fn spanned_words(text: &str) -> Vec<Word> {
    let lower = text.to_lowercase();
    let mut lowered = lower.chars();
    let mut words = Vec::new();
    let mut current: Vec<(char, usize, usize)> = Vec::new();
    let mut flush = |current: &mut Vec<(char, usize, usize)>| {
        let trim = |c: &(char, usize, usize)| c.0 == '\'' || c.0 == '-';
        let first = current.iter().position(|c| !trim(c));
        let last = current.iter().rposition(|c| !trim(c));
        if let (Some(first), Some(last)) = (first, last) {
            let kept = &current[first..=last];
            words.push(Word {
                text: kept.iter().map(|c| c.0).collect(),
                span: kept[0].1..kept[kept.len() - 1].2,
            });
        }
        current.clear();
    };
    for (at, original) in text.char_indices() {
        let end = at + original.len_utf8();
        // `str::to_lowercase` writes as many characters for each character as
        // `char::to_lowercase` does (a final sigma differs only in which one).
        for _ in 0..original.to_lowercase().count() {
            let Some(mut c) = lowered.next() else { break };
            if c == '\u{2019}' {
                c = '\'';
            }
            if c.is_alphanumeric() || c == '\'' || c == '-' {
                current.push((c, at, end));
            } else {
                flush(&mut current);
            }
        }
    }
    flush(&mut current);
    words
}

/// The words of a message as the text rules compare them ([`spanned_words`]).
pub fn words(text: &str) -> Vec<String> {
    spanned_words(text)
        .into_iter()
        .map(|word| word.text)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset-keeping split gives exactly the words the split before
    /// it gave, on text where lowercasing changes lengths or depends on
    /// position.
    #[test]
    fn spanned_words_are_the_words_the_old_split_gave() {
        fn legacy(text: &str) -> Vec<String> {
            text.to_lowercase()
                .replace('\u{2019}', "'")
                .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
                .map(|word| word.trim_matches(|c| c == '\'' || c == '-'))
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect()
        }
        for text in [
            "ΟΔΟΣ ΣΑΣ σ",
            "İstanbul İİ",
            "it\u{2019}s 'quoted' -dash- a--b ''",
            "ﬁle ß ẞ Ǆ",
            "tab\tnew\nline 🙂emoji🙂",
            "",
        ] {
            assert_eq!(words(text), legacy(text), "{text}");
        }
    }

    #[test]
    fn a_word_character_is_what_the_split_keeps_together() {
        for c in ['a', 'É', '7', '\'', '-', '\u{2019}'] {
            assert!(is_word_char(c), "{c}");
        }
        for c in [' ', ',', '.', '"', '\n', '—'] {
            assert!(!is_word_char(c), "{c}");
        }
    }
}
