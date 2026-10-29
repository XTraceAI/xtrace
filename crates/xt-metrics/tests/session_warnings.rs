//! M-21 session warnings read from a real store: each rule fires from the
//! stored facts it names, automatic inputs never count as a person's, and a
//! rule whose inputs were not kept says so instead of reporting nothing.
use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use xt_fixtures::TempDb;
use xt_metrics::{
    CheckState, MetricsDb, Missing, RepeatThresholds, SessionRepeats, SessionWarnings, WarningKind,
    WholeSessionWarnings, Window,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, batch::IngestBatch, retention::RetentionMode,
    tool_sent::ToolSentKind,
};

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
        .session_warnings(window(), &TimeZone::UTC)
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
        // stretch circling, as it does the six reads above; neither is a
        // warning.
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

    // M-20, the Dashboard's number, still finds both circling stretches;
    // no warning is made from either.
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
    // Outcomes and interruptions are metadata; the correction
    // reads the preview; the other prompts' words were not kept.
    assert_eq!(
        kinds,
        vec![
            WarningKind::RepeatedFailure,
            WarningKind::Interruption,
            WarningKind::Correction,
        ]
    );
    assert_eq!(found.checks.repeated_failure, CheckState::Complete);
    assert_eq!(found.checks.interruption, CheckState::Complete);
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

fn result_observation(kind: &str) -> CanonicalRecord {
    let block = |id| json!({"type":"tool_result","tool_use_id":id,"is_error":true,"content":""});
    let content = match kind {
        "unknown" => Value::Null,
        "empty" => json!([]),
        "other" => json!([{"type":"text","text":""}]),
        "one" => json!([block("t1")]),
        "two" => json!([block("t1"), block("t2")]),
        "replaced" => json!([block("t1"), {"type":"text","text":""}]),
        _ => unreachable!(),
    };
    record(json!({"uuid":"r","type":"user","timestamp":at(0,2),
        "message":{"role":"user","content":content}}))
}

fn observed_result_pair(first: &str, second: &str, same_batch: bool) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("h", &at(0, 0), "Run both checks please"),
            calls(
                "a",
                &at(0, 1),
                &[
                    ("t1", "Bash", json!({"command":"pnpm test"})),
                    ("t2", "Bash", json!({"command":"pnpm test"})),
                ],
            ),
        ],
        false,
    );
    let observations = [result_observation(first), result_observation(second)];
    if same_batch {
        db.store_mut()
            .upsert_records(SESSION, &observations, false)
            .unwrap();
    } else {
        for observation in &observations {
            db.store_mut()
                .upsert_records(SESSION, std::slice::from_ref(observation), false)
                .unwrap();
        }
    }
    db
}

#[test]
fn a_removed_result_cannot_keep_a_repeated_failure_warning_alive() {
    let mut db = TempDb::empty().unwrap();
    let original = result_observation("two");
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("h", &at(0, 0), "Run both checks please"),
            calls(
                "a",
                &at(0, 1),
                &[
                    ("t1", "Bash", json!({"command":"pnpm test"})),
                    ("t2", "Bash", json!({"command":"pnpm test"})),
                ],
            ),
            original.clone(),
        ],
        false,
    );
    let before = warnings(&db, SESSION);
    assert!(
        before
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::RepeatedFailure)
    );
    assert_eq!(before.checks.repeated_failure, CheckState::Complete);
    let mut removed = original;
    removed.message.content.as_mut().unwrap().pop();
    db.store_mut()
        .upsert_records(SESSION, &[removed], false)
        .unwrap();
    let found = warnings(&db, SESSION);
    assert!(
        found
            .warnings
            .iter()
            .all(|w| w.kind != WarningKind::RepeatedFailure)
    );
    assert_eq!(
        found.checks.repeated_failure,
        CheckState::Incomplete {
            missing: vec![Missing::OutcomeUnknown { results: 1 }]
        }
    );
    assert_eq!((found.tool_results, found.tool_results_known), (2, 1));
}

#[test]
fn result_position_disagreements_are_unknown_in_both_orders() {
    for (first, second) in [
        ("two", "one"),
        ("one", "two"),
        ("two", "replaced"),
        ("replaced", "two"),
    ] {
        for same_batch in [false, true] {
            let db = observed_result_pair(first, second, same_batch);
            let found = warnings(&db, SESSION);
            assert!(
                found
                    .warnings
                    .iter()
                    .all(|w| w.kind != WarningKind::RepeatedFailure),
                "{first} -> {second}, same batch: {same_batch}"
            );
            assert_eq!(
                found.checks.repeated_failure,
                CheckState::Incomplete {
                    missing: vec![Missing::OutcomeUnknown { results: 1 }]
                }
            );
            assert_eq!((found.tool_results, found.tool_results_known), (2, 1));
        }
    }
}

