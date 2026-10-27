//! Generated IPC shapes for M-21 session warnings, and every word a screen
//! shows about them.
//!
//! Which warnings a session has, and whether each check could read everything
//! it needed, is `xt_metrics::session_warnings::decide`'s answer over the
//! session's whole history ([`xt_metrics::MetricsDb::whole_session_warnings`]).
//! This module only names those answers: each kind's label, each check's
//! label, why a check could not read everything, the row's count and the
//! detail's summary are written here once, so the Dashboard's session rows,
//! the Effort card's day rows, the Sessions list and a session's own page all
//! say the same thing about the same session. No screen works out a warning,
//! a count or a sentence of its own.
//!
//! A warning the user dismissed (`dismissed_warnings`, migration 28) is left
//! out of every mark, count and day here, and listed apart on its session's
//! page so it can be undone. A dismissal is keyed by the warning's identity
//! (`kind:anchor`), which a growing session keeps, and stores the warning's
//! size then ([`SessionWarning::size`]): a dismissed repeated prompt sent
//! once more, a repeated failure that fails again, or a repeated-calls
//! stretch with more repeats has grown past it, so it shows again everywhere
//! and its page says so (`returned`). A correction or an interruption is one
//! event and stays dismissed.
use crate::state::StateError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;
use xt_metrics::{
    CheckState, Dismissal, Missing, SessionWarning, WarningKind, WholeSessionWarnings,
};

/// Sessions one request names at most: a Sessions page (51), or the session
/// rows a Dashboard view has in its scroll area.
pub const MAX_WARNING_SESSIONS: usize = xt_metrics::MAX_SESSIONS;

/// Every fixed sentence a screen shows about warnings that no answer words
/// (while one is read, when a read fails, before a lane's axis) and the
/// request limit. Written once here and exported beside the generated types
/// as `SessionWarningsText.ts` ([`export_text`]), so the UI's batches and
/// notes say exactly what the native side means.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct WarningsText {
    /// Distinct sessions one `session_warnings` request may name.
    pub max_sessions: usize,
    /// A Dashboard session row's marker for warnings before its activity
    /// axis; the UI puts the axis length in for `{hours}`.
    pub before_axis_one: &'static str,
    /// The same with `{count}` warnings.
    pub before_axis_many: &'static str,
    /// The Effort card's note when its warnings could not be read, so a failed
    /// read never looks like a report with none.
    pub window_unread: &'static str,
    /// A session page's Warnings card: the read failed.
    pub session_unread: &'static str,
    /// …the answer is being read.
    pub session_reading: &'static str,
    /// …this data source cannot answer.
    pub session_unavailable: &'static str,
    /// …no indexed session has this identifier.
    pub session_missing: &'static str,
    /// The card's subtitle: what its answer covers.
    pub session_scope: &'static str,
    /// A warning's button on its session's page that dismisses it.
    pub dismiss: &'static str,
    /// A dismissed warning's button that undoes the dismissal.
    pub restore: &'static str,
    /// The heading over a session's dismissed warnings.
    pub dismissed_heading: &'static str,
    /// Said when a dismissal could not be saved.
    pub dismiss_failed: &'static str,
    /// Said when a dismissal could not be undone.
    pub restore_failed: &'static str,
    /// Under a mark's warning: what opening the mark (a click, or Enter
    /// with it focused) does.
    pub open_hint: &'static str,
    /// Said before a Diagnose click: which messages will be sent, and to
    /// which tool (`session_diagnosis_dto::BEFORE_SENDING`).
    pub diagnose_before_sending: &'static str,
    /// Every other word the Diagnose controls show.
    pub diagnose: crate::session_diagnosis_dto::DiagnoseText,
    /// The heading of the Dashboard's window for one warning.
    pub modal_heading: &'static str,
    /// Over the messages around a warning in that window.
    pub conversation_heading: &'static str,
    /// Who sent a message saved under the user role, in that conversation.
    pub you: &'static str,
    /// Who sent a message saved under the assistant role.
    pub agent: &'static str,
    /// Over the reason in that window.
    pub flagged_label: &'static str,
    /// A short heading over the reason, by the warning's kind.
    pub flagged_headings: KindWords,
    /// Under the reason: what the reader can do next.
    pub flagged_next: &'static str,
    /// What the flagged message itself is called on its card, by kind: one
    /// message, so one call or one prompt.
    pub message_tags: KindWords,
}

