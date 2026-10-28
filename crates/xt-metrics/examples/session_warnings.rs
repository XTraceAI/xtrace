//! Count M-21 session warnings in one database, read-only.
//!
//! `cargo run -p xt-metrics --example session_warnings -- <database>
//!  [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--admission-ids] [--counts-only]`
//!
//! The window is the database's whole recorded history, or the days from
//! `--from` through `--to` (both inclusive, in this computer's time zone).
//! Every session with a timed record in the window is read over its whole
//! history, as the app reads it, and only warnings inside the window count.
//! Prints how many sessions were read, how many have at least one warning,
//! the count per kind, how many sessions each rule could not fully check and
//! why, the admitted-mistake counts per phrase name, the saved reply facts
//! in the window, and up to eight example sessions per kind.
//!
//! `--admission-ids` also prints one line per admitted-mistake warning: the
//! session, the person's message ID, the reply ID and the phrase name.
//! `--counts-only` leaves out the examples, which print saved session titles
//! and the words a correction matched. Nothing else prints message text.

use jiff::{civil::Date, tz::TimeZone};
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};
use xt_metrics::{
    CheckState, MAX_SESSIONS, MetricsDb, Missing, SessionWarnings, WarningKind,
    WholeSessionWarnings, Window,
};
use xt_store::reply_admission::{self, Phrase};

const EXAMPLES: usize = 8;
const USAGE: &str = "usage: session_warnings <database> [--from YYYY-MM-DD] [--to YYYY-MM-DD] \
                     [--admission-ids] [--counts-only]";

struct Args {
    path: String,
    from: Option<Date>,
    to: Option<Date>,
    admission_ids: bool,
    counts_only: bool,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut path = None;
    let (mut from, mut to) = (None, None);
    let (mut admission_ids, mut counts_only) = (false, false);
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        match arg.as_str() {
            "--from" => from = Some(given.next().ok_or(USAGE)?.parse::<Date>()?),
            "--to" => to = Some(given.next().ok_or(USAGE)?.parse::<Date>()?),
            "--admission-ids" => admission_ids = true,
            "--counts-only" => counts_only = true,
            _ if arg.starts_with("--") || path.is_some() => return Err(USAGE.into()),
            _ => path = Some(arg),
        }
    }
    Ok(Args {
        path: path.ok_or(USAGE)?,
        from,
        to,
        admission_ids,
        counts_only,
    })
}