#[test]
fn conflicted_empty_result_observations_never_report_a_complete_check() {
    for (first, second) in [
        ("empty", "two"),
        ("two", "empty"),
        ("other", "two"),
        ("two", "other"),
    ] {
        for same_batch in [false, true] {
            let db = observed_result_pair(first, second, same_batch);
            let parent_conflict: bool = Connection::open(db.path())
                .unwrap()
                .query_row(
                    "SELECT has_conflict FROM records WHERE uuid='r'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(parent_conflict);
            let found = warnings(&db, SESSION);
            assert!(
                found
                    .warnings
                    .iter()
                    .all(|w| w.kind != WarningKind::RepeatedFailure)
            );
            let missing = if found.tool_results == 0 {
                Missing::OutcomeNotSaved { records: 1 }
            } else {
                assert_eq!(found.tool_results, 2);
                Missing::OutcomeUnknown { results: 2 }
            };
            let expected = CheckState::Incomplete {
                missing: vec![missing],
            };
            assert_eq!(
                found.checks.repeated_failure, expected,
                "{first} -> {second}, same batch: {same_batch}"
            );
            assert_eq!(found.checks.interruption, expected);
            assert_eq!(found.tool_results_known, 0);
        }
    }
}

#[test]
fn unknown_content_does_not_dispute_known_failure_evidence() {
    for (first, second) in [("unknown", "two"), ("two", "unknown"), ("two", "two")] {
        for same_batch in [false, true] {
            let db = observed_result_pair(first, second, same_batch);
            let found = warnings(&db, SESSION);
            assert!(
                found
                    .warnings
                    .iter()
                    .any(|w| w.kind == WarningKind::RepeatedFailure)
            );
            assert_eq!(
                found.checks.repeated_failure,
                CheckState::Complete,
                "{first} -> {second}, same batch: {same_batch}"
            );
            assert_eq!((found.tool_results, found.tool_results_known), (2, 2));
        }
    }
}

/// Read one session the way the writer does, with its tool-sent claims, so
/// previews and proofs are kept as ingestion keeps them.
fn ingest(
    db: &mut TempDb,
    id: &str,
    host: &str,
    rows: &[CanonicalRecord],
    tool_sent: &[Option<ToolSentKind>],
    keep_content: bool,
) {
    let mut meta = SessionMeta::new(id, host, SessionSource::ReadersCli);
    meta.native_session_id = Some(id.into());
    meta.surface = Some("cli".into());
    db.store_mut()
        .set_retention_mode(if keep_content {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    let mut batch = IngestBatch::new(&meta, rows, keep_content);
    batch.tool_sent = tool_sent;
    let saved = db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert!(saved.records.iter().all(|r| r.disposition.is_accepted()));
}

#[test]
fn codex_s_proven_interrupted_turn_note_is_an_interruption() {
    let mut db = TempDb::empty().unwrap();
    let rows = [
        says("h1", &at(0, 0), "make the tests pass"),
        reply("a1", &at(0, 1)),
        says(
            "t1",
            &at(0, 2),
            "<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>",
        ),
        says("h2", &at(0, 3), "use the other branch instead"),
    ];
    ingest(
        &mut db,
        "codex-s",
        "codex",
        &rows,
        &[None, None, Some(ToolSentKind::CodexTurnAborted), None],
        false,
    );
    let found = warnings(&db, "codex-s");
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| (w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    assert_eq!(
        interruptions,
        vec![(uuids(&["t1"]), "you interrupted the agent (12:00)")]
    );
    assert_eq!(found.checks.interruption, CheckState::Complete);
    // The proven note is not the person's message.
    assert_eq!(found.person_messages, 2);
}

#[test]
fn a_decline_and_its_marker_at_one_moment_are_one_interruption_in_source_order() {
    let mut db = TempDb::empty().unwrap();
    let rows = [
        says("h1", &at(0, 0), "edit the parser"),
        calls(
            "a1",
            &at(0, 1),
            &[("t1", "Edit", json!({"file_path": "/p.rs"}))],
        ),
        result(
            "z-decline",
            &at(0, 2),
            "t1",
            Some(true),
            "The user doesn't want to proceed with this tool use. The tool use was rejected.",
        ),
        says(
            "a-marker",
            &at(0, 2),
            "[Request interrupted by user for tool use]",
        ),
    ];
    seed(&mut db, SESSION, "claude", &rows, false);
    // Claude Code writes the marker after the result it follows.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "UPDATE records SET parent_uuid='z-decline' WHERE uuid='a-marker'",
            [],
        )
        .unwrap();
    let found = warnings(&db, SESSION);
    let interruptions: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::Interruption)
        .map(|w| (w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    assert_eq!(
        interruptions,
        vec![(
            uuids(&["z-decline", "a-marker"]),
            "you declined a call to Edit (12:00)"
        )]
    );
}

#[test]
fn an_interrupted_call_and_a_permission_denial_are_not_failures() {
    let mut db = TempDb::empty().unwrap();
    let bash = json!({"command": "cargo test"});
    let rows = [
        says("h1", &at(0, 0), "run the tests please"),
        calls("a1", &at(0, 1), &[("t1", "Bash", bash.clone())]),
        result(
            "r1",
            &at(0, 2),
            "t1",
            Some(true),
            "[Request interrupted by user for tool use]",
        ),
        reply("a2", &at(0, 3)),
        calls("a3", &at(0, 4), &[("t2", "Bash", bash.clone())]),
        result(
            "r2",
            &at(0, 5),
            "t2",
            Some(true),
            "Permission for this action was denied. Reason: outside the project.",
        ),
        calls("a4", &at(0, 6), &[("t3", "Bash", bash)]),
        result(
            "r3",
            &at(0, 7),
            "t3",
            Some(true),
            "Permission to use Bash has been denied because Claude Code is running in don't ask mode.",
        ),
        reply("a5", &at(0, 8)),
    ];
    seed(&mut db, SESSION, "claude", &rows, false);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone(), w.reason.as_str()))
        .collect();
    // The interrupted call is the person's interruption; the two denials by
    // the host's own check are neither a decline nor a repeated failure.
    assert_eq!(
        summary,
        vec![(
            WarningKind::Interruption,
            uuids(&["r1"]),
            "you interrupted a call to Bash (12:00)"
        )]
    );
    assert_eq!(found.checks.repeated_failure, CheckState::Complete);
    assert_eq!(found.checks.interruption, CheckState::Complete);
    assert_eq!((found.tool_results, found.tool_results_known), (3, 3));
}

/// Two prompts identical for their first 280 characters and different after,
/// and a reply's correction that comes after character 280.
fn long_messages() -> Vec<CanonicalRecord> {
    let opening = "please look at the module layout and the tests ".repeat(7);
    assert!(opening.chars().count() > 300);
    vec![
        says("h1", &at(0, 0), &format!("{opening} then fix the parser")),
        reply("a1", &at(0, 1)),
        says("h2", &at(0, 2), &format!("{opening} that is wrong")),
        reply("a2", &at(0, 3)),
        says("h3", &at(0, 4), &format!("{opening} then fix the lexer")),
        reply("a3", &at(0, 5)),
    ]
}

#[test]
fn whole_kept_messages_are_read_past_the_preview_length() {
    let mut db = TempDb::empty().unwrap();
    ingest(&mut db, SESSION, "claude", &long_messages(), &[], true);
    let found = warnings(&db, SESSION);
    let summary: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (w.kind, w.record_uuids.clone()))
        .collect();
    // The correction after character 280 is found, and the two prompts that
    // differ only after it are not the same prompt.
    assert_eq!(summary, vec![(WarningKind::Correction, uuids(&["h2"]))]);
    assert_eq!(found.checks.correction, CheckState::Complete);
    assert_eq!(found.checks.repeated_prompt, CheckState::Complete);
    assert_eq!((found.person_messages, found.person_messages_read), (3, 3));
}

#[test]
fn a_cut_preview_never_reads_as_the_whole_message() {
    let mut db = TempDb::empty().unwrap();
    ingest(&mut db, SESSION, "claude", &long_messages(), &[], false);
    let found = warnings(&db, SESSION);
    // Only the previews' first 280 characters were kept: no correction is
    // found there, the prompts look the same there, and neither rule says
    // it read everything.
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    // The two messages sent after a reply.
    assert_eq!(
        found.checks.correction,
        CheckState::Incomplete {
            missing: vec![Missing::TextCut { messages: 2 }]
        }
    );
    assert_eq!(
        found.checks.repeated_prompt,
        CheckState::Incomplete {
            missing: vec![Missing::TextCut { messages: 3 }]
        }
    );
    assert_eq!((found.person_messages, found.person_messages_read), (3, 0));
}

fn whole(db: &TempDb, ids: &[&str]) -> std::collections::BTreeMap<String, WholeSessionWarnings> {
    MetricsDb::open(db.path())
        .unwrap()
        .whole_session_warnings(ids, &TimeZone::UTC)
        .unwrap()
}

fn read(answer: &WholeSessionWarnings) -> &SessionWarnings {
    match answer {
        WholeSessionWarnings::Read { warnings, .. } => warnings,
        other => panic!("not read: {other:?}"),
    }
}

#[test]
fn a_whole_session_read_is_the_window_read_over_the_session_s_own_history() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    // A second session on the same surface weeks earlier, and an untimed
    // one, so the request mixes every answer.
    let earlier = |minute: u32| format!("2026-08-01T09:{minute:02}:00Z");
    seed(
        &mut db,
        "older",
        "claude",
        &[
            says("o1", &earlier(0), "please rename the module"),
            reply("o2", &earlier(1)),
            says("o3", &earlier(2), "no, the other module"),
            reply("o4", &earlier(3)),
        ],
        true,
    );
    let answers = whole(&db, &[SESSION, "older", SESSION, "nobody"]);
    assert_eq!(
        answers.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["nobody", "older", SESSION]
    );
    assert_eq!(answers["nobody"], WholeSessionWarnings::Missing);
    // The same answer the window read gives over a window that holds all of
    // the session, and it carries the session's own clock times.
    assert_eq!(read(&answers[SESSION]), &warnings(&db, SESSION));
    assert_eq!(
        answers[SESSION],
        WholeSessionWarnings::Read {
            warnings: warnings(&db, SESSION),
            untimed_records: 0
        }
    );
    // The older session's correction is found although it is outside the
    // seven days the window read covers, where it is not even listed.
    let older = read(&answers["older"]);
    assert_eq!(older.warnings.len(), 1);
    assert_eq!(older.warnings[0].kind, WarningKind::Correction);
    assert_eq!(
        older.warnings[0].reason,
        "you wrote \"no\" after a reply (09:02)"
    );
    assert!(
        MetricsDb::open(db.path())
            .unwrap()
            .session_warnings(window(), &TimeZone::UTC)
            .unwrap()
            .iter()
            .all(|s| s.session_id != "older")
    );
    // Asked alone, a session gets the same answer as among others.
    assert_eq!(whole(&db, &["older"])["older"], answers["older"]);
    assert_eq!(whole(&db, &[SESSION])[SESSION], answers[SESSION]);
}