/// One phrase for each warning kind, keyed as `SessionWarningKind` is
/// serialized, so a screen looks one up by the warning's `kind`.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct KindWords {
    pub correction: &'static str,
    pub interruption: &'static str,
    pub repeated_calls: &'static str,
    pub repeated_prompt: &'static str,
    pub repeated_failure: &'static str,
}

pub const TEXT: WarningsText = WarningsText {
    max_sessions: MAX_WARNING_SESSIONS,
    before_axis_one: "1 warning before the last {hours} hours",
    before_axis_many: "{count} warnings before the last {hours} hours",
    window_unread: "Warnings could not be read.",
    session_unread: "This session’s warnings could not be read.",
    session_reading: "Reading this session’s warnings…",
    session_unavailable: "Warnings are not available here.",
    session_missing: "This Mac’s index holds no session with this identifier.",
    session_scope: "Over the whole session, whatever the range",
    dismiss: "Dismiss",
    restore: "Undo",
    dismissed_heading: "Dismissed",
    dismiss_failed: "This warning could not be dismissed. Nothing was changed.",
    restore_failed: "This warning could not be brought back. Nothing was changed.",
    open_hint: "Open to see it on the session page",
    diagnose_before_sending: crate::session_diagnosis_dto::BEFORE_SENDING,
    diagnose: crate::session_diagnosis_dto::DIAGNOSE_TEXT,
    modal_heading: "Session warning",
    conversation_heading: "Conversation",
    you: "You",
    agent: "Agent",
    flagged_label: "Why this was flagged",
    flagged_headings: KindWords {
        correction: "You corrected the agent.",
        interruption: "You stopped the agent.",
        repeated_calls: "Another attempt. The same inputs.",
        repeated_prompt: "The same request, sent again.",
        repeated_failure: "Another attempt. The same failure.",
    },
    flagged_next: "See the conversation around it, then take a closer look.",
    message_tags: KindWords {
        correction: "Correction",
        interruption: "Interruption",
        repeated_calls: "Repeated call",
        repeated_prompt: "Repeated prompt",
        repeated_failure: "Repeated failure",
    },
};

/// Writes [`TEXT`] as `SessionWarningsText.ts` into the generated directory.
pub fn export_text(directory: &std::path::Path) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(&TEXT).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(directory)?;
    std::fs::write(
        directory.join("SessionWarningsText.ts"),
        format!(
            "// Generated by `cargo xtask dto-export` from `TEXT` in \
             apps/desktop/src-tauri/src/session_warnings_dto.rs. Do not edit this file manually.\n\n\
             /** Fixed warning sentences and the request limit, as the native side states them. */\n\
             export const SESSION_WARNINGS_TEXT = {json} as const;\n"
        ),
    )
}

/// The five checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionWarningKind {
    Correction,
    Interruption,
    /// M-20's circling verdict over the calls that are not expected repeats,
    /// named so it is never read as the Dashboard's own M-20 number.
    RepeatedCalls,
    RepeatedPrompt,
    RepeatedFailure,
}

impl From<WarningKind> for SessionWarningKind {
    fn from(kind: WarningKind) -> Self {
        match kind {
            WarningKind::Correction => Self::Correction,
            WarningKind::Interruption => Self::Interruption,
            WarningKind::RepeatedCalls => Self::RepeatedCalls,
            WarningKind::RepeatedPrompt => Self::RepeatedPrompt,
            WarningKind::RepeatedFailure => Self::RepeatedFailure,
        }
    }
}

/// A warning's kind as a screen names it.
fn kind_label(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::Correction => "Correction",
        WarningKind::Interruption => "Interruption",
        WarningKind::RepeatedCalls => "Repeated calls",
        WarningKind::RepeatedPrompt => "Repeated prompt",
        WarningKind::RepeatedFailure => "Repeated failure",
    }
}

