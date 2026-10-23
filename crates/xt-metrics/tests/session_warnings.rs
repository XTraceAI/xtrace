//! M-21 session warnings read from a real store: each rule fires from the
//! stored facts it names, automatic inputs never count as a person's, and a
//! rule whose inputs were not kept says so instead of reporting nothing.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    CheckState, MetricsDb, Missing, RepeatThresholds, SessionWarnings, WarningKind, Window,
};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, retention::RetentionMode};

const SESSION: &str = "s";

fn ms(value: &str) -> i64 {
    value.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn at(minute: u32, second: u32) -> String {
    format!("2026-09-07T12:{minute:02}:{second:02}Z")
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn says(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn reply(uuid: &str, time: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","content":[{"type":"text","text":"Synthetic reply"}]}}))
}
/// One assistant record calling each `(id, tool, input)`.
fn calls(uuid: &str, time: &str, calls: &[(&str, &str, Value)]) -> CanonicalRecord {
    let content: Vec<Value> = calls
        .iter()
        .map(|(id, name, input)| json!({"type":"tool_use","id":id,"name":name,"input":input}))
        .collect();
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","content":content}}))
}
fn result(uuid: &str, time: &str, call: &str, failed: bool, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":call,"is_error":failed,"content":text}]}}))
}

fn seed(db: &mut TempDb, rows: &[CanonicalRecord], keep_content: bool) {
    let mut meta = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
    meta.native_session_id = Some(SESSION.into());
    meta.surface = Some("cli".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    db.store_mut()
        .set_retention_mode(if keep_content {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    db.store_mut()
        .upsert_records(SESSION, rows, keep_content)
        .unwrap();
}

fn warnings(db: &TempDb) -> SessionWarnings {
    let mut all = MetricsDb::open(db.path())
        .unwrap()
        .session_warnings(window(), RepeatThresholds::default(), &TimeZone::UTC)
        .unwrap();
    assert_eq!(all.len(), 1);
    all.remove(0)
}

fn conversation() -> Vec<CanonicalRecord> {
    let pnpm = json!({"command": "pnpm test"});
    let read = json!({"file_path": "/a.rs"});
    vec![
        says("h1", &at(0, 0), "Fix the build"),
        calls("a1", &at(0, 1), &[("t1", "Bash", pnpm.clone())]),
        result("r1", &at(0, 2), "t1", true, "1 test failed"),
        calls("a2", &at(0, 3), &[("t2", "Bash", pnpm.clone())]),
        result("r2", &at(0, 4), "t2", true, "1 test failed"),
        calls(
            "a3",
            &at(0, 5),
            &[("t3", "Edit", json!({"file_path": "/b.rs"}))],
        ),
        result(
            "r3",
            &at(0, 6),
            "t3",
            true,
            "The user doesn't want to proceed with this tool use. The tool use was rejected.",
        ),
        says(
            "i1",
            &at(0, 7),
            "[Request interrupted by user for tool use]",
        ),
        // Two identical automatic inputs: the classification says no person
        // sent them, so they are never a repeated prompt or a correction.
        says("x1", &at(0, 8), "<system-reminder>stop</system-reminder>"),
        reply("a4", &at(0, 9)),
        says("x2", &at(0, 10), "<system-reminder>stop</system-reminder>"),
        says("h2", &at(1, 0), "still not working"),
        reply("a5", &at(1, 30)),
        says("h3", &at(2, 0), "fix  the build"),
        calls(
            "a6",
            &at(2, 30),
            &[
                ("t4", "Read", read.clone()),
                ("t5", "Read", read.clone()),
                ("t6", "Read", read.clone()),
                ("t7", "Read", read.clone()),
                ("t8", "Read", read.clone()),
                ("t9", "Read", read),
            ],
        ),
        reply("a7", &at(6, 30)),
        says("h4", &at(7, 0), "I still need an export button"),
        reply("a8", &at(7, 30)),
    ]
}

#[test]
fn every_rule_fires_from_kept_content() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, &conversation(), true);
    let found = warnings(&db);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    let uuids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        summary,
        vec![
            (
                WarningKind::RepeatedFailure,
                uuids(&["r1", "r2"]),
                "the same Bash call failed twice (12:00, 12:00)"
            ),
            (
                WarningKind::Interruption,
                uuids(&["r3"]),
                "you declined a call to Edit (12:00)"
            ),
            (
                WarningKind::Interruption,
                uuids(&["i1"]),
                "you interrupted the agent (12:00)"
            ),
            (
                WarningKind::Correction,
                uuids(&["h2"]),
                "you wrote \"still not\" after a reply (12:01)"
            ),
            (
                WarningKind::Circling,
                uuids(&["h3", "a6", "a7"]),
                "went in circles: 5 repeated calls in 4 active minutes, most often the same Read call (6 times) (from 12:02)"
            ),
            (
                WarningKind::RepeatedPrompt,
                uuids(&["h1", "h3"]),
                "same prompt sent twice (12:00, 12:02)"
            ),
        ]
    );
    for kind in WarningKind::ALL {
        assert_eq!(found.checks.get(kind), &CheckState::Complete, "{kind:?}");
    }
    assert_eq!((found.person_messages, found.person_messages_read), (4, 4));
}

#[test]
fn unkept_content_is_said_and_a_kept_preview_is_read() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, &conversation(), false);
    // The index keeps a short preview of a person's message whatever the
    // retention setting says; keep one, for the correction only.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "INSERT INTO record_previews(record_uuid,session_id,kind,text,truncated,rule_version)
             VALUES('h2',?1,'person','still not working',0,1)",
            [SESSION],
        )
        .unwrap();
    let found = warnings(&db);
    let kinds: Vec<_> = found.warnings.iter().map(|w| w.kind).collect();
    // Interruption and circling read metadata only; the correction reads the
    // preview. Failures, declines and the other prompts were not kept.
    assert_eq!(
        kinds,
        vec![
            WarningKind::Interruption,
            WarningKind::Correction,
            WarningKind::Circling
        ]
    );
    assert_eq!(found.checks.circling, CheckState::Complete);
    assert_eq!(
        found.checks.correction,
        CheckState::Incomplete {
            missing: vec![Missing::TextNotKept { messages: 2 }]
        }
    );
    assert_eq!(
        found.checks.repeated_prompt,
        CheckState::Incomplete {
            missing: vec![Missing::TextNotKept { messages: 3 }]
        }
    );
    // Seven records carry calls (four) or results (three), none of them
    // kept, so failed and declined calls cannot be seen.
    let unkept = CheckState::Incomplete {
        missing: vec![Missing::ToolResultsNotKept { records: 7 }],
    };
    assert_eq!(found.checks.repeated_failure, unkept);
    assert_eq!(found.checks.interruption, unkept);
    assert_eq!((found.person_messages, found.person_messages_read), (4, 1));
}