#[test]
fn a_session_with_no_timed_record_is_said_to_be_untimed() {
    let mut db = TempDb::empty().unwrap();
    let mut untimed = says("u1", &at(0, 0), "please do the thing");
    untimed.timestamp = None;
    seed(&mut db, "untimed", "claude", &[untimed], true);
    let mut late = says("u2", &at(0, 0), "and this one");
    late.timestamp = None;
    seed(
        &mut db,
        "partly",
        "claude",
        &[says("p1", &at(0, 0), "please do the thing"), late],
        true,
    );
    let answers = whole(&db, &["untimed", "partly"]);
    assert_eq!(
        answers["untimed"],
        WholeSessionWarnings::Untimed { untimed_records: 1 }
    );
    let WholeSessionWarnings::Read {
        untimed_records, ..
    } = answers["partly"]
    else {
        panic!("read");
    };
    assert_eq!(untimed_records, 1);
}

#[test]
fn a_circling_stretch_of_agent_calls_is_never_a_warning() {
    let mut db = TempDb::empty().unwrap();
    let task = json!({"description": "review", "prompt": "review the diff"});
    let message = json!({"to": "reviewer", "message": "status?"});
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("h1", &at(0, 0), "run the reviewers please"),
            calls(
                "a1",
                &at(0, 30),
                &[
                    ("t1", "Agent", task.clone()),
                    ("t2", "Agent", task.clone()),
                    ("t3", "Agent", task.clone()),
                    ("t4", "SendMessage", message.clone()),
                    ("t5", "SendMessage", message.clone()),
                    ("t6", "SendMessage", message.clone()),
                    ("t7", "Agent", task),
                    ("t8", "SendMessage", message),
                ],
            ),
            reply("a2", &at(5, 0)),
        ],
        true,
    );
    let answers = whole(&db, &[SESSION]);
    let found = read(&answers[SESSION]);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(
        found.checks.get(WarningKind::RepeatedFailure),
        &CheckState::Complete
    );
    // M-20, the Dashboard's number, still counts them.
    let SessionRepeats::Measured { stretches, .. } = MetricsDb::open(db.path())
        .unwrap()
        .session_repeats(window(), SESSION, RepeatThresholds::default())
        .unwrap()
    else {
        panic!("measured");
    };
    assert_eq!(stretches.len(), 1);
    assert_eq!(stretches[0].circling, Some(true));
}