/// A check as a sentence about it names it.
fn check_label(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::Correction => "corrections",
        WarningKind::Interruption => "interruptions",
        WarningKind::RepeatedCalls => "repeated calls",
        WarningKind::RepeatedPrompt => "repeated prompts",
        WarningKind::RepeatedFailure => "repeated failures",
    }
}

/// One warning, as every screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningItem {
    /// The warning's identity inside its session, kept while the session
    /// grows: `kind:anchor`, where the anchor is the saved record the warning
    /// starts at (for a tool result, that record and the result's position
    /// in it, `record#block`). See `xt_metrics::SessionWarning::anchor`.
    pub id: String,
    pub kind: SessionWarningKind,
    /// The kind in words: "Correction", "Repeated calls", …
    pub label: String,
    /// The moment it points at, UTC milliseconds: where a dot is drawn.
    #[ts(type = "number")]
    pub at_ms: i64,
    /// That moment's day in the requested zone ("Sep 4"), for a list that
    /// spans days: a reason's clock times carry no date of their own.
    pub day: String,
    /// Why, in one sentence, with clock times in the requested zone. Quotes
    /// at most the few words a correction matched.
    pub reason: String,
    /// A short native summary for the issue navigator; full reason stays available.
    pub compact_reason: String,
    /// What a mark for this warning alone is named: "Warning. Sep 4,
    /// Correction: …", its day, label and reason.
    pub name: String,
    /// The saved records it points at, earliest first.
    pub record_uuids: Vec<String>,
    /// The one message it points at (`xt_metrics::SessionWarning::points_at`):
    /// the message a mark opens the session at and Diagnose flags.
    pub points_at: SessionWarningPoint,
    /// Set when the user dismissed it and it has grown since, so it shows
    /// again: "Shown again: you dismissed it at 2 failures; the call has now
    /// failed 3 times." `null` otherwise.
    pub returned: Option<String>,
}

/// A saved record, and for a tool result that result's block in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningPoint {
    pub record_uuid: String,
    pub block: Option<u32>,
}

impl SessionWarningItem {
    /// `dismissal` is the warning's stored dismissal, if any.
    fn from_core(
        warning: SessionWarning,
        dismissal: Option<&Dismissal>,
        zone: &jiff::tz::TimeZone,
    ) -> Self {
        let label = kind_label(warning.kind);
        let point = warning.points_at();
        let returned = dismissal.and_then(|dismissal| returned_text(&warning, dismissal));
        let day = jiff::Timestamp::from_millisecond(warning.at_ms)
            .map(|at| at.to_zoned(zone.clone()).strftime("%b %-d").to_string())
            .unwrap_or_default();
        Self {
            id: warning.id(),
            kind: warning.kind.into(),
            label: label.to_owned(),
            name: format!("Warning. {day}, {label}: {}", warning.reason),
            day,
            at_ms: warning.at_ms,
            reason: warning.reason,
            compact_reason: warning.compact_reason,
            record_uuids: warning.record_uuids,
            points_at: SessionWarningPoint {
                record_uuid: point.record_uuid,
                block: point.block,
            },
            returned,
        }
    }
}

/// How far a dismissed warning has grown past the size it was dismissed at,
/// as `(then, now)`; `None` when it has not (or is one event, which cannot).
fn grown(warning: &SessionWarning, dismissal: &Dismissal) -> Option<(u64, u64)> {
    match (dismissal.size, warning.size) {
        (Some(then), Some(now)) if now > then => Some((then, now)),
        _ => None,
    }
}

/// Said on a warning that shows again because it grew after it was dismissed.
fn returned_text(warning: &SessionWarning, dismissal: &Dismissal) -> Option<String> {
    let (then, now) = grown(warning, dismissal)?;
    Some(match warning.kind {
        WarningKind::RepeatedPrompt => format!(
            "Shown again: you dismissed it when the prompt had been sent {then} times; it has now been sent {now} times."
        ),
        WarningKind::RepeatedFailure => format!(
            "Shown again: you dismissed it at {then} failures; the call has now failed {now} times."
        ),
        WarningKind::RepeatedCalls => {
            format!("Shown again: you dismissed it at {then} repeated calls; there are now {now}.")
        }
        WarningKind::Correction | WarningKind::Interruption => return None,
    })
}

