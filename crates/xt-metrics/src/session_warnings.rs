//! M-21 session warnings: simple rules that say a session probably went
//! wrong somewhere. No model and no score: a warning fires when one rule
//! matches, and each warning stands on its own with a plain-English reason.
//!
//! [`decide`] is the one function that decides a session's warnings. It reads
//! only the evidence handed to it; [`MetricsDb::session_warnings`] gathers that
//! evidence from the store and calls it once per session, and anything that
//! later shows a warning reads its result rather than working one out again.
//!
//! The five rules, and the existing answers each one reads:
//!
//! * **Correction**: a message the stored M-02 classification calls a
//!   person's, sent after an agent reply, holds a correction word ([`correction`]).
//! * **Interruption**: an input the store flagged as Claude Code's
//!   `[Request interrupted by user…]` marker (`records.is_interrupted`), or a
//!   tool result saying the person declined the call ([`DECLINED`]).
//! * **Circling**: an M-09 stretch M-20 judged circling, exactly as
//!   [`crate::repeats`] judges it, with its thresholds. Nothing is recounted.
//! * **Repeated prompt**: the same person's message, compared lowercased with
//!   whitespace collapsed, sent two or more times in the session.
//! * **Repeated failure**: tool results reporting failure (`is_error`) for two
//!   or more calls with the same stored M-20 comparison key.
//!
//! Sidechain records (an agent's own sub-conversation inside the session) are
//! left out of every rule: they are not the person's conversation.
//!
//! An unknown is never a zero. Each rule says whether it read everything it
//! needed ([`CheckState::Complete`]) or which inputs it could not read. A
//! warning found among the inputs it could read is still a warning; an
//! incomplete rule with no warning says nothing about the session.

use crate::{
    MetricsDb, Result, Window, hands_off,
    repeats::{RepeatDensity, RepeatThresholds, SessionRepeats, UnknownRepeats},
    span_detail::{self, PromptText},
};
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::timestamp::{self, InstantKey};

/// One session's records inside the window, with the stored facts every rule
/// reads: who sent each one (M-02), whether only part of a person's record is
/// theirs, the stored interruption flag, the stated tool-result and tool-call
/// presence, and whether content retention kept the record's content.
pub(crate) const RECORD_QUERY: &str = "SELECT v.uuid,v.ts,v.ts_ms,v.type,v.is_sidechain,
       v.human_is_eligible,v.human_text_len IS NOT v.text_len,v.is_interrupted,
       v.is_tool_result_carrier,v.tool_use_count,r.content_json IS NOT NULL
     FROM v_session_events v JOIN records r ON r.uuid=v.uuid
     WHERE v.session_id=?1 AND v.ts_ms>=?2 AND v.ts_ms<?3";

/// A kept record's content blocks.
const CONTENT_QUERY: &str = "SELECT content_json FROM records WHERE uuid=?1";

/// Words that, on their own, mark a person's message as a correction.
pub const CORRECTION_WORDS: [&str; 4] = ["no", "wrong", "stop", "again"];
/// Two words that, in this order, mark a correction.
pub const CORRECTION_PHRASE: [&str; 2] = ["i", "said"];
/// "still" counts only with one of these within [`STILL_REACH`] words of it,
/// before or after: "still not working" is a correction, "I still need an
/// export button" is not.
pub const STILL_NEGATIVES: [&str; 9] = [
    "not",
    "broken",
    "fails",
    "error",
    "same",
    "wrong",
    "doesn't",
    "happening",
    "see",
];
pub const STILL_REACH: usize = 5;
/// The start of the tool result Claude Code writes when the person declines a
/// tool call at its permission prompt.
pub const DECLINED: &str = "The user doesn't want to proceed with this tool use";
/// A repeated prompt or a repeated failure needs at least this many.
pub const REPEAT_AT_LEAST: usize = 2;

/// The five rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    Correction,
    Interruption,
    Circling,
    RepeatedPrompt,
    RepeatedFailure,
}

impl WarningKind {
    pub const ALL: [Self; 5] = [
        Self::Correction,
        Self::Interruption,
        Self::Circling,
        Self::RepeatedPrompt,
        Self::RepeatedFailure,
    ];
}

/// One warning: which rule matched, where, and why in plain words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionWarning {
    pub kind: WarningKind,
    pub session_id: String,
    /// The moment it points at, on the stored millisecond projection: the
    /// message, the interruption, the stretch's start, the first repeat.
    pub at_ms: i64,
    /// The saved records it points at, earliest first.
    pub record_uuids: Vec<String>,
    /// One short sentence. A correction quotes only the matched words; no
    /// other message text is ever part of it.
    pub reason: String,
}