#[test]
fn every_warning_of_a_session_has_its_own_identity() {
    let mut db = TempDb::empty().unwrap();
    let declined =
        "The user doesn't want to proceed with this tool use. The tool use was rejected.";
    let edit = |path: &str| json!({"file_path": path});
    let result_blocks = |uuid: &str, time: &str, results: &[(&str, bool, &str)]| {
        let content: Vec<Value> = results
            .iter()
            .map(|(call, failed, text)| {
                json!({"type":"tool_result","tool_use_id":call,"is_error":failed,"content":text})
            })
            .collect();
        record(json!({"uuid":uuid,"type":"user","timestamp":time,
            "message":{"role":"user","content":content}}))
    };
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("h1", &at(0, 0), "edit both files please"),
            // Two calls in one record, both declined in one result record:
            // two interruptions on the same record.
            calls(
                "a1",
                &at(0, 1),
                &[("t1", "Edit", edit("/a.rs")), ("t2", "Edit", edit("/b.rs"))],
            ),
            result_blocks(
                "r1",
                &at(0, 2),
                &[("t1", true, declined), ("t2", true, declined)],
            ),
            // Two different commands failing together, twice: two repeated
            // failures whose first result is the same record.
            calls(
                "a2",
                &at(0, 3),
                &[
                    ("t3", "Bash", json!({"command": "make"})),
                    ("t4", "Bash", json!({"command": "make test"})),
                ],
            ),
            result_blocks(
                "r2",
                &at(0, 4),
                &[("t3", true, "failed"), ("t4", true, "failed")],
            ),
            calls(
                "a3",
                &at(0, 5),
                &[
                    ("t5", "Bash", json!({"command": "make"})),
                    ("t6", "Bash", json!({"command": "make test"})),
                ],
            ),
            result_blocks(
                "r3",
                &at(0, 6),
                &[("t5", true, "failed"), ("t6", true, "failed")],
            ),
            reply("a4", &at(0, 7)),
        ],
        true,
    );
    let answers = whole(&db, &[SESSION]);
    let found = read(&answers[SESSION]);
    let kinds: Vec<_> = found.warnings.iter().map(|w| w.kind).collect();
    assert_eq!(
        kinds,
        vec![
            WarningKind::Interruption,
            WarningKind::Interruption,
            WarningKind::RepeatedFailure,
            WarningKind::RepeatedFailure
        ]
    );
    let ids: Vec<_> = found.warnings.iter().map(|w| w.id()).collect();
    assert_eq!(
        ids.iter().collect::<BTreeSet<_>>().len(),
        ids.len(),
        "{ids:?}"
    );
    assert_eq!(ids[0], "interruption:r1#0");
    assert_eq!(ids[1], "interruption:r1#1");
    assert!(ids[2].starts_with("repeated_failure:r2#"), "{ids:?}");
    // Read again, the identities are the same.
    let again: Vec<_> = read(&whole(&db, &[SESSION])[SESSION])
        .warnings
        .iter()
        .map(|w| w.id())
        .collect();
    assert_eq!(again, ids);
}