/// Whether one check read everything it needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningCheck {
    pub kind: SessionWarningKind,
    /// `false` when some inputs could not be read: a warning it found is
    /// real, and finding none says nothing.
    pub complete: bool,
    /// The whole sentence: "Checked: corrections", or "Not fully checked:
    /// repeated failures — tool outcomes were not saved for 3 records".
    pub text: String,
}

/// What M-21 says about one session over its whole history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionWarningsAnswer {
    /// No indexed session has this identifier.
    Missing,
    Read {
        /// The warnings not dismissed, earliest first: the only ones any
        /// mark, count or day shows.
        warnings: Vec<SessionWarningItem>,
        /// "2 warnings": what a row's marker is named; `null` exactly when
        /// there is no warning left, and then no row shows a marker.
        count_label: Option<String>,
        /// The detail's one-line answer, counting only the warnings not
        /// dismissed. With none it says whether every check read everything,
        /// so a session some checks could not fully read never reads as one
        /// with nothing wrong, and how many the user dismissed.
        summary: String,
        /// Completion summary for the collapsed checks.
        checks_summary: String,
        /// The warnings found now that the user dismissed, earliest first:
        /// listed apart on the session's page, each with an undo.
        dismissed: Vec<SessionWarningItem>,
        /// The five checks, in a fixed order.
        checks: Vec<SessionWarningCheck>,
        /// How many checks could not read everything.
        #[ts(type = "number")]
        incomplete_checks: u32,
    },
}

/// One session's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningsEntry {
    pub session_id: String,
    pub answer: SessionWarningsAnswer,
}

/// The answers for the sessions a view asked about, in identifier order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningsList {
    /// The zone every reason's clock times are written in.
    pub timezone: String,
    pub sessions: Vec<SessionWarningsEntry>,
}

/// Warnings a day row of the Effort card names, and its card lists, at most;
/// the rest are counted ("and 3 more").
pub const DAY_WARNING_LIMIT: usize = 5;

/// One warning on one day of a Dashboard report, with what the UI names its
/// session by: the saved title and the Guardian hint, which it passes through
/// the same host-title read and `displayTitle` rule the session rows use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DashboardDayWarning {
    pub session_id: String,
    pub host: String,
    /// The saved title, as the lanes and Sessions show it; `null` when none.
    pub title: Option<String>,
    /// Verified native Guardian header; a display hint, never a saved title.
    pub automated_review: bool,
    /// The session's last timed record, UTC milliseconds: the version the UI
    /// reads the session's host title for, as a lane uses its latest span.
    #[ts(type = "number")]
    pub last_ms: i64,
    /// The warning without its session: "Correction: you wrote …", its label
    /// and reason. The day row's name and card put the session's name before
    /// it.
    pub text: String,
    pub warning: SessionWarningItem,
}

/// One whole local day of a Dashboard report that has warnings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DashboardWarningDay {
    /// The day's date, as the report's own day rows name it ("2026-10-20").
    pub date: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    /// Every warning that day, earliest first: one dot each.
    pub warnings: Vec<DashboardDayWarning>,
    /// How many of the first `warnings` the day row names and its card lists:
    /// at most [`DAY_WARNING_LIMIT`].
    pub listed: u32,
    /// "2 warnings": every warning that day, counted.
    pub count_label: String,
    /// "and 3 more" when fewer than every warning are listed; `null`
    /// otherwise.
    pub more_label: Option<String>,
}

/// The warnings inside a Dashboard report's whole days, for its Effort card,
/// by day: every session with one there, each read over its whole history.
/// Days without a warning are left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DashboardWarnings {
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    pub timezone: String,
    pub days: Vec<DashboardWarningDay>,
}