/// Inputs a rule needed and could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "missing", rename_all = "snake_case")]
pub enum Missing {
    /// A person's messages whose words neither content retention nor the
    /// index's short preview kept.
    TextNotKept { messages: u64 },
    /// A person's messages only part of which is theirs (a Codex question
    /// reply or image wrapper); nothing locates their part.
    WrappedText { messages: u64 },
    /// Inputs whose M-02 classification is unknown: any of them may be a
    /// person's message.
    Unclassified { inputs: u64 },
    /// Inputs the store could not flag as an interruption or not.
    InterruptionUnknown { inputs: u64 },
    /// Records carrying tool calls or tool results whose content was not
    /// kept, so failed and declined calls in them cannot be seen.
    ToolResultsNotKept { records: u64 },
    /// Tool results naming a call this session's kept records do not hold.
    UnmatchedResults { results: u64 },
    /// Failed calls whose stored comparison key cannot be compared.
    FailedCallKey { reason: UnknownRepeats, calls: u64 },
    /// M-20 states no stretches for this session: its surface's clock is
    /// excluded, or a classification it needs is unknown.
    CirclingUnmeasured,
    /// M-20 could not count the calls of some stretches.
    RepeatsUnknown {
        reason: UnknownRepeats,
        stretches: u64,
    },
}

/// Whether a rule read every input it needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckState {
    /// Everything was read: no warning of this kind means none happened.
    Complete,
    /// Some inputs could not be read. Warnings found are real; their absence
    /// proves nothing.
    Incomplete { missing: Vec<Missing> },
}

impl CheckState {
    fn of(missing: Vec<Missing>) -> Self {
        if missing.is_empty() {
            Self::Complete
        } else {
            Self::Incomplete { missing }
        }
    }
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Each rule's coverage for one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WarningChecks {
    pub correction: CheckState,
    pub interruption: CheckState,
    pub circling: CheckState,
    pub repeated_prompt: CheckState,
    pub repeated_failure: CheckState,
}

impl WarningChecks {
    pub fn get(&self, kind: WarningKind) -> &CheckState {
        match kind {
            WarningKind::Correction => &self.correction,
            WarningKind::Interruption => &self.interruption,
            WarningKind::Circling => &self.circling,
            WarningKind::RepeatedPrompt => &self.repeated_prompt,
            WarningKind::RepeatedFailure => &self.repeated_failure,
        }
    }
}

/// M-21 for one session: its warnings, earliest first, and each rule's
/// coverage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionWarnings {
    pub session_id: String,
    pub warnings: Vec<SessionWarning>,
    pub checks: WarningChecks,
    /// Messages the stored classification calls a person's, outside
    /// sidechains, and how many of them had words the text rules could read.
    pub person_messages: u64,
    pub person_messages_read: u64,
}

/// One record as the rules read it.
pub(crate) struct EvidenceRecord {
    pub(crate) uuid: String,
    pub(crate) at_ms: i64,
    pub(crate) agent: bool,
    pub(crate) sidechain: bool,
    /// M-02: `Some(true)` a person's message, `None` unknown.
    pub(crate) person: Option<bool>,
    /// A person's message's words; `None` for every other record.
    pub(crate) words: Option<PromptText>,
    pub(crate) interrupted: Option<bool>,
}

/// One tool result read from kept content, with the call it answers.
pub(crate) struct ToolOutcome {
    /// The record carrying the result.
    pub(crate) result_uuid: String,
    pub(crate) at_ms: i64,
    pub(crate) tool_name: String,
    /// The call's M-20 comparison key, or why it cannot be compared.
    pub(crate) key: std::result::Result<String, UnknownRepeats>,
    pub(crate) failed: bool,
    pub(crate) declined: bool,
}

/// Everything [`decide`] reads about one session.
pub(crate) struct Evidence {
    pub(crate) session_id: String,
    /// Chronological: exact instant, then UUID.
    pub(crate) records: Vec<EvidenceRecord>,
    pub(crate) outcomes: Vec<ToolOutcome>,
    /// Records with tool calls or results whose content was not kept.
    pub(crate) tool_records_not_kept: u64,
    pub(crate) unmatched_results: u64,
    pub(crate) repeats: SessionRepeats,
}

/// The words of a message as the text rules compare them: lowercase, curly
/// apostrophes made straight, split on anything that is not a letter, digit,
/// apostrophe or hyphen.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .map(|word| word.trim_matches(|c| c == '\'' || c == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The first correction in a message, as the matched words, or `None`.
pub fn correction(text: &str) -> Option<String> {
    let words = words(text);
    for (index, word) in words.iter().enumerate() {
        if CORRECTION_WORDS.contains(&word.as_str()) {
            return Some(word.clone());
        }
        if word == CORRECTION_PHRASE[0]
            && words.get(index + 1).map(String::as_str) == Some(CORRECTION_PHRASE[1])
        {
            return Some(CORRECTION_PHRASE.join(" "));
        }
        if word == "still" {
            let low = index.saturating_sub(STILL_REACH);
            let high = (index + STILL_REACH).min(words.len() - 1);
            // The nearest negative word, after before before on a tie.
            let near = (1..=STILL_REACH)
                .flat_map(|distance| [index.checked_add(distance), index.checked_sub(distance)])
                .flatten()
                .filter(|other| (low..=high).contains(other))
                .find(|other| STILL_NEGATIVES.contains(&words[*other].as_str()));
            if let Some(other) = near {
                return Some(words[index.min(other)..=index.max(other)].join(" "));
            }
        }
    }
    None
}