#[test]
fn a_window_lists_only_its_own_warnings_of_whole_session_answers() {
    let mut db = TempDb::empty().unwrap();
    // One session across two weeks: a correction on Aug 30, outside the
    // window, and the conversation's warnings on Sep 7, inside it.
    let mut rows = vec![
        says("e1", "2026-08-30T09:00:00Z", "please rename the module"),
        reply("e2", "2026-08-30T09:01:00Z"),
        says("e3", "2026-08-30T09:02:00Z", "no, the other module"),
        reply("e4", "2026-08-30T09:03:00Z"),
    ];
    rows.extend(conversation());
    seed(&mut db, SESSION, "claude", &rows, true);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let inside = metrics
        .warnings_in_window(window(), &TimeZone::UTC)
        .unwrap();
    let whole = whole(&db, &[SESSION]);
    let all = &read(&whole[SESSION]).warnings;
    assert_eq!(all[0].record_uuids, uuids(&["e3"]));
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].session_id, SESSION);
    // The same warnings, the one before the window left out.
    assert_eq!(&inside[0].warnings[..], &all[1..]);
    // The session's last timed record, the version its title is read for.
    assert_eq!(inside[0].last_ms, ms("2026-09-07T12:11:30Z"));
}

#[test]
fn a_batch_and_a_window_read_give_each_session_the_answer_it_gets_alone() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    // A session that lives for five weeks, so every group's whole windows
    // reach back to it.
    seed(
        &mut db,
        "long",
        "claude",
        &[
            says("l1", "2026-08-01T09:00:00Z", "please rename the module"),
            reply("l2", "2026-08-01T09:01:00Z"),
            says("l3", "2026-08-01T09:02:00Z", "no, the other module"),
            reply("l4", "2026-08-01T09:03:00Z"),
            says("l5", "2026-09-07T12:30:00Z", "and now the tests"),
            reply("l6", "2026-09-07T12:31:00Z"),
        ],
        true,
    );
    // Sessions on the same surface whose records all state one instant,
    // inside the conversation's window, answered in a batch exactly as alone.
    for index in 0..4 {
        let rows: Vec<_> = (0..6)
            .map(|n| says(&format!("d{index}-{n}"), &at(0, 30), "the same instant"))
            .collect();
        seed(
            &mut db,
            &format!("degenerate-{index}"),
            "claude",
            &rows,
            true,
        );
    }
    // Enough short sessions over August that the identifiers are looked up in
    // more than one group.
    let mut ids = vec![
        SESSION.to_owned(),
        "long".to_owned(),
        "degenerate-0".to_owned(),
        "degenerate-1".to_owned(),
        "degenerate-2".to_owned(),
        "degenerate-3".to_owned(),
    ];
    for index in 0..200 {
        let id = format!("m{index:03}");
        let time = |minute: u32| format!("2026-08-{:02}T10:{minute:02}:00Z", 2 + index % 27);
        seed(
            &mut db,
            &id,
            "claude",
            &[
                says(&format!("{id}-1"), &time(0), "please fix the parser"),
                reply(&format!("{id}-2"), &time(1)),
                says(&format!("{id}-3"), &time(2), "no, the other parser"),
                reply(&format!("{id}-4"), &time(3)),
            ],
            true,
        );
        ids.push(id);
    }
    ids.sort();
    let alone = |id: &str| whole(&db, &[id]).remove(id).unwrap();
    // A batch of the most a request may name, with the long-lived session.
    let mut named: Vec<&str> = ids.iter().map(String::as_str).rev().take(199).collect();
    named.push("long");
    assert_eq!(named.len(), 200);
    assert!(named.contains(&"long") && named.contains(&SESSION));
    let batch = whole(&db, &named);
    for id in &named {
        assert_eq!(batch[*id], alone(id), "{id}");
    }
    // A window over everyone: one snapshot for 206 sessions, each answer the
    // session's own, only the warnings inside kept.
    let wide = Window::new(ms("2026-08-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap();
    let inside = MetricsDb::open(db.path())
        .unwrap()
        .warnings_in_window(wide, &TimeZone::UTC)
        .unwrap();
    let mut expected = Vec::new();
    for id in &ids {
        let kept: Vec<_> = read(&alone(id))
            .warnings
            .iter()
            .filter(|w| w.at_ms >= wide.start_ms() && w.at_ms < wide.end_ms())
            .cloned()
            .collect();
        if !kept.is_empty() {
            expected.push((id.clone(), kept));
        }
    }
    assert!(expected.len() > 200, "{}", expected.len());
    let inside: Vec<_> = inside
        .into_iter()
        .map(|found| (found.session_id, found.warnings))
        .collect();
    assert_eq!(inside, expected);
}

fn warm_saved(db: &mut TempDb, force: bool) {
    let candidate = MetricsDb::open(db.path())
        .unwrap()
        .prepare_session_warning_refresh(&if force {
            vec![SESSION.to_owned()]
        } else {
            vec![]
        })
        .unwrap()
        .unwrap();
    assert!(
        db.store_mut()
            .publish_session_warnings(&candidate.rows)
            .unwrap()
            .rejected
            == 0
    );
}

#[test]
fn saved_answers_round_trip_every_rule_state_point_size_and_requested_zone() {
    for start in [
        "2026-09-07T12:00:00Z",
        "2026-03-08T09:58:00Z",
        "2026-11-01T08:58:00Z",
    ] {
        let mut db = TempDb::empty().unwrap();
        let delta = ms(start) - ms(&at(0, 0));
        let mut rows = conversation();
        rows.push(says("marker", &at(12, 0), "[Request interrupted by user]"));
        rows.push(calls(
            "interrupt-call",
            &at(13, 0),
            &[(
                "interrupt-tool",
                "Bash",
                json!({"command":"echo synthetic"}),
            )],
        ));
        rows.push(result(
            "interrupt-result",
            &at(13, 1),
            "interrupt-tool",
            Some(true),
            "[Request interrupted by user for tool use]",
        ));
        rows.push(says(
            "next-day-prompt",
            "2026-09-08T12:00:00Z",
            "Fix the build please",
        ));
        for row in &mut rows {
            row.timestamp = Some(
                Timestamp::from_millisecond(ms(row.timestamp.as_deref().unwrap()) + delta)
                    .unwrap()
                    .to_string(),
            );
        }
        seed(&mut db, SESSION, "claude", &rows, true);
        let mut untimed = says("untimed-r", &at(0, 0), "a synthetic untimed input");
        untimed.timestamp = None;
        seed(&mut db, "untimed", "claude", &[untimed], true);
        seed(&mut db, "empty", "claude", &[], true);
        seed(
            &mut db,
            "quiet",
            "claude",
            &[says("quiet-r", start, "a single synthetic request")],
            true,
        );
        warm_saved(&mut db, false);
        let metrics = MetricsDb::open(db.path()).unwrap();
        assert!(
            metrics
                .prepare_session_warning_refresh(&[])
                .unwrap()
                .is_none()
        );
        for name in ["UTC", "America/Los_Angeles", "Asia/Tokyo"] {
            let zone = TimeZone::get(name).unwrap();
            let ids = [SESSION, "untimed", "empty", "quiet", "missing"];
            let computed = metrics.whole_session_warnings(&ids, &zone).unwrap();
            let saved = metrics.saved_whole_session_warnings(&ids, &zone).unwrap();
            assert_eq!(saved, computed, "{start} {name}");
            let start = ms(start);
            let window = Window::new(start - 1000, start + 2 * 86_400_000).unwrap();
            assert_eq!(
                metrics.saved_warnings_in_window(window, &zone).unwrap(),
                metrics.warnings_in_window(window, &zone).unwrap()
            );
            assert!(!metrics.session_warning_refresh_needed());
        }
        let sql = Connection::open(db.path()).unwrap();
        let payload: String = sql
            .query_row(
                "SELECT payload_json FROM saved_session_warnings WHERE session_id=?1",
                [SESSION],
                |r| r.get(0),
            )
            .unwrap();
        for forbidden in [
            "Fix the build please",
            "pnpm test",
            "America/Los_Angeles",
            "tool_use_id",
            "group_key",
        ] {
            assert!(!payload.contains(forbidden), "{forbidden}");
        }
        let value: Value = serde_json::from_str(&payload).unwrap();
        let kinds: BTreeSet<_> = value["answer"]["warnings"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["reason"]["reason"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            BTreeSet::from([
                "correction",
                "interrupted",
                "stopped_call",
                "repeated_prompt",
                "repeated_failure",
            ])
        );
    }
}

#[test]
fn stale_missing_corrupt_and_new_version_answers_compute_then_repair() {
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    let computed = whole(&db, &[SESSION]);
    let cold = MetricsDb::open(db.path()).unwrap();
    assert_eq!(
        cold.saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        computed
    );
    assert!(cold.session_warning_refresh_needed());
    warm_saved(&mut db, false);
    for mutation in [
        "DELETE FROM saved_session_warnings",
        "UPDATE saved_session_warnings SET payload_json='{}'",
        "UPDATE saved_session_warnings SET payload_json=json_set(payload_json,'$.version',999)",
        "UPDATE saved_session_warnings SET rule_version=999",
    ] {
        Connection::open(db.path())
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        assert_eq!(
            metrics
                .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
                .unwrap(),
            computed,
            "{mutation}"
        );
        assert!(metrics.session_warning_refresh_needed());
        warm_saved(&mut db, !metrics.session_warning_repairs().is_empty());
        assert!(
            MetricsDb::open(db.path())
                .unwrap()
                .session_warning_cache_is_current()
                .unwrap()
        );
    }
    // A direct canonical writer changes old records without changing counts or timestamps.
    let before = MetricsDb::open(db.path())
        .unwrap()
        .prepare_session_warning_refresh(&[SESSION.to_owned()])
        .unwrap()
        .unwrap();
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=0 WHERE uuid='h2'", [])
        .unwrap();
    assert!(
        db.store_mut()
            .publish_session_warnings(&before.rows)
            .unwrap()
            .rejected
            > 0
    );
    let stale = MetricsDb::open(db.path()).unwrap();
    assert!(!stale.session_warning_cache_is_current().unwrap());
    assert_eq!(
        stale
            .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        whole(&db, &[SESSION])
    );
    warm_saved(&mut db, false);
}

#[test]
fn another_sessions_timestamp_health_no_longer_changes_the_answer() {
    // Only the removed repeated-calls rule read the surface's clock health;
    // the four rules read the session alone, so the answer stays the same
    // and the refreshed saved answer is that same answer.
    let mut db = TempDb::empty().unwrap();
    seed(&mut db, SESSION, "claude", &conversation(), true);
    warm_saved(&mut db, false);
    let before = MetricsDb::open(db.path())
        .unwrap()
        .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
        .unwrap();
    for index in 0..4 {
        let rows: Vec<_> = (0..6)
            .map(|n| {
                says(
                    &format!("health-{index}-{n}"),
                    &at(0, 30),
                    "the same instant",
                )
            })
            .collect();
        seed(&mut db, &format!("health-{index}"), "claude", &rows, true);
    }
    let oracle = whole(&db, &[SESSION]);
    assert_eq!(before, oracle);
    warm_saved(&mut db, false);
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        oracle
    );
}

#[test]
fn saved_two_failures_in_one_record_keep_the_second_block_and_growth_count() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("h", &at(0, 0), "fix the synthetic tests please"),
            calls(
                "calls",
                &at(0, 1),
                &[
                    ("c1", "Bash", json!({"command":"test"})),
                    ("c2", "Bash", json!({"command":"test"})),
                ],
            ),
            record(
                json!({"uuid":"results","type":"user","timestamp":at(0,2),"message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1","is_error":true,"content":"failed"},
                    {"type":"tool_result","tool_use_id":"c2","is_error":true,"content":"failed"}
                ]}}),
            ),
        ],
        true,
    );
    warm_saved(&mut db, false);
    let before = MetricsDb::open(db.path())
        .unwrap()
        .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
        .unwrap();
    let warning = read(&before[SESSION])
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::RepeatedFailure)
        .unwrap();
    assert_eq!(
        warning.points_at(),
        xt_metrics::WarningPoint {
            record_uuid: "results".into(),
            block: Some(1)
        }
    );
    assert_eq!(warning.size, Some(2));
    for mutation in [
        "UPDATE saved_session_warnings SET payload_json=json_remove(payload_json,'$.answer.warnings.warnings[0].repeat','$.answer.warnings.warnings[0].size')",
        "UPDATE saved_session_warnings SET payload_json=json_set(payload_json,'$.answer.warnings.warnings[0].kind','correction')",
    ] {
        Connection::open(db.path())
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        let corrupted = MetricsDb::open(db.path()).unwrap();
        assert_eq!(
            corrupted
                .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
                .unwrap(),
            before
        );
        assert!(!corrupted.session_warning_repairs().is_empty());
        warm_saved(&mut db, true);
    }
    db.store_mut()
        .dismiss_warning(SESSION, &warning.id(), warning.size, 1)
        .unwrap();
    db.store_mut()
        .upsert_records(
            SESSION,
            &[
                calls(
                    "more-call",
                    &at(1, 0),
                    &[("c3", "Bash", json!({"command":"test"}))],
                ),
                result("more-result", &at(1, 1), "c3", Some(true), "failed"),
            ],
            true,
        )
        .unwrap();
    warm_saved(&mut db, false);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let after = metrics
        .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
        .unwrap();
    let grown = read(&after[SESSION])
        .warnings
        .iter()
        .find(|w| w.id() == warning.id())
        .unwrap();
    assert_eq!(grown.size, Some(3));
    assert_eq!(grown.points_at(), warning.points_at());
    assert_eq!(
        metrics.dismissed_warnings(&[SESSION]).unwrap()[SESSION][&warning.id()].size,
        Some(2)
    );
    assert!(
        db.store_mut()
            .restore_warning(SESSION, &warning.id())
            .unwrap()
    );
    assert!(metrics.dismissed_warnings(&[SESSION]).unwrap().is_empty());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        after
    );
}