/// The day's count label, how many warnings it lists and its "and N more".
fn day_words(count: usize) -> (u32, String, Option<String>) {
    let listed = count.min(DAY_WARNING_LIMIT);
    let more = count - listed;
    (
        listed as u32,
        plural(count as u64, "warning", "warnings"),
        (more > 0).then(|| format!("and {more} more")),
    )
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// "1 of your messages", "3 of your messages".
fn yours(messages: u64) -> String {
    format!("{messages} of your messages")
}

/// Why a check could not read everything, in plain words.
fn checks_summary(total: usize, incomplete: usize) -> String {
    if incomplete == 0 {
        format!("Checks · all {total} complete")
    } else {
        format!("Checks · {incomplete} of {total} incomplete")
    }
}

fn missing_text(missing: &Missing) -> String {
    match missing {
        Missing::TextNotKept { messages } => {
            format!("the words of {} were not kept", yours(*messages))
        }
        Missing::TextCut { messages } => format!(
            "only the first {} characters of {} were kept",
            xt_metrics::MAX_PROMPT_CHARS,
            yours(*messages)
        ),
        Missing::WrappedText { messages } => format!(
            "{} {} only partly yours (a wrapped reply or image), so the words could not be read",
            yours(*messages),
            if *messages == 1 { "is" } else { "are" }
        ),
        Missing::Unclassified { inputs } => format!(
            "whether you sent {} is not known",
            plural(*inputs, "input", "inputs")
        ),
        Missing::InterruptionUnknown { inputs } => format!(
            "whether {} {} an interruption was not saved",
            plural(*inputs, "input", "inputs"),
            if *inputs == 1 { "was" } else { "were" }
        ),
        Missing::OutcomeNotSaved { records } => format!(
            "tool outcomes were not saved for {}",
            plural(*records, "record", "records")
        ),
        Missing::OutcomeUnknown { results } => format!(
            "{} {} not say how {} ended",
            plural(*results, "tool result", "tool results"),
            if *results == 1 { "does" } else { "do" },
            if *results == 1 { "it" } else { "they" }
        ),
        Missing::UnmatchedResults { results } => format!(
            "{} a call that was not saved",
            plural(*results, "failed result names", "failed results name")
        ),
        Missing::FailedCallKey { calls, .. } => format!(
            "{} could not be compared with the others",
            plural(*calls, "failed call", "failed calls")
        ),
        Missing::StretchesUnmeasured => {
            "this session's working stretches could not be measured".to_owned()
        }
        Missing::RepeatsUnknown { stretches, .. } => format!(
            "the calls in {} could not be counted",
            plural(*stretches, "working stretch", "working stretches")
        ),
    }
}

/// The detail's line for one check.
fn check_text(kind: WarningKind, state: &CheckState) -> String {
    match state {
        CheckState::Complete => format!("Checked: {}", check_label(kind)),
        CheckState::Incomplete { missing } => format!(
            "Not fully checked: {} — {}",
            check_label(kind),
            missing
                .iter()
                .map(missing_text)
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

/// Whether the user dismissed `warning` and it has not grown past the size
/// it was dismissed at, given its session's dismissals by identity
/// ([`xt_metrics::MetricsDb::dismissed_warnings`]): the one test every
/// answer here uses to leave a warning out.
fn is_dismissed(dismissed: &BTreeMap<String, Dismissal>, warning: &SessionWarning) -> bool {
    dismissed
        .get(&warning.id())
        .is_some_and(|dismissal| grown(warning, dismissal).is_none())
}

/// One session's answer, worded; days are written in `zone`. A warning whose
/// identity is in `dismissed` is listed apart and counted nowhere else.
pub fn answer(
    found: WholeSessionWarnings,
    dismissed: &BTreeMap<String, Dismissal>,
    zone: &jiff::tz::TimeZone,
) -> SessionWarningsAnswer {
    let (warnings, checks, untimed) = match found {
        WholeSessionWarnings::Missing => return SessionWarningsAnswer::Missing,
        WholeSessionWarnings::Untimed { untimed_records } => {
            let why = if untimed_records == 1 {
                "this session's one saved record does not say when it happened".to_owned()
            } else {
                format!(
                    "none of this session's {untimed_records} saved records says when it happened"
                )
            };
            let checks = WarningKind::ALL
                .into_iter()
                .map(|kind| SessionWarningCheck {
                    kind: kind.into(),
                    complete: false,
                    text: format!("Not checked: {} — {why}", check_label(kind)),
                })
                .collect::<Vec<_>>();
            return SessionWarningsAnswer::Read {
                warnings: vec![],
                count_label: None,
                summary: format!(
                    "Not checked: {why}, so no warning can be found and none is shown."
                ),
                checks_summary: checks_summary(checks.len(), checks.len()),
                dismissed: vec![],
                incomplete_checks: checks.len() as u32,
                checks,
            };
        }
        WholeSessionWarnings::Read {
            warnings,
            untimed_records,
        } => (warnings.warnings, warnings.checks, untimed_records),
    };
    let checks: Vec<SessionWarningCheck> = WarningKind::ALL
        .into_iter()
        .map(|kind| {
            let state = checks.get(kind);
            let mut text = check_text(kind, state);
            if untimed > 0 {
                // Records with no time were read by no rule.
                text = if state.is_complete() {
                    format!(
                        "Not fully checked: {} — {} no time and {} not read",
                        check_label(kind),
                        plural(untimed, "saved record states", "saved records state"),
                        if untimed == 1 { "was" } else { "were" }
                    )
                } else {
                    format!(
                        "{text}; {} no time and {} not read",
                        plural(untimed, "saved record states", "saved records state"),
                        if untimed == 1 { "was" } else { "were" }
                    )
                };
            }
            SessionWarningCheck {
                kind: kind.into(),
                complete: state.is_complete() && untimed == 0,
                text,
            }
        })
        .collect();
    let incomplete = checks.iter().filter(|check| !check.complete).count() as u32;
    let (hidden, warnings): (Vec<_>, Vec<_>) = warnings
        .into_iter()
        .partition(|warning| is_dismissed(dismissed, warning));
    let count = warnings.len() as u64;
    let count_label = (count > 0).then(|| plural(count, "warning", "warnings"));
    let summary = match (count, incomplete, hidden.len()) {
        (0, 0, 0) => "No warnings. Every check read everything it needed.".to_owned(),
        (0, n, 0) => format!(
            "No warnings found, but {} of 5 checks could not read everything, so something may still have gone wrong.",
            n
        ),
        (0, 0, gone) => format!(
            "No warnings left: you dismissed {gone}. Every check read everything it needed."
        ),
        (0, n, gone) => format!(
            "No warnings left: you dismissed {gone}, but {n} of 5 checks could not read everything, so something may still have gone wrong."
        ),
        (count, 0, _) => format!("{} to review.", plural(count, "warning", "warnings")),
        (count, n, _) => format!(
            "{} to review. {} of 5 checks could not read everything, so there may be more.",
            plural(count, "warning", "warnings"),
            n
        ),
    };
    SessionWarningsAnswer::Read {
        warnings: warnings
            .into_iter()
            .map(|warning| {
                let dismissal = dismissed.get(&warning.id());
                SessionWarningItem::from_core(warning, dismissal, zone)
            })
            .collect(),
        count_label,
        summary,
        checks_summary: checks_summary(checks.len(), incomplete as usize),
        dismissed: hidden
            .into_iter()
            .map(|warning| {
                let dismissal = dismissed.get(&warning.id());
                SessionWarningItem::from_core(warning, dismissal, zone)
            })
            .collect(),
        checks,
        incomplete_checks: incomplete,
    }
}

/// The identifiers a warnings request may name: canonical session
/// identifiers, at most [`MAX_WARNING_SESSIONS`] distinct ones. Refused
/// before storage is touched otherwise.
pub fn validate_ids(ids: &[String]) -> Result<Vec<&str>, StateError> {
    let mut named: Vec<&str> = ids.iter().map(String::as_str).collect();
    named.sort_unstable();
    named.dedup();
    if named.len() > MAX_WARNING_SESSIONS
        || named
            .iter()
            .any(|id| id.is_empty() || id.len() > 256 || id.trim() != *id)
    {
        return Err(StateError::InvalidWarningRequest);
    }
    Ok(named)
}

/// What a dismissal or its undo left stored: the warning is dismissed or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionWarningDismissal {
    pub session_id: String,
    pub warning_id: String,
    /// `true` after a dismissal, `false` after an undo.
    pub dismissed: bool,
}

/// A dismissal or undo names one session as a warnings request does and one
/// warning identity, `kind:anchor` with one of the five kinds and an anchor.
/// Refused before storage is touched otherwise.
pub fn validate_dismissal(session_id: &str, warning_id: &str) -> Result<(), StateError> {
    validate_ids(&[session_id.to_owned()]).map_err(|_| StateError::InvalidWarningDismissal)?;
    let well_formed = warning_id.len() <= xt_store::dismissed_warning::MAX_WARNING_ID_CHARS
        && warning_id.split_once(':').is_some_and(|(kind, anchor)| {
            !anchor.is_empty() && WarningKind::ALL.iter().any(|known| known.as_str() == kind)
        });
    if well_formed {
        Ok(())
    } else {
        Err(StateError::InvalidWarningDismissal)
    }
}

/// The answers for exactly the named sessions, reasons written in `zone`.
pub fn session_warnings(
    db: &xt_metrics::MetricsDb,
    ids: &[&str],
    zone: &jiff::tz::TimeZone,
    timezone: String,
) -> Result<SessionWarningsList, StateError> {
    let (found, dismissed) = db.read_snapshot(|db| {
        Ok((
            db.saved_whole_session_warnings(ids, zone)?,
            db.dismissed_warnings(ids)?,
        ))
    })?;
    let none = BTreeMap::new();
    Ok(SessionWarningsList {
        timezone,
        sessions: found
            .into_iter()
            .map(|(session_id, found)| SessionWarningsEntry {
                answer: answer(found, dismissed.get(&session_id).unwrap_or(&none), zone),
                session_id,
            })
            .collect(),
    })
}

/// The warnings inside one report's whole days, by day, with what names each
/// session. `window` is the report's whole days ([`crate::dashboard::report_whole_days`]).
pub fn window_warnings(
    db: &xt_metrics::MetricsDb,
    window: xt_metrics::Window,
    zone: &jiff::tz::TimeZone,
    timezone: String,
) -> Result<DashboardWarnings, StateError> {
    let (found, context, dismissed) = db.read_snapshot(|db| {
        let found = db.saved_warnings_in_window(window, zone)?;
        let mut context = Vec::new();
        let mut dismissed = BTreeMap::new();
        for chunk in found.chunks(xt_metrics::MAX_SESSIONS) {
            let ids: Vec<&str> = chunk
                .iter()
                .map(|found| found.session_id.as_str())
                .collect();
            context.extend(db.session_context(&ids)?);
            dismissed.extend(db.dismissed_warnings(&ids)?);
        }
        Ok((found, context, dismissed))
    })?;
    let context: std::collections::BTreeMap<_, _> = context
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut all = Vec::new();
    for found in found {
        let row = context.get(&found.session_id);
        let hidden = dismissed.get(&found.session_id);
        for warning in found.warnings {
            if hidden.is_some_and(|hidden| is_dismissed(hidden, &warning)) {
                continue;
            }
            let dismissal = hidden.and_then(|hidden| hidden.get(&warning.id()));
            let warning = SessionWarningItem::from_core(warning, dismissal, zone);
            all.push(DashboardDayWarning {
                session_id: found.session_id.clone(),
                host: row.map(|row| row.host.clone()).unwrap_or_default(),
                title: row.and_then(|row| row.title.clone()),
                automated_review: row.is_some_and(|row| row.automated_review),
                last_ms: found.last_ms,
                text: format!("{}: {}", warning.label, warning.reason),
                warning,
            });
        }
    }
    all.sort_by(|a, b| {
        (a.warning.at_ms, &a.session_id, &a.warning.id).cmp(&(
            b.warning.at_ms,
            &b.session_id,
            &b.warning.id,
        ))
    });
    let mut days = Vec::new();
    for bucket in window.local_days(zone.clone())? {
        let (start, end) = (bucket.window.start_ms(), bucket.window.end_ms());
        let warnings: Vec<_> = all
            .iter()
            .filter(|entry| entry.warning.at_ms >= start && entry.warning.at_ms < end)
            .cloned()
            .collect();
        if warnings.is_empty() {
            continue;
        }
        let (listed, count_label, more_label) = day_words(warnings.len());
        days.push(DashboardWarningDay {
            date: bucket.date.to_string(),
            start_ms: start,
            end_ms: end,
            warnings,
            listed,
            count_label,
            more_label,
        });
    }
    Ok(DashboardWarnings {
        start_ms: window.start_ms(),
        end_ms: window.end_ms(),
        timezone,
        days,
    })
}

/// One Dashboard range's warnings as a fixture carries them: the request
/// that reads them (the report's range, window end and zone) and the answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FixtureDashboardWarnings {
    pub window_days: u32,
    #[ts(type = "number")]
    pub window_end_ms: i64,
    pub timezone: String,
    pub warnings: DashboardWarnings,
}

/// Every session a fixture's Sessions pages and Dashboard lanes name, read
/// through the same command path in the fixture's zone (UTC), so a fixture
/// answers the same request the native command does.
pub fn fixture_session_warnings(
    path: &std::path::Path,
    now_ms: i64,
    prices: Option<&serde_json::Value>,
) -> Result<SessionWarningsList, StateError> {
    let metrics = xt_metrics::MetricsDb::open(path)?;
    let mut ids = std::collections::BTreeSet::new();
    for page in crate::dto::fixture_session_pages(path, now_ms, prices)? {
        ids.extend(page.rows.into_iter().map(|row| row.id));
    }
    for report in crate::dashboard::fixture_reports(path, now_ms, prices)? {
        ids.extend(report.lanes.into_iter().map(|lane| lane.session_id));
        ids.extend(
            report
                .lane_sessions
                .into_iter()
                .chain(report.lane_sub_sessions)
                .map(|session| session.session_id),
        );
    }
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let timezone =
        crate::dashboard::zone_name(&jiff::tz::TimeZone::UTC, crate::dto::MetricClock::Fixture);
    let mut list = SessionWarningsList {
        timezone: timezone.clone(),
        sessions: Vec::new(),
    };
    for chunk in ids.chunks(MAX_WARNING_SESSIONS) {
        list.sessions.extend(
            session_warnings(&metrics, chunk, &jiff::tz::TimeZone::UTC, timezone.clone())?.sessions,
        );
    }
    Ok(list)
}

/// Every range's Effort warnings at the fixture's pinned instant, in UTC.
pub fn fixture_dashboard_warnings(
    path: &std::path::Path,
    now_ms: i64,
) -> Result<Vec<FixtureDashboardWarnings>, StateError> {
    let metrics = xt_metrics::MetricsDb::open(path)?;
    let zone = jiff::tz::TimeZone::UTC;
    let timezone = crate::dashboard::zone_name(&zone, crate::dto::MetricClock::Fixture);
    crate::dashboard::WINDOW_PRESETS
        .into_iter()
        .map(|days| {
            let window = crate::dashboard::report_whole_days(days, now_ms, &zone)?;
            Ok(FixtureDashboardWarnings {
                window_days: days,
                window_end_ms: now_ms,
                timezone: timezone.clone(),
                warnings: window_warnings(&metrics, window, &zone, timezone.clone())?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_lists_its_first_five_warnings_and_counts_the_rest() {
        assert_eq!(
            day_words(7),
            (5, "7 warnings".to_owned(), Some("and 2 more".to_owned()))
        );
        assert_eq!(day_words(5), (5, "5 warnings".to_owned(), None));
        assert_eq!(day_words(1), (1, "1 warning".to_owned(), None));
    }

    /// Pins the lane marker's template text: the UI fills it in
    /// (`beforeAxisLabel`), so its words are checked here and there.
    #[test]
    fn the_lane_marker_template_and_the_request_limit() {
        assert_eq!(
            TEXT.before_axis_one,
            "1 warning before the last {hours} hours"
        );
        assert_eq!(
            TEXT.before_axis_many,
            "{count} warnings before the last {hours} hours"
        );
        assert_eq!(TEXT.max_sessions, xt_metrics::MAX_SESSIONS);
    }
}
