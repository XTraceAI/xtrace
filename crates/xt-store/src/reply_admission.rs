//! Which fixed admission phrases an agent's reply opens with: the single owner
//! of the phrase table (each phrase's stored name, the words it matches and
//! the label a warning shows), of the matcher, and of the saved per-reply
//! facts (`reply_admissions`, migration 30).
//!
//! The facts are read at write time from the record's own text blocks, as
//! the tool-result facts are ([`crate::tool_result`]), so a metadata-only
//! index keeps them once the blocks are discarded. Nothing here keeps text:
//! a reply's words are compared in memory and dropped, and only the fixed
//! names of the phrases found are saved.
//!
//! Which phrases make a warning is decided when warnings are computed
//! (`xt_metrics::session_warnings`), so narrowing that list needs no rescan.
//! Adding or changing a phrase here changes what is saved: bump
//! [`FACT_VERSION`] with it, and reset the Claude checkpoints in a migration
//! so older replies are read again.
//!
//! A saved row is replaced only by an observation with a higher
//! [`FACT_VERSION`]; one with the same version that states other phrases
//! marks the row conflicted, which readers treat as unknown.

use crate::{
    Result,
    batch::RecordOutcome,
    model::{CanonicalRecord, RecordType},
    record_text,
    words::{Word, is_word_char, spanned_words},
};
use rusqlite::{Connection, params};

/// The write-time derivation of the saved phrases this build implements.
/// Bump it only together with a change to [`phrases`] or the phrase table;
/// rows written under a lower version are then replaced when their source is
/// read again.
pub const FACT_VERSION: i64 = 1;
/// How many characters (not bytes) of a reply's joined text blocks are read.
pub const REPLY_CHARS: usize = 600;
/// The longest saved phrase list (the table's CHECK).
pub const MAX_PHRASES_CHARS: usize = 600;
/// The longest record identifier a row is kept for (the table's CHECK).
/// `records.uuid` has no such limit; a longer one gets no row, and the
/// warning rule reads that reply as not checked.
pub const MAX_RECORD_ID_CHARS: usize = 256;

/// One admission phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phrase {
    YouAreRight,
    MyMistake,
    MyBad,
    IWasWrong,
    ThatWasWrong,
    IGotItWrong,
    IMisread,
    IMisunderstood,
    IOverstated,
    IOverclaimed,
    IShouldHave,
    GoodCatch,
    YouCaught,
    IExplainedBadly,
    ThatsOnMe,
    IMadeAMistake,
    IWasMistaken,
    Sorry,
    Apology,
    IMissed,
    IDidntAnswer,
    MyError,
    FairPoint,
    IWasToo,
    IConflated,
    IMixedUp,
}

/// One word position of a phrase.
#[derive(Clone, Copy, Debug)]
enum Slot {
    /// Exactly this word.
    Is(&'static str),
    /// Any one of these words.
    OneOf(&'static [&'static str]),
    /// A word that starts with this ("apolog": apology, apologies, apologize).
    StartsWith(&'static str),
}

use Slot::{Is, OneOf, StartsWith};

/// Everything about one phrase, defined once.
#[derive(Clone, Copy, Debug)]
pub struct PhraseSpec {
    /// The stored name: lowercase ASCII letters and `_` only.
    pub name: &'static str,
    /// How a warning quotes it.
    pub label: &'static str,
    /// The word runs that match it, each a whole run of whole words.
    forms: &'static [&'static [Slot]],
}

const THAT_IT_THIS: &[&str] = &["that", "it", "this"];