#[test]
fn content_purge_clears_saved_correction_span_and_rejects_its_late_candidate() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        SESSION,
        "claude",
        &[
            says("purge-person", &at(0, 0), "please fix the test"),
            reply("purge-agent", &at(1, 0)),
            says(
                "purge-correction",
                &at(2, 0),
                "still cachepurgeuniqueword not working",
            ),
        ],
        true,
    );
    warm_saved(&mut db, false);
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert!(metrics.session_warning_cache_is_current().unwrap());
    let sql = Connection::open(db.path()).unwrap();
    let saved: String = sql
        .query_row(
            "SELECT payload_json FROM saved_session_warnings WHERE session_id=?1",
            [SESSION],
            |row| row.get(0),
        )
        .unwrap();
    assert!(saved.contains("still cachepurgeuniqueword not"));
    // A current cache normally needs no work: force this identifier to hold a
    // real, nonempty pre-purge candidate at the source's current revision.
    let candidate = metrics
        .prepare_session_warning_refresh(&[SESSION.to_owned()])
        .unwrap()
        .unwrap();
    assert_eq!(candidate.rows.len(), 1);
    assert!(
        candidate.rows[0]
            .payload_json
            .contains("cachepurgeuniqueword")
    );
    let before_revision: i64 = sql
        .query_row(
            "SELECT revision FROM session_warning_inputs WHERE session_id=?1",
            [SESSION],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(candidate.rows[0].revision, before_revision);
    let purged = db
        .store_mut()
        .purge_content(&xt_store::retention::ContentRegistry::default())
        .unwrap();
    assert!(purged.invalidate_content);
    assert_eq!(
        purged
            .tables
            .iter()
            .find(|table| table.table == "saved_session_warnings")
            .unwrap()
            .rows,
        1
    );
    // No refresh worker exists in this test. The rows are gone at commit.
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let after_revision: i64 = sql
        .query_row(
            "SELECT revision FROM session_warning_inputs WHERE session_id=?1",
            [SESSION],
            |row| row.get(0),
        )
        .unwrap();
    assert!(after_revision > before_revision);
    let late = db
        .store_mut()
        .publish_session_warnings(&candidate.rows)
        .unwrap();
    assert_eq!((late.published, late.rejected), (0, 1));
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let fresh = metrics
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap();
    assert_eq!(fresh.rows.len(), 1);
    assert!(!fresh.rows[0].payload_json.contains("cachepurgeuniqueword"));
}