/// Which phrase a warning's reason quotes, by its label.
fn quoted_phrase(reason: &str) -> Option<Phrase> {
    let label = reason.split('"').nth(1)?;
    Phrase::ALL
        .into_iter()
        .find(|phrase| phrase.label() == label)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = args()?;
    let path = &args.path;
    let metrics = MetricsDb::open(path)?;
    let side = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    side.pragma_update(None, "query_only", true)?;
    let (first, last): (Option<i64>, Option<i64>) = side.query_row(
        "SELECT min(ts_ms),max(ts_ms) FROM v_session_events",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (Some(first), Some(last)) = (first, last) else {
        println!("no timed events");
        return Ok(());
    };
    let zone = TimeZone::system();
    let day_start = |date: Date| -> Result<i64, jiff::Error> {
        Ok(date.to_zoned(zone.clone())?.timestamp().as_millisecond())
    };
    // The exact instant can sit up to a second past its millisecond
    // projection around a leap second; end well after the last one.
    let start = match args.from {
        Some(date) => day_start(date)?,
        None => first,
    };
    let end = match args.to {
        Some(date) => day_start(date.tomorrow()?)?,
        None => last + 2_000,
    };
    let window = Window::new(start, end)?;
    let started = std::time::Instant::now();
    // Each session active in the window, read over its whole history as the
    // app reads it; only warnings inside the window count.
    let active: Vec<String> = side
        .prepare(
            "SELECT DISTINCT session_id FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2
               AND session_id IN (SELECT session_id FROM sessions WHERE kind='user')
             ORDER BY session_id",
        )?
        .query_map([start, end], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut all = Vec::<SessionWarnings>::new();
    for chunk in active.chunks(MAX_SESSIONS) {
        let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
        for (_, answer) in metrics.whole_session_warnings(&ids, &zone)? {
            if let WholeSessionWarnings::Read { mut warnings, .. } = answer {
                warnings
                    .warnings
                    .retain(|w| w.at_ms >= window.start_ms() && w.at_ms < window.end_ms());
                all.push(warnings);
            }
        }
    }
    let elapsed = started.elapsed();

    let host = |id: &str| -> rusqlite::Result<(String, Option<String>)> {
        side.query_row(
            "SELECT host,nullif(trim(title),'') FROM sessions WHERE session_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    };
    let mut hosts = BTreeMap::<String, String>::new();
    for session in &all {
        hosts.insert(session.session_id.clone(), host(&session.session_id)?.0);
    }

    println!("Session warnings (M-21), read-only");
    println!(
        "window: {} .. {} ({}), read in {:.1} s",
        jiff::Timestamp::from_millisecond(start)?,
        jiff::Timestamp::from_millisecond(end)?,
        if args.from.is_none() && args.to.is_none() {
            "whole history"
        } else {
            "--from/--to, end exclusive"
        },
        elapsed.as_secs_f64()
    );
    let mut per_host = BTreeMap::<&str, (u64, u64)>::new();
    for session in &all {
        let entry = per_host.entry(&hosts[&session.session_id]).or_default();
        entry.0 += 1;
        entry.1 += u64::from(!session.warnings.is_empty());
    }
    println!();
    println!("sessions read: {}", all.len());
    println!(
        "sessions with at least one warning: {}",
        all.iter().filter(|s| !s.warnings.is_empty()).count()
    );
    for (host, (read, warned)) in &per_host {
        println!("  {host}: {read} read, {warned} with a warning");
    }

    println!();
    println!("per kind: warnings / sessions with one / sessions not fully checked");
    for kind in WarningKind::ALL {
        let warnings: usize = all
            .iter()
            .map(|s| s.warnings.iter().filter(|w| w.kind == kind).count())
            .sum();
        let sessions = all
            .iter()
            .filter(|s| s.warnings.iter().any(|w| w.kind == kind))
            .count();
        let incomplete = all
            .iter()
            .filter(|s| !s.checks.get(kind).is_complete())
            .count();
        println!("  {kind:?}: {warnings} / {sessions} / {incomplete}");
        let mut by_host = BTreeMap::<&str, usize>::new();
        for s in all
            .iter()
            .filter(|s| s.warnings.iter().any(|w| w.kind == kind))
        {
            *by_host.entry(&hosts[&s.session_id]).or_default() += 1;
        }
        if !by_host.is_empty() {
            println!("      sessions by app: {by_host:?}");
        }
    }

    println!();
    println!("why rules could not fully check a session (sessions per reason):");
    for kind in WarningKind::ALL {
        let mut reasons = BTreeMap::<String, usize>::new();
        for session in &all {
            if let CheckState::Incomplete { missing } = session.checks.get(kind) {
                let names: BTreeSet<_> = missing.iter().map(reason_name).collect();
                for name in names {
                    *reasons.entry(name).or_default() += 1;
                }
            }
        }
        println!("  {kind:?}: {reasons:?}");
    }
    let text_none = all
        .iter()
        .filter(|s| s.person_messages > 0 && s.person_messages_read == 0)
        .count();
    let no_person = all.iter().filter(|s| s.person_messages == 0).count();
    let (sent, read) = all.iter().fold((0, 0), |(sent, read), s| {
        (sent + s.person_messages, read + s.person_messages_read)
    });
    println!();
    println!(
        "sessions where the text rules (correction, repeated prompt) could not fully run: {}",
        all.iter()
            .filter(
                |s| !s.checks.correction.is_complete() || !s.checks.repeated_prompt.is_complete()
            )
            .count()
    );
    println!("  of which no person's message words were readable at all: {text_none}");
    println!("sessions with no person's message in the window: {no_person}");
    println!("person's messages: {sent}, words readable for {read}");
    let (results, known) = all.iter().fold((0, 0), |(results, known), s| {
        (results + s.tool_results, known + s.tool_results_known)
    });
    println!(
        "tool results with a saved outcome: {results}, known outcome for {known}; \
         sessions where every result had a known outcome: {}",
        all.iter()
            .filter(|s| s.tool_results > 0 && s.tool_results == s.tool_results_known)
            .count()
    );

    // Which correction words matched: the quoted words of each reason.
    let mut matched = BTreeMap::<String, usize>::new();
    for warning in all
        .iter()
        .flat_map(|s| &s.warnings)
        .filter(|w| w.kind == WarningKind::Correction)
    {
        if let Some(phrase) = warning.reason.split('"').nth(1) {
            let word = if phrase.starts_with("still") || phrase.ends_with("still") {
                "still + negative"
            } else {
                phrase
            };
            *matched.entry(word.to_owned()).or_default() += 1;
        }
    }
    println!();
    println!("correction warnings by matched word: {matched:?}");

    // Admitted mistakes: the first warning phrase of each reply, and the
    // replies the rule could not read.
    let admissions: Vec<_> = all
        .iter()
        .flat_map(|s| {
            s.warnings
                .iter()
                .filter(|w| w.kind == WarningKind::AdmittedMistake)
        })
        .collect();
    let mut first_phrase = BTreeMap::<&str, usize>::new();
    for warning in &admissions {
        let name = quoted_phrase(&warning.reason).map_or("(unknown)", Phrase::name);
        *first_phrase.entry(name).or_default() += 1;
    }
    println!();
    println!("admitted-mistake warnings by first phrase name: {first_phrase:?}");
    let (mut not_checked, mut not_checked_sessions) = (0, 0);
    for session in &all {
        if let CheckState::Incomplete { missing } = &session.checks.admitted_mistake {
            let replies: u64 = missing
                .iter()
                .map(|m| match m {
                    Missing::ReplyNotChecked { replies } => *replies,
                    _ => 0,
                })
                .sum();
            not_checked += replies;
            not_checked_sessions += usize::from(replies > 0);
        }
    }
    println!(
        "replies to a person's message not checked (no readable saved phrases): {not_checked}, \
         in {not_checked_sessions} sessions"
    );

    // The saved facts themselves, for agent replies with text in the window.
    let mut facts = side.prepare(
        "SELECT a.phrases,a.fact_version,a.conflicted,r.has_conflict,
                a.record_uuid IS NOT NULL
         FROM records r JOIN sessions s ON s.session_id=r.session_id
         LEFT JOIN reply_admissions a ON a.record_uuid=r.uuid AND a.session_id=r.session_id
         WHERE s.kind='user' AND r.type='assistant' AND r.is_sidechain=0 AND r.text_len>0
           AND r.ts_ms>=?1 AND r.ts_ms<?2",
    )?;
    let mut rows = facts.query([start, end])?;
    let (mut replies, mut saved, mut conflicted, mut record_conflict, mut newer, mut with_phrase) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut every_phrase = BTreeMap::<String, usize>::new();
    while let Some(row) = rows.next()? {
        replies += 1;
        record_conflict += u64::from(row.get::<_, bool>(3)?);
        if !row.get::<_, bool>(4)? {
            continue;
        }
        saved += 1;
        conflicted += u64::from(row.get::<_, bool>(2)?);
        newer += u64::from(row.get::<_, i64>(1)? > reply_admission::FACT_VERSION);
        let phrases: String = row.get(0)?;
        with_phrase += u64::from(!phrases.is_empty());
        for name in phrases.split_whitespace() {
            *every_phrase.entry(name.to_owned()).or_default() += 1;
        }
    }
    drop(rows);
    println!(
        "agent replies with text in the window: {replies}; with saved phrases row: {saved}; \
         rows marked conflicted: {conflicted}; replies on a conflicted record: {record_conflict}; \
         rows from a later build: {newer}; rows naming a phrase: {with_phrase}"
    );
    println!(
        "saved phrase names in those rows (every phrase, not only the first): {every_phrase:?}"
    );
    if args.admission_ids {
        println!();
        println!("admitted-mistake warnings: session person_message_id reply_id phrase");
        for session in &all {
            for warning in session
                .warnings
                .iter()
                .filter(|w| w.kind == WarningKind::AdmittedMistake)
            {
                println!(
                    "  {} {} {} {}",
                    session.session_id,
                    warning.record_uuids.first().map_or("-", String::as_str),
                    warning.anchor,
                    quoted_phrase(&warning.reason).map_or("(unknown)", Phrase::name)
                );
            }
        }
    }
    if args.counts_only {
        return Ok(());
    }

    for kind in WarningKind::ALL {
        println!();
        println!("examples: {kind:?}");
        let mut shown = 0;
        for session in &all {
            let Some(first) = session.warnings.iter().find(|w| w.kind == kind) else {
                continue;
            };
            let more = session.warnings.iter().filter(|w| w.kind == kind).count() - 1;
            let (host, title) = host(&session.session_id)?;
            println!(
                "  {} [{host}] {:?} at {}: {}{}",
                session.session_id,
                title.as_deref().unwrap_or("(no saved title)"),
                jiff::Timestamp::from_millisecond(first.at_ms)?
                    .to_zoned(zone.clone())
                    .strftime("%Y-%m-%d %H:%M"),
                first.reason,
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                }
            );
            shown += 1;
            if shown == EXAMPLES {
                break;
            }
        }
        if shown == 0 {
            println!("  none");
        }
    }
    Ok(())
}

fn reason_name(missing: &Missing) -> String {
    match missing {
        Missing::TextNotKept { .. } => "words not kept".into(),
        Missing::TextCut { .. } => "only the first 280 characters kept".into(),
        Missing::WrappedText { .. } => "only part of message is the person's".into(),
        Missing::Unclassified { .. } => "sender unknown".into(),
        Missing::InterruptionUnknown { .. } => "interruption flag unknown".into(),
        Missing::OutcomeNotSaved { .. } => "tool outcome not saved".into(),
        Missing::OutcomeUnknown { .. } => "tool outcome unknown".into(),
        Missing::UnmatchedResults { .. } => "result names an unknown call".into(),
        Missing::FailedCallKey { reason, .. } => format!("failed call key: {reason:?}"),
        Missing::ReplyNotChecked { .. } => "reply not checked".into(),
    }
}