impl Phrase {
    /// Every phrase, in the order the table lists them. Two phrases found at
    /// the same word are saved in this order.
    pub const ALL: [Self; 26] = [
        Self::YouAreRight,
        Self::MyMistake,
        Self::MyBad,
        Self::IWasWrong,
        Self::ThatWasWrong,
        Self::IGotItWrong,
        Self::IMisread,
        Self::IMisunderstood,
        Self::IOverstated,
        Self::IOverclaimed,
        Self::IShouldHave,
        Self::GoodCatch,
        Self::YouCaught,
        Self::IExplainedBadly,
        Self::ThatsOnMe,
        Self::IMadeAMistake,
        Self::IWasMistaken,
        Self::Sorry,
        Self::Apology,
        Self::IMissed,
        Self::IDidntAnswer,
        Self::MyError,
        Self::FairPoint,
        Self::IWasToo,
        Self::IConflated,
        Self::IMixedUp,
    ];

    /// The phrase table: each phrase's name, label and the words it matches.
    pub fn spec(self) -> PhraseSpec {
        let (name, label, forms): (_, _, &'static [&'static [Slot]]) = match self {
            Self::YouAreRight => (
                "you_are_right",
                "you're right",
                &[
                    &[Is("you're"), Is("right")],
                    &[Is("you"), Is("are"), Is("right")],
                ],
            ),
            Self::MyMistake => ("my_mistake", "my mistake", &[&[Is("my"), Is("mistake")]]),
            Self::MyBad => ("my_bad", "my bad", &[&[Is("my"), Is("bad")]]),
            Self::IWasWrong => (
                "i_was_wrong",
                "I was wrong",
                &[&[Is("i"), Is("was"), Is("wrong")]],
            ),
            Self::ThatWasWrong => (
                "that_was_wrong",
                "that was wrong",
                &[&[Is("that"), Is("was"), Is("wrong")]],
            ),
            Self::IGotItWrong => (
                "i_got_it_wrong",
                "I got it wrong",
                &[&[Is("i"), Is("got"), OneOf(THAT_IT_THIS), Is("wrong")]],
            ),
            Self::IMisread => ("i_misread", "I misread", &[&[Is("i"), Is("misread")]]),
            Self::IMisunderstood => (
                "i_misunderstood",
                "I misunderstood",
                &[&[Is("i"), Is("misunderstood")]],
            ),
            Self::IOverstated => (
                "i_overstated",
                "I overstated",
                &[&[Is("i"), Is("overstated")]],
            ),
            Self::IOverclaimed => (
                "i_overclaimed",
                "I overclaimed",
                &[&[Is("i"), Is("overclaimed")]],
            ),
            Self::IShouldHave => (
                "i_should_have",
                "I should have",
                &[&[Is("i"), Is("should"), Is("have")]],
            ),
            Self::GoodCatch => ("good_catch", "good catch", &[&[Is("good"), Is("catch")]]),
            Self::YouCaught => ("you_caught", "you caught", &[&[Is("you"), Is("caught")]]),
            Self::IExplainedBadly => (
                "i_explained_badly",
                "I explained it badly",
                &[
                    &[Is("i"), Is("explained"), Is("badly")],
                    &[Is("i"), Is("explained"), OneOf(THAT_IT_THIS), Is("badly")],
                ],
            ),
            Self::ThatsOnMe => (
                "thats_on_me",
                "that's on me",
                &[&[Is("that's"), Is("on"), Is("me")]],
            ),
            Self::IMadeAMistake => (
                "i_made_a_mistake",
                "I made a mistake",
                &[&[Is("i"), Is("made"), Is("a"), Is("mistake")]],
            ),
            Self::IWasMistaken => (
                "i_was_mistaken",
                "I was mistaken",
                &[&[Is("i"), Is("was"), Is("mistaken")]],
            ),
            // "I'm sorry" is covered: its last word is "sorry".
            Self::Sorry => ("sorry", "sorry", &[&[Is("sorry")]]),
            Self::Apology => ("apology", "apologies", &[&[StartsWith("apolog")]]),
            Self::IMissed => ("i_missed", "I missed", &[&[Is("i"), Is("missed")]]),
            Self::IDidntAnswer => (
                "i_didnt_answer",
                "I didn't answer",
                &[&[Is("i"), Is("didn't"), Is("answer")]],
            ),
            // "That's my error" is covered: it ends in "my error".
            Self::MyError => ("my_error", "my error", &[&[Is("my"), Is("error")]]),
            Self::FairPoint => ("fair_point", "fair point", &[&[Is("fair"), Is("point")]]),
            Self::IWasToo => (
                "i_was_too",
                "I was too",
                &[&[Is("i"), Is("was"), Is("too")]],
            ),
            Self::IConflated => ("i_conflated", "I conflated", &[&[Is("i"), Is("conflated")]]),
            Self::IMixedUp => (
                "i_mixed_up",
                "I mixed up",
                &[&[Is("i"), Is("mixed"), Is("up")]],
            ),
        };
        PhraseSpec { name, label, forms }
    }

    /// The stored name.
    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// How a warning quotes the phrase.
    pub fn label(self) -> &'static str {
        self.spec().label
    }

    /// The phrase a stored name names, if this build knows it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|phrase| phrase.name() == name)
    }

    /// Whether one of the phrase's word runs starts at `words[at]`.
    fn matches_at(self, words: &[Word], at: usize) -> bool {
        self.spec().forms.iter().any(|form| {
            form.len() <= words.len().saturating_sub(at)
                && form.iter().zip(&words[at..]).all(|(slot, word)| {
                    let word = word.text.as_str();
                    match slot {
                        Is(expected) => word == *expected,
                        OneOf(options) => options.contains(&word),
                        StartsWith(prefix) => word.starts_with(prefix),
                    }
                })
        })
    }
}