/// How a prompt is compared with the session's other prompts.
fn prompt_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn times_word(count: usize) -> String {
    match count {
        2 => "twice".to_owned(),
        n => format!("{n} times"),
    }
}

/// Local clock times, the date added to any not on the first one's day.
fn clock_times(moments: &[i64], zone: &TimeZone) -> String {
    let zoned: Vec<_> = moments
        .iter()
        .filter_map(|ms| Timestamp::from_millisecond(*ms).ok())
        .map(|at| at.to_zoned(zone.clone()))
        .collect();
    let first_day = zoned.first().map(|at| at.date());
    zoned
        .iter()
        .map(|at| {
            if Some(at.date()) == first_day {
                at.strftime("%H:%M").to_string()
            } else {
                at.strftime("%b %-d %H:%M").to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Decide one session's warnings from its evidence. This is M-21: every
/// warning, and every statement about what could not be checked, comes from
/// here.
pub(crate) fn decide(evidence: &Evidence, zone: &TimeZone) -> SessionWarnings {
    let session_id = &evidence.session_id;
    let mut warnings = Vec::new();
    let warn = |kind, at_ms, record_uuids: Vec<String>, reason: String| SessionWarning {
        kind,
        session_id: session_id.clone(),
        at_ms,
        record_uuids,
        reason,
    };

    // Corrections, repeated prompts and interruptions walk the person's own
    // conversation once.
    let mut replied = false;
    let mut unclassified = 0;
    let (mut correction_not_kept, mut correction_wrapped) = (0, 0);
    let (mut prompt_not_kept, mut prompt_wrapped) = (0, 0);
    let mut interruption_unknown = 0;
    let mut prompts = BTreeMap::<String, Vec<&EvidenceRecord>>::new();
    let (mut person_messages, mut person_messages_read) = (0, 0);
    for record in evidence.records.iter().filter(|r| !r.sidechain) {
        if record.agent {
            replied = true;
            continue;
        }
        match record.interrupted {
            Some(true) => warnings.push(warn(
                WarningKind::Interruption,
                record.at_ms,
                vec![record.uuid.clone()],
                format!(
                    "you interrupted the agent ({})",
                    clock_times(&[record.at_ms], zone)
                ),
            )),
            Some(false) => {}
            None => interruption_unknown += 1,
        }
        match record.person {
            Some(true) => {}
            Some(false) => continue,
            None => {
                unclassified += 1;
                continue;
            }
        }
        let after_reply = std::mem::replace(&mut replied, false);
        person_messages += 1;
        match &record.words {
            Some(PromptText::Stored { text, .. }) => {
                person_messages_read += 1;
                if after_reply && let Some(phrase) = correction(text) {
                    warnings.push(warn(
                        WarningKind::Correction,
                        record.at_ms,
                        vec![record.uuid.clone()],
                        format!(
                            "you wrote \"{phrase}\" after a reply ({})",
                            clock_times(&[record.at_ms], zone)
                        ),
                    ));
                }
                let key = prompt_key(text);
                if !key.is_empty() {
                    prompts.entry(key).or_default().push(record);
                }
            }
            Some(PromptText::Wrapped) => {
                correction_wrapped += u64::from(after_reply);
                prompt_wrapped += 1;
            }
            Some(PromptText::NotStored) | None => {
                correction_not_kept += u64::from(after_reply);
                prompt_not_kept += 1;
            }
        }
    }
    for sent in prompts
        .values()
        .filter(|sent| sent.len() >= REPEAT_AT_LEAST)
    {
        let moments: Vec<_> = sent.iter().map(|r| r.at_ms).collect();
        warnings.push(warn(
            WarningKind::RepeatedPrompt,
            sent[1].at_ms,
            sent.iter().map(|r| r.uuid.clone()).collect(),
            format!(
                "same prompt sent {} ({})",
                times_word(sent.len()),
                clock_times(&moments, zone)
            ),
        ));
    }
    let text_missing = |not_kept: u64, wrapped: u64| {
        let mut missing = Vec::new();
        if not_kept > 0 {
            missing.push(Missing::TextNotKept { messages: not_kept });
        }
        if wrapped > 0 {
            missing.push(Missing::WrappedText { messages: wrapped });
        }
        if unclassified > 0 {
            missing.push(Missing::Unclassified {
                inputs: unclassified,
            });
        }
        missing
    };

    // Tool results: declined calls are interruptions, failed ones are grouped
    // by the call's comparison key.
    let mut failures = BTreeMap::<&str, Vec<&ToolOutcome>>::new();
    let mut keyless = BTreeMap::<UnknownRepeats, u64>::new();
    for outcome in &evidence.outcomes {
        if outcome.declined {
            warnings.push(warn(
                WarningKind::Interruption,
                outcome.at_ms,
                vec![outcome.result_uuid.clone()],
                format!(
                    "you declined a call to {} ({})",
                    outcome.tool_name,
                    clock_times(&[outcome.at_ms], zone)
                ),
            ));
        } else if outcome.failed {
            match &outcome.key {
                Ok(key) => failures.entry(key).or_default().push(outcome),
                Err(reason) => *keyless.entry(*reason).or_default() += 1,
            }
        }
    }
    for failed in failures.values().filter(|f| f.len() >= REPEAT_AT_LEAST) {
        let moments: Vec<_> = failed.iter().map(|o| o.at_ms).collect();
        warnings.push(warn(
            WarningKind::RepeatedFailure,
            failed[1].at_ms,
            failed.iter().map(|o| o.result_uuid.clone()).collect(),
            format!(
                "the same {} call failed {} ({})",
                failed[0].tool_name,
                times_word(failed.len()),
                clock_times(&moments, zone)
            ),
        ));
    }
    let mut tool_missing = Vec::new();
    if evidence.tool_records_not_kept > 0 {
        tool_missing.push(Missing::ToolResultsNotKept {
            records: evidence.tool_records_not_kept,
        });
    }
    if evidence.unmatched_results > 0 {
        tool_missing.push(Missing::UnmatchedResults {
            results: evidence.unmatched_results,
        });
    }
    let mut failure_missing = tool_missing.clone();
    failure_missing.extend(
        keyless
            .into_iter()
            .map(|(reason, calls)| Missing::FailedCallKey { reason, calls }),
    );
    let mut interruption_missing = Vec::new();
    if interruption_unknown > 0 {
        interruption_missing.push(Missing::InterruptionUnknown {
            inputs: interruption_unknown,
        });
    }
    interruption_missing.extend(tool_missing);

    // Circling is M-20's own verdict, stretch by stretch.
    let at = |uuid: &str| {
        evidence
            .records
            .iter()
            .find(|r| r.uuid == uuid)
            .map(|r| r.at_ms)
    };
    let mut circling_missing = Vec::new();
    match &evidence.repeats {
        SessionRepeats::Missing | SessionRepeats::Unmeasured { .. } => {
            circling_missing.push(Missing::CirclingUnmeasured)
        }
        SessionRepeats::Measured { stretches, .. } => {
            let mut unknown = BTreeMap::<UnknownRepeats, u64>::new();
            for stretch in stretches {
                match (&stretch.repeats, stretch.circling) {
                    (RepeatDensity::Unknown { reason }, _) => {
                        *unknown.entry(*reason).or_default() += 1
                    }
                    (RepeatDensity::Measured { repeats, worst }, Some(true)) => {
                        let mut uuids = BTreeSet::new();
                        uuids.insert(stretch.start_uuid.clone());
                        let worst = worst.as_ref().map(|group| {
                            uuids.insert(group.representative.record_uuid.clone());
                            format!(
                                ", most often the same {} call ({})",
                                group.tool_name,
                                times_word(group.count as usize)
                            )
                        });
                        uuids.insert(stretch.end_uuid.clone());
                        let start_ms = at(&stretch.start_uuid).unwrap_or_default();
                        let mut record_uuids: Vec<_> = uuids.into_iter().collect();
                        record_uuids.sort_by_key(|uuid| at(uuid));
                        warnings.push(warn(
                            WarningKind::Circling,
                            start_ms,
                            record_uuids,
                            format!(
                                "went in circles: {repeats} repeated calls in {} active minutes{} (from {})",
                                stretch.active_duration_ms / 60_000,
                                worst.unwrap_or_default(),
                                clock_times(&[start_ms], zone)
                            ),
                        ));
                    }
                    (RepeatDensity::Measured { .. }, _) => {}
                }
            }
            circling_missing.extend(
                unknown
                    .into_iter()
                    .map(|(reason, stretches)| Missing::RepeatsUnknown { reason, stretches }),
            );
        }
    }

    warnings.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.kind.cmp(&b.kind)));
    SessionWarnings {
        session_id: session_id.clone(),
        warnings,
        checks: WarningChecks {
            correction: CheckState::of(text_missing(correction_not_kept, correction_wrapped)),
            interruption: CheckState::of(interruption_missing),
            circling: CheckState::of(circling_missing),
            repeated_prompt: CheckState::of(text_missing(prompt_not_kept, prompt_wrapped)),
            repeated_failure: CheckState::of(failure_missing),
        },
        person_messages,
        person_messages_read,
    }
}

/// The text of a tool result's `content`, which the host writes as a string,
/// as a list of parts, or not at all.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

impl MetricsDb {
    /// M-21 for every user session with an event inside the window, ordered
    /// by session id. Every rule reads only the session's records inside the
    /// window; circling is M-20 over that same window. `zone` only writes the
    /// clock times in each reason.
    ///
    /// Read inside one snapshot, so every session's rules describe the same
    /// committed state.
    pub fn session_warnings(
        &self,
        window: Window,
        thresholds: RepeatThresholds,
        zone: &TimeZone,
    ) -> Result<Vec<SessionWarnings>> {
        self.read_snapshot(|db| {
            let surfaces = hands_off::load(&db.connection, window)?;
            let mut all = Vec::new();
            for ((host, surface), sessions) in &surfaces {
                // M-09's health verdict is about the surface, so it is judged
                // once for all of its sessions, as M-20 judges it for each.
                let excluded = hands_off::health(host, surface, sessions);
                for (session_id, events) in sessions {
                    let repeats = db.loaded_repeats(
                        excluded.clone(),
                        Some(events.as_slice()),
                        session_id,
                        thresholds,
                    )?;
                    let evidence = db.warning_evidence(window, session_id, repeats)?;
                    all.push(decide(&evidence, zone));
                }
            }
            all.sort_by(|a, b| a.session_id.cmp(&b.session_id));
            Ok(all)
        })
    }

    fn warning_evidence(
        &self,
        window: Window,
        session_id: &str,
        repeats: SessionRepeats,
    ) -> Result<Evidence> {
        struct Row {
            instant: InstantKey,
            record: EvidenceRecord,
            partial: bool,
            tool_bearing: bool,
            kept: bool,
        }
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut rows = Vec::new();
        let mut statement = self.connection.prepare(RECORD_QUERY)?;
        let mut result = statement.query(rusqlite::params![
            session_id,
            window.start_ms(),
            window.candidate_end_ms()?
        ])?;
        while let Some(row) = result.next()? {
            let raw: String = row.get(1)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let agent = row.get::<_, String>(3)? == "assistant";
            let carrier: Option<bool> = row.get(8)?;
            let tool_calls: Option<i64> = row.get(9)?;
            rows.push(Row {
                instant,
                record: EvidenceRecord {
                    uuid: row.get(0)?,
                    at_ms: row.get(2)?,
                    agent,
                    sidechain: row.get(4)?,
                    person: if agent { Some(false) } else { row.get(5)? },
                    words: None,
                    interrupted: if agent { Some(false) } else { row.get(7)? },
                },
                partial: row.get(6)?,
                // A record that may carry a call or a result: unknown counts.
                tool_bearing: if agent {
                    tool_calls != Some(0)
                } else {
                    carrier != Some(false)
                },
                kept: row.get(10)?,
            });
        }
        drop(result);
        drop(statement);
        rows.sort_by(|a, b| {
            a.instant
                .cmp(&b.instant)
                .then_with(|| a.record.uuid.cmp(&b.record.uuid))
        });

        // A person's words, read the one way every reader reads them.
        for row in &mut rows {
            if row.record.person == Some(true) && !row.record.sidechain {
                row.record.words = Some(span_detail::person_words(
                    &self.connection,
                    &row.record.uuid,
                    row.partial,
                )?);
            }
        }

        // Tool calls and results, only from content retention kept. A call is
        // named by its `id` at its block position; a result by `tool_use_id`.
        let mut tool_records_not_kept = 0;
        let mut calls = BTreeMap::<String, (String, u64)>::new();
        let mut results = Vec::new();
        for row in rows
            .iter()
            .filter(|row| !row.record.sidechain && row.tool_bearing)
        {
            if !row.kept {
                tool_records_not_kept += 1;
                continue;
            }
            let Some(blocks) = self.content_blocks(&row.record.uuid)? else {
                tool_records_not_kept += 1;
                continue;
            };
            for (index, block) in blocks.iter().enumerate() {
                let kind = block.get("type").and_then(Value::as_str);
                if row.record.agent && kind == Some("tool_use") {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        calls.insert(id.to_owned(), (row.record.uuid.clone(), index as u64));
                    }
                } else if !row.record.agent && kind == Some("tool_result") {
                    // The same reading the session view uses: failed only
                    // when the result says so.
                    let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
                    let declined = failed && result_text(block).starts_with(DECLINED);
                    results.push((
                        row.record.uuid.clone(),
                        row.record.at_ms,
                        block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        failed,
                        declined,
                    ));
                }
            }
        }
        let stored = if results.iter().any(|result| result.3) {
            self.stored_calls(session_id)?
        } else {
            BTreeMap::new()
        };
        let mut outcomes = Vec::new();
        let mut unmatched_results = 0;
        for (result_uuid, at_ms, call_id, failed, declined) in results {
            if !failed {
                continue;
            }
            let call = call_id
                .and_then(|id| calls.get(&id))
                .and_then(|(uuid, index)| {
                    stored
                        .get(uuid)?
                        .iter()
                        .find(|call| call.block_index == *index)
                });
            let Some(call) = call else {
                unmatched_results += 1;
                continue;
            };
            outcomes.push(ToolOutcome {
                result_uuid,
                at_ms,
                tool_name: call.name.clone(),
                key: call.comparable_key().map(str::to_owned),
                failed,
                declined,
            });
        }

        Ok(Evidence {
            session_id: session_id.to_owned(),
            records: rows.into_iter().map(|row| row.record).collect(),
            outcomes,
            tool_records_not_kept,
            unmatched_results,
            repeats,
        })
    }

    /// A kept record's content blocks; `None` when none were kept or the kept
    /// content is not a block list.
    fn content_blocks(&self, uuid: &str) -> Result<Option<Vec<Value>>> {
        let json: Option<Option<String>> = self
            .connection
            .query_row(CONTENT_QUERY, [uuid], |row| row.get(0))
            .optional()?;
        Ok(match json.flatten() {
            // The column only holds valid JSON (its CHECK); anything that is
            // not a block list has no calls or results this read can name.
            Some(json) => match serde_json::from_str::<Value>(&json) {
                Ok(Value::Array(blocks)) => Some(blocks),
                _ => None,
            },
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repeats::{RepeatGroup, StretchRepeats},
        session_stretches::ToolBlock,
    };

    const T0: i64 = 1_790_000_000_000; // 2026-09-21T14:13:20Z
    const MINUTE: i64 = 60_000;

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    fn agent(uuid: &str, minute: i64) -> EvidenceRecord {
        EvidenceRecord {
            uuid: uuid.into(),
            at_ms: T0 + minute * MINUTE,
            agent: true,
            sidechain: false,
            person: Some(false),
            words: None,
            interrupted: Some(false),
        }
    }
    fn person(uuid: &str, minute: i64, text: &str) -> EvidenceRecord {
        EvidenceRecord {
            uuid: uuid.into(),
            at_ms: T0 + minute * MINUTE,
            agent: false,
            sidechain: false,
            person: Some(true),
            words: Some(span_detail::prompt_excerpt(text)),
            interrupted: Some(false),
        }
    }
    /// An input the classification calls automatic: a hook, a tool result, a
    /// task notification. Its words are never read.
    fn automatic(uuid: &str, minute: i64) -> EvidenceRecord {
        EvidenceRecord {
            person: Some(false),
            words: None,
            ..person(uuid, minute, "")
        }
    }
    fn evidence(records: Vec<EvidenceRecord>) -> Evidence {
        Evidence {
            session_id: "s".into(),
            records,
            outcomes: vec![],
            tool_records_not_kept: 0,
            unmatched_results: 0,
            repeats: SessionRepeats::Measured {
                thresholds: RepeatThresholds::default(),
                stretches: vec![],
            },
        }
    }
    fn of_kind(warnings: &SessionWarnings, kind: WarningKind) -> Vec<&SessionWarning> {
        warnings
            .warnings
            .iter()
            .filter(|w| w.kind == kind)
            .collect()
    }

    #[test]
    fn still_counts_only_beside_a_negative_word() {
        assert_eq!(correction("I still need an export button"), None);
        assert_eq!(correction("still not working"), Some("still not".into()));
        assert_eq!(
            correction("It\u{2019}s still broken"),
            Some("still broken".into())
        );
        assert_eq!(
            correction("the same thing is still happening"),
            Some("still happening".into())
        );
        assert_eq!(
            correction("I still don't see it"),
            Some("still don't see".into())
        );
        // A negative word six words away is out of reach.
        assert_eq!(correction("still a b c d e broken"), None);
        assert_eq!(
            correction("still a b c d broken"),
            Some("still a b c d broken".into())
        );
    }

    #[test]
    fn correction_words_match_whole_words_only() {
        assert_eq!(correction("No, use the other file"), Some("no".into()));
        assert_eq!(correction("that's WRONG"), Some("wrong".into()));
        assert_eq!(correction("Please stop."), Some("stop".into()));
        assert_eq!(correction("try again"), Some("again".into()));
        assert_eq!(correction("I said the left panel"), Some("i said".into()));
        for none in [
            "notice the nothing",
            "a no-op change",
            "stopwatch",
            "she said yes",
            "add an export button",
        ] {
            assert_eq!(correction(none), None, "{none}");
        }
    }

    #[test]
    fn a_correction_needs_an_agent_reply_before_it() {
        let found = decide(
            &evidence(vec![
                person("p1", 0, "no tests yet, start with the parser"),
                agent("a1", 1),
                person("p2", 2, "still not working"),
                person("p3", 3, "wrong file"),
            ]),
            &utc(),
        );
        let corrections = of_kind(&found, WarningKind::Correction);
        // The opening message and one sent before any new reply do not count.
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].record_uuids, vec!["p2".to_owned()]);
        assert_eq!(corrections[0].at_ms, T0 + 2 * MINUTE);
        assert!(corrections[0].reason.contains("\"still not\""));
        assert_eq!(
            corrections[0].reason,
            "you wrote \"still not\" after a reply (14:15)"
        );
        assert!(found.checks.correction.is_complete());
    }

    #[test]
    fn same_prompt_twice_is_compared_without_case_or_spacing() {
        let found = decide(
            &evidence(vec![
                person("p1", 5, "Run the  tests"),
                agent("a1", 6),
                automatic("hook", 6),
                person("p2", 12, "run the tests\n"),
                agent("a2", 13),
                person("p3", 14, "run the tests please"),
            ]),
            &utc(),
        );
        let repeated = of_kind(&found, WarningKind::RepeatedPrompt);
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].record_uuids, vec!["p1".to_owned(), "p2".into()]);
        assert_eq!(repeated[0].at_ms, T0 + 12 * MINUTE);
        assert!(repeated[0].reason.starts_with("same prompt sent twice ("));
        assert!(found.checks.repeated_prompt.is_complete());
    }

    #[test]
    fn automatic_messages_never_count_as_prompts_or_corrections() {
        // Two identical hook outputs and a tool result reading "wrong": the
        // stored classification says no person sent them, so neither rule
        // reads them, whatever they would have said.
        let mut hook = automatic("h1", 1);
        hook.words = Some(span_detail::prompt_excerpt("wrong"));
        let found = decide(
            &evidence(vec![
                person("p1", 0, "build it"),
                agent("a1", 1),
                hook,
                automatic("h2", 2),
                automatic("h3", 3),
                agent("a2", 4),
            ]),
            &utc(),
        );
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert!(found.checks.correction.is_complete());
        assert!(found.checks.repeated_prompt.is_complete());
    }

    #[test]
    fn unkept_words_and_unknown_senders_leave_text_rules_incomplete() {
        let mut unkept = person("p2", 2, "");
        unkept.words = Some(PromptText::NotStored);
        let mut wrapped = person("p3", 4, "");
        wrapped.words = Some(PromptText::Wrapped);
        let mut unknown = person("u", 5, "");
        unknown.person = None;
        unknown.words = None;
        let found = decide(
            &evidence(vec![
                person("p1", 0, "start"),
                agent("a1", 1),
                unkept,
                agent("a2", 3),
                wrapped,
                unknown,
            ]),
            &utc(),
        );
        assert!(found.warnings.is_empty());
        assert_eq!(
            found.checks.correction,
            CheckState::Incomplete {
                missing: vec![
                    Missing::TextNotKept { messages: 1 },
                    Missing::WrappedText { messages: 1 },
                    Missing::Unclassified { inputs: 1 },
                ]
            }
        );
        // The opening message was read; the unkept one still counts against
        // the repeated-prompt rule, which needs every prompt.
        assert_eq!(
            found.checks.repeated_prompt,
            CheckState::Incomplete {
                missing: vec![
                    Missing::TextNotKept { messages: 1 },
                    Missing::WrappedText { messages: 1 },
                    Missing::Unclassified { inputs: 1 },
                ]
            }
        );
    }

    #[test]
    fn interruptions_come_from_the_stored_flag_and_declined_calls() {
        let mut stop = automatic("i1", 3);
        stop.interrupted = Some(true);
        let mut side = automatic("i2", 4);
        side.interrupted = Some(true);
        side.sidechain = true;
        let mut unknown = automatic("i3", 5);
        unknown.interrupted = None;
        let mut evidence = evidence(vec![
            person("p1", 0, "go"),
            agent("a1", 1),
            stop,
            side,
            unknown,
        ]);
        evidence.outcomes.push(ToolOutcome {
            result_uuid: "r1".into(),
            at_ms: T0 + 2 * MINUTE,
            tool_name: "Bash".into(),
            key: Ok("k".into()),
            failed: true,
            declined: true,
        });
        let found = decide(&evidence, &utc());
        let interruptions = of_kind(&found, WarningKind::Interruption);
        assert_eq!(interruptions.len(), 2, "{interruptions:?}");
        assert_eq!(
            interruptions[0].reason,
            "you declined a call to Bash (14:15)"
        );
        assert_eq!(interruptions[1].record_uuids, vec!["i1".to_owned()]);
        assert!(
            interruptions[1]
                .reason
                .starts_with("you interrupted the agent")
        );
        // A declined call is not a failed command.
        assert!(of_kind(&found, WarningKind::RepeatedFailure).is_empty());
        assert_eq!(
            found.checks.interruption,
            CheckState::Incomplete {
                missing: vec![Missing::InterruptionUnknown { inputs: 1 }]
            }
        );
    }

    fn failure(uuid: &str, minute: i64, tool: &str, key: &str) -> ToolOutcome {
        ToolOutcome {
            result_uuid: uuid.into(),
            at_ms: T0 + minute * MINUTE,
            tool_name: tool.into(),
            key: Ok(key.into()),
            failed: true,
            declined: false,
        }
    }

    #[test]
    fn the_same_call_failing_twice_is_one_warning() {
        let mut evidence = evidence(vec![person("p1", 0, "fix it"), agent("a1", 1)]);
        evidence.outcomes = vec![
            failure("r1", 2, "Bash", "pnpm-test"),
            failure("r2", 3, "Bash", "cargo-build"),
            failure("r3", 4, "Bash", "pnpm-test"),
            failure("r4", 5, "Bash", "pnpm-test"),
            ToolOutcome {
                key: Err(UnknownRepeats::MissingKey),
                ..failure("r5", 6, "Bash", "")
            },
        ];
        let found = decide(&evidence, &utc());
        let failed = of_kind(&found, WarningKind::RepeatedFailure);
        assert_eq!(failed.len(), 1);
        assert_eq!(
            failed[0].record_uuids,
            vec!["r1".to_owned(), "r3".into(), "r4".into()]
        );
        assert!(
            failed[0]
                .reason
                .starts_with("the same Bash call failed 3 times"),
            "{}",
            failed[0].reason
        );
        assert_eq!(
            found.checks.repeated_failure,
            CheckState::Incomplete {
                missing: vec![Missing::FailedCallKey {
                    reason: UnknownRepeats::MissingKey,
                    calls: 1
                }]
            }
        );
    }

    #[test]
    fn unkept_tool_results_leave_failure_and_decline_checks_incomplete() {
        let mut evidence = evidence(vec![person("p1", 0, "fix it"), agent("a1", 1)]);
        evidence.tool_records_not_kept = 2;
        let found = decide(&evidence, &utc());
        let missing = CheckState::Incomplete {
            missing: vec![Missing::ToolResultsNotKept { records: 2 }],
        };
        assert_eq!(found.checks.repeated_failure, missing);
        assert_eq!(found.checks.interruption, missing);
        assert!(found.checks.correction.is_complete());
    }

    fn stretch(
        start: &str,
        end: &str,
        repeats: RepeatDensity,
        circling: Option<bool>,
    ) -> StretchRepeats {
        StretchRepeats {
            start_uuid: start.into(),
            end_uuid: end.into(),
            start: String::new(),
            end: String::new(),
            duration_ms: 600_000,
            active_duration_ms: 540_000,
            repeats,
            circling,
        }
    }

    #[test]
    fn circling_is_m20_s_own_verdict() {
        let mut evidence = evidence(vec![
            person("p1", 0, "go"),
            agent("a1", 1),
            agent("a2", 9),
            person("p2", 20, "next"),
            agent("a3", 21),
        ]);
        evidence.repeats = SessionRepeats::Measured {
            thresholds: RepeatThresholds::default(),
            stretches: vec![
                stretch(
                    "p1",
                    "a2",
                    RepeatDensity::Measured {
                        repeats: 6,
                        worst: Some(RepeatGroup {
                            tool_name: "Bash".into(),
                            count: 5,
                            representative: ToolBlock {
                                record_uuid: "a1".into(),
                                block_index: 0,
                            },
                        }),
                    },
                    Some(true),
                ),
                // Measured and not circling: no warning, nothing missing.
                stretch(
                    "p2",
                    "a3",
                    RepeatDensity::Measured {
                        repeats: 1,
                        worst: None,
                    },
                    Some(false),
                ),
            ],
        };
        let found = decide(&evidence, &utc());
        let circling = of_kind(&found, WarningKind::Circling);
        assert_eq!(circling.len(), 1);
        assert_eq!(
            circling[0].record_uuids,
            vec!["p1".to_owned(), "a1".into(), "a2".into()]
        );
        assert_eq!(circling[0].at_ms, T0);
        assert!(
            circling[0]
                .reason
                .starts_with("went in circles: 6 repeated calls in 9 active minutes, most often the same Bash call (5 times)"),
            "{}",
            circling[0].reason
        );
        assert!(found.checks.circling.is_complete());

        // An uncounted stretch and an unmeasured session are unknowns.
        evidence.repeats = SessionRepeats::Measured {
            thresholds: RepeatThresholds::default(),
            stretches: vec![stretch(
                "p1",
                "a2",
                RepeatDensity::Unknown {
                    reason: UnknownRepeats::MissingKey,
                },
                None,
            )],
        };
        assert_eq!(
            decide(&evidence, &utc()).checks.circling,
            CheckState::Incomplete {
                missing: vec![Missing::RepeatsUnknown {
                    reason: UnknownRepeats::MissingKey,
                    stretches: 1
                }]
            }
        );
        evidence.repeats = SessionRepeats::Unmeasured {
            excluded_surface: None,
        };
        assert_eq!(
            decide(&evidence, &utc()).checks.circling,
            CheckState::Incomplete {
                missing: vec![Missing::CirclingUnmeasured]
            }
        );
    }

    #[test]
    fn times_on_another_day_carry_their_date() {
        let text = clock_times(&[T0, T0 + 5 * MINUTE, T0 + 24 * 60 * MINUTE], &utc());
        assert_eq!(text, "14:13, 14:18, Sep 22 14:13");
    }

    #[test]
    fn the_evidence_read_touches_no_argument() {
        for forbidden in ["input_json", "command", "file_path"] {
            assert!(!RECORD_QUERY.contains(forbidden), "{forbidden}");
        }
    }
}