#[test]
fn polling_a_running_job_is_never_a_warning_and_an_old_cache_is_refreshed() {
    let mut db = TempDb::empty().unwrap();
    let mut rows = vec![says("human-start", &at(0, 0), "start the updater tests")];
    for n in 1..=12 {
        rows.push(reply(&format!("setup-{n}"), &at(0, n)));
    }
    let expected: Vec<_> = (0..20)
        .map(|n| (format!("todo-{n}"), "TodoWrite", json!({"todos":[]})))
        .collect();
    let expected: Vec<_> = expected
        .iter()
        .map(|(id, name, input)| (id.as_str(), *name, input.clone()))
        .collect();
    rows.push(calls("expected-tools", &at(1, 0), &expected));
    let repeated: Vec<_> = (0..6)
        .map(|n| {
            (
                format!("poll-{n}"),
                "write_stdin",
                json!({"session_id":123,"chars":""}),
            )
        })
        .collect();
    let repeated: Vec<_> = repeated
        .iter()
        .map(|(id, name, input)| (id.as_str(), *name, input.clone()))
        .collect();
    rows.push(calls("actual-repeated-call", &at(2, 0), &repeated));
    rows.push(reply("finish", &at(6, 0)));
    seed(&mut db, SESSION, "claude", &rows, true);
    let computed = whole(&db, &[SESSION]);
    // Six polls of one running job and twenty task-list updates: once a
    // repeated-calls warning, now no warning at all.
    assert!(
        read(&computed[SESSION]).warnings.is_empty(),
        "{:?}",
        read(&computed[SESSION]).warnings
    );
    // M-20 itself still measures the stretch.
    let all = MetricsDb::open(db.path())
        .unwrap()
        .session_repeats(window(), SESSION, RepeatThresholds::default())
        .unwrap();
    let SessionRepeats::Measured { stretches, .. } = all else {
        panic!("repeats")
    };
    let xt_metrics::RepeatDensity::Measured {
        worst: Some(worst), ..
    } = &stretches[0].repeats
    else {
        panic!("worst")
    };
    assert_eq!(worst.tool_name, "TodoWrite");
    warm_saved(&mut db, false);
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE saved_session_warnings SET rule_version=1", [])
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert!(!metrics.session_warning_cache_is_current().unwrap());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        computed
    );
    warm_saved(&mut db, false);
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert!(metrics.session_warning_cache_is_current().unwrap());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&[SESSION], &TimeZone::UTC)
            .unwrap(),
        computed
    );
}