impl serde::Serialize for Phrase {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for Phrase {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::parse(&name).ok_or_else(|| serde::de::Error::custom("unknown admission phrase"))
    }
}

/// The admission phrases in the first [`REPLY_CHARS`] characters of a
/// reply's text, each once, in order of first appearance (two found at the
/// same word in [`Phrase::ALL`] order). Phrases match whole words only, as
/// [`spanned_words`] splits them; only "apolog" matches the start of a word.
/// A word the limit cuts in two is left out.
pub fn phrases(text: &str) -> Vec<Phrase> {
    let (head, next) = match text.char_indices().nth(REPLY_CHARS) {
        Some((at, next)) => (&text[..at], Some(next)),
        None => (text, None),
    };
    let mut words = spanned_words(head);
    let cut_in_word =
        next.is_some_and(is_word_char) && head.chars().last().is_some_and(is_word_char);
    if cut_in_word {
        words.pop();
    }
    let mut found = Vec::new();
    for at in 0..words.len() {
        for phrase in Phrase::ALL {
            if !found.contains(&phrase) && phrase.matches_at(&words, at) {
                found.push(phrase);
            }
        }
    }
    found
}

/// How a phrase list is saved: names separated by single spaces, empty for
/// none.
pub fn to_saved(found: &[Phrase]) -> String {
    found
        .iter()
        .map(|phrase| phrase.name())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A saved phrase list read back; `None` when it names a phrase this build
/// does not know.
pub fn from_saved(saved: &str) -> Option<Vec<Phrase>> {
    saved.split_whitespace().map(Phrase::parse).collect()
}

/// Whether an agent record has reply text: its joined text blocks are not
/// empty. The same test as a stored `text_len > 0` (`text_len` counts the
/// characters of the text blocks only).
pub fn has_reply_text(blocks: &[serde_json::Value]) -> bool {
    !record_text::joined(blocks).is_empty()
}

/// Keep one reply's phrases under `version`: a new row, a replacement of a
/// row written under a lower version, nothing for the same phrases, and a
/// conflict mark for other phrases under the same version. Only a record
/// this session owns, unconflicted, is written to, and none for an
/// identifier over [`MAX_RECORD_ID_CHARS`] characters. Returns whether a row
/// was written or replaced.
pub(crate) fn keep(
    connection: &Connection,
    session_id: &str,
    uuid: &str,
    found: &[Phrase],
    version: i64,
) -> Result<bool> {
    if uuid.chars().count() > MAX_RECORD_ID_CHARS {
        return Ok(false);
    }
    let saved = to_saved(found);
    let written = connection.execute(
        "INSERT INTO reply_admissions(record_uuid,session_id,phrases,fact_version)
         SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM records
           WHERE uuid=?1 AND session_id=?2 AND has_conflict=0)
         ON CONFLICT(record_uuid) DO UPDATE SET
           phrases=excluded.phrases,fact_version=excluded.fact_version,conflicted=0
         WHERE reply_admissions.session_id=excluded.session_id
           AND excluded.fact_version>reply_admissions.fact_version",
        params![uuid, session_id, saved, version],
    )?;
    connection.execute(
        "UPDATE reply_admissions SET conflicted=1 WHERE record_uuid=?1 AND session_id=?2
           AND fact_version=?4 AND phrases IS NOT ?3 AND conflicted=0",
        params![uuid, session_id, saved, version],
    )?;
    Ok(written > 0)
}

