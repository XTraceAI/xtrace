//! M-21 session warnings read from a real store: each rule fires from the
//! stored facts it names, automatic inputs never count as a person's, and a
//! rule whose inputs were not kept says so instead of reporting nothing.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{
    CheckState, MetricsDb, Missing, RepeatThresholds, SessionRepeats, SessionWarnings, WarningKind,
    Window,
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
fn result(uuid: &str, time: &str, call: &str, failed: Option<bool>, text: &str) -> CanonicalRecord {
    let mut block = json!({"type":"tool_result","tool_use_id":call,"content":text});
    if let Some(failed) = failed {
        block["is_error"] = failed.into();
    }
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[block]}}))
}

fn seed(db: &mut TempDb, id: &str, host: &str, rows: &[CanonicalRecord], keep_content: bool) {
    let mut meta = SessionMeta::new(id, host, SessionSource::Transcript);
    meta.native_session_id = Some(id.into());
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
        .upsert_records(id, rows, keep_content)
        .unwrap();
}

fn warnings(db: &TempDb, id: &str) -> SessionWarnings {
    MetricsDb::open(db.path())
        .unwrap()
        .session_warnings(window(), RepeatThresholds::default(), &TimeZone::UTC)
        .unwrap()
        .into_iter()
        .find(|s| s.session_id == id)
        .unwrap()
}
fn uuids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn conversation() -> Vec<CanonicalRecord> {
    let pnpm = json!({"command": "pnpm test"});
    let read = json!({"file_path": "/a.rs"});
    let todo = json!({"todos": []});
    vec![
        says("h1", &at(0, 0), "Fix the build please"),
        calls("a1", &at(0, 1), &[("t1", "Bash", pnpm.clone())]),
        result("r1", &at(0, 2), "t1", Some(true), "1 test failed"),
        calls("a2", &at(0, 3), &[("t2", "Bash", pnpm.clone())]),
        result("r2", &at(0, 4), "t2", Some(true), "1 test failed"),
        calls(
            "a3",
            &at(0, 5),
            &[("t3", "Edit", json!({"file_path": "/b.rs"}))],
        ),
        result(
            "r3",
            &at(0, 6),
            "t3",
            Some(true),
            "The user doesn't want to proceed with this tool use. The tool use was rejected.",
        ),
        // Claude Code's marker right after the declined call: one interruption.
        says(
            "i1",
            &at(0, 7),
            "[Request interrupted by user for tool use]",
        ),
        // Two identical automatic inputs: the classification says no person
        // sent them, so they are never a repeated prompt or a correction.
        says(
            "x1",
            &at(0, 8),
            "<system-reminder>stop it now please</system-reminder>",
        ),
        reply("a4", &at(0, 9)),
        says(
            "x2",
            &at(0, 10),
            "<system-reminder>stop it now please</system-reminder>",
        ),
        says("h2", &at(1, 0), "still not working"),
        reply("a5", &at(1, 30)),
        says("h3", &at(2, 0), "fix  the build please"),
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
        // Six task-list updates in four active minutes: M-20 calls this
        // stretch circling; they are expected repeats, so M-21 does not.
        calls(
            "a8",
            &at(7, 30),
            &[
                ("u1", "TodoWrite", todo.clone()),
                ("u2", "TodoWrite", todo.clone()),
                ("u3", "TodoWrite", todo.clone()),
                ("u4", "TodoWrite", todo.clone()),
                ("u5", "TodoWrite", todo.clone()),
                ("u6", "TodoWrite", todo),
            ],
        ),
        reply("a9", &at(11, 30)),
    ]
}

#[test]
fn every_rule_fires_from_kept_content() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone(), w.reason.as_str()))
        .collect();
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
                uuids(&["r3", "i1"]),
                "you declined a call to Edit (12:00)"
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
    assert_eq!((found.tool_results, found.tool_results_known), (3, 3));

    // M-20, the Dashboard's number, still compares every call: the
    // task-list stretch M-21 leaves alone is circling there.
    let SessionRepeats::Measured { stretches, .. } = MetricsDb::open(db.path())
        .unwrap()
        .session_repeats(window(), SESSION, RepeatThresholds::default())
        .unwrap()
    else {
        panic!("measured");
    };
    let circling: Vec<_> = stretches
        .iter()
        .filter(|s| s.circling == Some(true))
        .map(|s| s.start_uuid.as_str())
        .collect();
    assert_eq!(circling, vec!["h3", "h4"]);
}

#[test]
fn outcomes_are_read_without_kept_content_and_a_preview_is_read() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), false);
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
    let found = warnings(&db, SESSION);
    let kinds: Vec<_> = found.warnings.iter().map(|w| w.kind).collect();
    // Outcomes, interruptions and circling are metadata; the correction
    // reads the preview; the other prompts' words were not kept.
    assert_eq!(
        kinds,
        vec![
            WarningKind::RepeatedFailure,
            WarningKind::Interruption,
            WarningKind::Correction,
            WarningKind::Circling
        ]
    );
    assert_eq!(found.checks.repeated_failure, CheckState::Complete);
    assert_eq!(found.checks.interruption, CheckState::Complete);
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
    assert_eq!((found.person_messages, found.person_messages_read), (4, 1));
}

#[test]
fn results_indexed_before_outcomes_were_kept_are_said_not_saved() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), false);
    // As an index written before migration 27 holds them.
    Connection::open(db.path())
        .unwrap()
        .execute_batch("DELETE FROM tool_results; UPDATE tool_uses SET call_id=NULL;")
        .unwrap();
    let found = warnings(&db, SESSION);
    assert!(
        found
            .warnings
            .iter()
            .all(|w| w.kind != WarningKind::RepeatedFailure)
    );
    // The interruption marker is still there, on its own now.
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| w.record_uuids.clone())
        .collect();
    assert_eq!(interruptions, vec![uuids(&["i1"])]);
    let missing = CheckState::Incomplete {
        missing: vec![Missing::OutcomeNotSaved { records: 3 }],
    };
    assert_eq!(found.checks.repeated_failure, missing);
    assert_eq!(found.checks.interruption, missing);
}

#[test]
fn codex_results_without_a_flag_are_unknown() {
    let mut db = TempDb::empty().unwrap();
    let bash = json!({"command": "cargo test"});
    seed(
        &mut db,
        "codex-s",
        "codex",
        &[
            says("h1", &at(0, 0), "make the tests pass"),
            calls("a1", &at(0, 1), &[("c1", "exec_command", bash.clone())]),
            result("r1", &at(0, 2), "c1", None, "Exit code: 1"),
            calls("a2", &at(0, 3), &[("c2", "exec_command", bash)]),
            result("r2", &at(0, 4), "c2", None, "Exit code: 1"),
            reply("a3", &at(0, 5)),
        ],
        false,
    );
    let found = warnings(&db, "codex-s");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(
        found.checks.repeated_failure,
        CheckState::Incomplete {
            missing: vec![Missing::OutcomeUnknown { results: 2 }]
        }
    );
    assert_eq!((found.tool_results, found.tool_results_known), (2, 0));
}