/// Keep the admission phrases of every accepted agent record of one write
/// that is outside sidechains and has reply text ([`has_reply_text`]), and
/// whose identifier fits the table ([`MAX_RECORD_ID_CHARS`]). An input whose
/// content was not observed states nothing. Returns how many
/// rows were written or replaced.
pub(crate) fn apply(
    connection: &Connection,
    session_id: &str,
    records: &[CanonicalRecord],
    outcomes: &[RecordOutcome],
) -> Result<usize> {
    let mut kept = 0;
    for outcome in outcomes {
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        if !outcome.disposition.is_accepted() {
            continue;
        }
        let input = &records[outcome.input_index];
        if input.record_type != RecordType::Assistant || input.is_sidechain {
            continue;
        }
        let Some(blocks) = &input.message.content else {
            continue;
        };
        let text = record_text::joined(blocks);
        if text.is_empty() {
            continue;
        }
        kept += usize::from(keep(
            connection,
            session_id,
            uuid,
            &phrases(&text),
            FACT_VERSION,
        )?);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(text: &str) -> String {
        to_saved(&phrases(text))
    }

    #[test]
    fn each_phrase_matches_its_own_words() {
        for (text, expected) in [
            ("You're right, the test was flaky.", "you_are_right"),
            ("you are right about that", "you_are_right"),
            ("That was my mistake.", "my_mistake"),
            ("My bad.", "my_bad"),
            ("I was wrong here.", "i_was_wrong"),
            ("that was wrong of me", "that_was_wrong"),
            ("I got that wrong", "i_got_it_wrong"),
            ("I got this wrong", "i_got_it_wrong"),
            ("I got it wrong", "i_got_it_wrong"),
            ("I misread the log", "i_misread"),
            ("I misunderstood you", "i_misunderstood"),
            ("I overstated it", "i_overstated"),
            ("I overclaimed there", "i_overclaimed"),
            ("I should have checked", "i_should_have"),
            ("Good catch!", "good_catch"),
            ("You caught a real bug", "you_caught"),
            ("I explained badly", "i_explained_badly"),
            ("I explained that badly", "i_explained_badly"),
            ("I explained it badly", "i_explained_badly"),
            ("I explained this badly", "i_explained_badly"),
            ("That's on me.", "thats_on_me"),
            ("I made a mistake in the parser", "i_made_a_mistake"),
            ("I was mistaken", "i_was_mistaken"),
            ("Sorry about that", "sorry"),
            ("I'm sorry", "sorry"),
            ("Apologies for the noise", "apology"),
            ("I apologize", "apology"),
            ("my apology", "apology"),
            ("I missed a file", "i_missed"),
            ("I didn't answer your question", "i_didnt_answer"),
            ("my error", "my_error"),
            ("That's my error", "my_error"),
            ("Fair point.", "fair_point"),
            ("I was too quick", "i_was_too"),
            ("I conflated the two", "i_conflated"),
            ("I mixed up the files", "i_mixed_up"),
        ] {
            assert_eq!(names(text), expected, "{text}");
        }
        // Every phrase is covered above.
        let covered: std::collections::BTreeSet<_> = [
            "you_are_right",
            "my_mistake",
            "my_bad",
            "i_was_wrong",
            "that_was_wrong",
            "i_got_it_wrong",
            "i_misread",
            "i_misunderstood",
            "i_overstated",
            "i_overclaimed",
            "i_should_have",
            "good_catch",
            "you_caught",
            "i_explained_badly",
            "thats_on_me",
            "i_made_a_mistake",
            "i_was_mistaken",
            "sorry",
            "apology",
            "i_missed",
            "i_didnt_answer",
            "my_error",
            "fair_point",
            "i_was_too",
            "i_conflated",
            "i_mixed_up",
        ]
        .into_iter()
        .collect();
        let all: std::collections::BTreeSet<_> = Phrase::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(covered, all);
    }

    #[test]
    fn curly_apostrophes_and_upper_case_match() {
        assert_eq!(names("YOU\u{2019}RE RIGHT"), "you_are_right");
        assert_eq!(names("That\u{2019}s On Me"), "thats_on_me");
        assert_eq!(names("I DIDN\u{2019}T ANSWER"), "i_didnt_answer");
        assert_eq!(names("APOLOGIES"), "apology");
    }

    #[test]
    fn wrongly_and_incorrectly_never_match() {
        for text in [
            "I wrongly assumed the cache was warm.",
            "I incorrectly read the flag.",
            "The test was wrongly skipped and incorrectly marked.",
            "wrongly",
            "incorrectly",
        ] {
            assert_eq!(names(text), "", "{text}");
        }
    }

    #[test]
    fn phrases_match_whole_words_only() {
        for text in [
            "my mistakes list",
            "sorrys",
            "unsorry",
            "I missedthe point",
            "good catches",
            "fair points",
            "I was toothless",
            "i-was-wrong",
            "the API was wrong", // "was wrong" without "I" or "that"
            "you are righteous",
            "prefix-apologies",
        ] {
            assert_eq!(names(text), "", "{text}");
        }
        // Words between the phrase's words break it.
        assert_eq!(names("you are not right"), "");
        assert_eq!(names("I really should have"), "");
        // Punctuation between words does not.
        assert_eq!(names("good, catch"), "good_catch");
    }

    #[test]
    fn names_come_out_in_order_of_first_appearance() {
        assert_eq!(
            names("Sorry, you're right: my mistake. Sorry again, you're right."),
            "sorry you_are_right my_mistake"
        );
        assert_eq!(
            names("Good catch. I missed it, sorry"),
            "good_catch i_missed sorry"
        );
        // Not table order: "i_was_too" is listed after "i_was_wrong".
        assert_eq!(
            names("I was too quick; I was wrong"),
            "i_was_too i_was_wrong"
        );
    }

    #[test]
    fn only_the_first_600_characters_count() {
        let pad = |chars: usize| "x".repeat(chars - 1) + " ";
        // A phrase ending exactly at the limit is read.
        let text = format!("{}sorry and more", pad(REPLY_CHARS - 5));
        assert_eq!(names(&text), "sorry");
        // One that crosses the limit is not.
        let text = format!("{}sorry", pad(REPLY_CHARS - 4));
        assert_eq!(names(&text), "");
        // A word the limit cuts is left out, even if its start would match.
        let text = format!("{}sorryful", pad(REPLY_CHARS - 5));
        assert_eq!(names(&text), "");
        // A phrase whose last word the limit cuts is left out.
        let text = format!("{}I was toothless", pad(REPLY_CHARS - 9));
        assert_eq!(names(&text), "");
        // Characters, not bytes: 590 two-byte characters still leave room.
        let text = format!("{} sorry", "é".repeat(REPLY_CHARS - 10));
        assert!(text.len() > REPLY_CHARS);
        assert_eq!(names(&text), "sorry");
        let text = format!("{} sorry", "é".repeat(REPLY_CHARS - 2));
        assert_eq!(names(&text), "");
    }

    #[test]
    fn the_table_is_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for phrase in Phrase::ALL {
            let name = phrase.name();
            assert!(seen.insert(name), "{name} twice");
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name}"
            );
            assert_eq!(Phrase::parse(name), Some(phrase));
            assert!(!phrase.label().is_empty());
            let json = serde_json::to_value(phrase).unwrap();
            assert_eq!(json, json!(name));
            assert_eq!(serde_json::from_value::<Phrase>(json).unwrap(), phrase);
            // Every word of every form is itself one whole word.
            for form in phrase.spec().forms {
                for slot in *form {
                    let words: Vec<&str> = match slot {
                        Is(word) | StartsWith(word) => vec![word],
                        OneOf(options) => options.to_vec(),
                    };
                    for word in words {
                        assert_eq!(crate::words::words(word), vec![word.to_owned()]);
                    }
                }
            }
        }
        assert!(serde_json::from_value::<Phrase>(json!("wrongly")).is_err());
        // Every phrase at once fits the saved column.
        assert!(to_saved(&Phrase::ALL).chars().count() <= MAX_PHRASES_CHARS);
        assert_eq!(
            from_saved(&to_saved(&Phrase::ALL)),
            Some(Phrase::ALL.to_vec())
        );
        assert_eq!(from_saved(""), Some(vec![]));
        assert_eq!(from_saved("sorry wrongly"), None);
    }

    #[test]
    fn a_higher_fact_version_replaces_a_row_and_a_lower_one_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.sqlite");
        let mut store = crate::Store::open(&path).unwrap();
        store
            .upsert_session(
                &crate::SessionMeta::new("s", "claude", crate::SessionSource::Transcript),
                false,
            )
            .unwrap();
        let records: Vec<CanonicalRecord> = vec![
            serde_json::from_value(json!({"uuid":"a1","type":"assistant",
                "timestamp":"2026-09-07T12:00:01Z",
                "message":{"role":"assistant","content":[{"type":"text","text":"Good catch."}]}}))
            .unwrap(),
        ];
        store.upsert_records("s", &records, false).unwrap();
        let connection = Connection::open(&path).unwrap();
        let row = || {
            connection
                .query_row(
                    "SELECT phrases,fact_version,conflicted FROM reply_admissions",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    },
                )
                .unwrap()
        };
        assert_eq!(row(), ("good_catch".into(), FACT_VERSION, false));
        // The same phrases again change nothing.
        assert!(!keep(&connection, "s", "a1", &[Phrase::GoodCatch], FACT_VERSION).unwrap());
        assert_eq!(row(), ("good_catch".into(), FACT_VERSION, false));
        // The same version stating other phrases is a contradiction.
        assert!(!keep(&connection, "s", "a1", &[Phrase::Sorry], FACT_VERSION).unwrap());
        assert_eq!(row(), ("good_catch".into(), FACT_VERSION, true));
        // A later derivation replaces the row and clears the mark.
        assert!(keep(&connection, "s", "a1", &[Phrase::Sorry], FACT_VERSION + 1).unwrap());
        assert_eq!(row(), ("sorry".into(), FACT_VERSION + 1, false));
        // Reading the source again under this build's lower version changes
        // nothing and is no contradiction.
        store.upsert_records("s", &records, false).unwrap();
        assert_eq!(row(), ("sorry".into(), FACT_VERSION + 1, false));
        // Another session cannot write to this record's row.
        assert!(!keep(&connection, "t", "a1", &[], FACT_VERSION + 2).unwrap());
        assert_eq!(row(), ("sorry".into(), FACT_VERSION + 1, false));
    }

    #[test]
    fn reply_text_is_the_joined_text_blocks() {
        assert!(has_reply_text(&[json!({"type":"text","text":"ok"})]));
        assert!(!has_reply_text(&[
            json!({"type":"thinking","thinking":"sorry"}),
            json!({"type":"tool_use","id":"t","name":"Bash","input":{}}),
            json!({"type":"text","text":""}),
        ]));
        assert!(!has_reply_text(&[]));
    }
}
