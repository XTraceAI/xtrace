//! Which admission phrases each agent reply opens with is kept as metadata
//! at write time (migration 30): fixed phrase names only, never reply text,
//! whatever the content retention setting says. Synthetic disposable
//! databases only.
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store, batch::NativeCheckpoint,
    retention::RetentionMode,
};

fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn reply(uuid: &str, second: u32, content: Value) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant",
        "timestamp":format!("2026-09-07T12:00:{second:02}Z"),
        "message":{"role":"assistant","content":content}}))
}
fn text(uuid: &str, second: u32, words: &str) -> CanonicalRecord {
    reply(uuid, second, json!([{"type":"text","text":words}]))
}
fn person(uuid: &str, second: u32, words: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user",
        "timestamp":format!("2026-09-07T12:00:{second:02}Z"),
        "message":{"role":"user","content":[{"type":"text","text":words}]}}))
}

fn store(directory: &tempfile::TempDir, keep: bool) -> (Store, std::path::PathBuf) {
    let path = directory.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .set_retention_mode(if keep {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    store
        .upsert_session(
            &SessionMeta::new("s", "claude", SessionSource::Transcript),
            false,
        )
        .unwrap();
    (store, path)
}

/// `(record, phrases, fact_version, conflicted)` for every kept row.
type Row = (String, String, i64, bool);
fn rows(path: &Path) -> Vec<Row> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT record_uuid,phrases,fact_version,conflicted FROM reply_admissions
             ORDER BY record_uuid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn row(uuid: &str, phrases: &str) -> Row {
    (uuid.into(), phrases.into(), 1, false)
}
fn revision(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT revision FROM session_warning_inputs WHERE session_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn session() -> Vec<CanonicalRecord> {
    let mut side = text("side", 9, "Sorry, you're right.");
    side.is_sidechain = true;
    vec![
        person("p1", 0, "Sorry, the parser is wrong"),
        text("a1", 1, "You\u{2019}re right, my mistake. Sorry!"),
        reply(
            "a2",
            2,
            json!([{"type":"thinking","thinking":"sorry, I was wrong"},
                   {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]),
        ),
        reply(
            "a3",
            3,
            json!([{"type":"text","text":"I wrongly and incorrectly "},
                   {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls"}},
                   {"type":"text","text":"assumed it."}]),
        ),
        reply("a4", 4, json!([{"type":"text","text":""}])),
        side,
    ]
}

#[test]
fn every_reply_with_text_gets_its_phrase_names_and_nothing_else_does() {
    for keep in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, path) = store(&directory, keep);
        store.upsert_records("s", &session(), false).unwrap();
        // A reply with text gets a row, empty when no phrase is found.
        // Thinking and tool calls only, empty text, the person's message and
        // the sidechain reply get none.
        assert_eq!(
            rows(&path),
            vec![row("a1", "you_are_right my_mistake sorry"), row("a3", "")],
            "keep {keep}"
        );
    }
}

#[test]
fn rows_are_exactly_the_records_with_reply_text_after_a_re_read() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    store.upsert_records("s", &session(), false).unwrap();
    let before = (rows(&path), revision(&path));
    // Reading the same source again changes no row and no revision.
    store.upsert_records("s", &session(), false).unwrap();
    assert_eq!((rows(&path), revision(&path)), before);
    // The rows are exactly the agent records outside sidechains whose stored
    // text length is above zero: the test the warning rule picks a reply by.
    let qualifying: Vec<String> = Connection::open(&path)
        .unwrap()
        .prepare(
            "SELECT uuid FROM records WHERE session_id='s' AND type='assistant'
               AND is_sidechain=0 AND text_len>0 ORDER BY uuid",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let kept: Vec<String> = rows(&path).into_iter().map(|r| r.0).collect();
    assert_eq!(kept, qualifying);
}

#[test]
fn records_read_before_content_was_seen_gain_a_row_when_read_again() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    // Indexed without content: nothing is known.
    let mut bare = text("a1", 1, "Good catch.");
    bare.message.content = None;
    store.upsert_records("s", &[bare], false).unwrap();
    assert!(rows(&path).is_empty());
    let before = revision(&path);
    store
        .upsert_records("s", &[text("a1", 1, "Good catch.")], false)
        .unwrap();
    assert_eq!(rows(&path), vec![row("a1", "good_catch")]);
    // A new fact changes the session's saved warnings.
    assert!(revision(&path) > before);
}

#[test]
fn other_phrases_under_the_same_version_mark_the_row_conflicted() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    store
        .upsert_records("s", &[text("a1", 1, "Good catch.")], false)
        .unwrap();
    let before = revision(&path);
    // The same record observed again with other words.
    store
        .upsert_records("s", &[text("a1", 1, "Fair point.")], false)
        .unwrap();
    assert_eq!(
        rows(&path),
        vec![("a1".into(), "good_catch".into(), 1, true)]
    );
    assert!(revision(&path) > before);
}

#[test]
fn the_table_refuses_text_and_unknown_shapes() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    store
        .upsert_records("s", &[text("a1", 1, "ok")], false)
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.execute("PRAGMA foreign_keys=ON", []).unwrap();
    for (phrases, version, conflicted) in [
        ("You're right", 1, 0),
        ("sorry, again", 1, 0),
        ("sorry2", 1, 0),
        ("SORRY", 1, 0),
        ("sorry", 0, 0),
        ("sorry", 1, 2),
    ] {
        connection
            .execute("DELETE FROM reply_admissions", [])
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO reply_admissions(record_uuid,session_id,phrases,fact_version,
                       conflicted) VALUES('a1','s',?1,?2,?3)",
                    rusqlite::params![phrases, version, conflicted],
                )
                .is_err(),
            "{phrases:?} {version} {conflicted}"
        );
    }
    let long = "sorry ".repeat(101);
    assert!(
        connection
            .execute(
                "INSERT INTO reply_admissions(record_uuid,session_id,phrases,fact_version)
                 VALUES('a1','s',?1,1)",
                [long.trim_end()],
            )
            .is_err()
    );
    // A row must name a record of its session.
    assert!(
        connection
            .execute(
                "INSERT INTO reply_admissions(record_uuid,session_id,phrases,fact_version)
                 VALUES('a1','other','sorry',1)",
                [],
            )
            .is_err()
    );
}

#[test]
fn every_change_to_a_row_moves_the_session_s_revision() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    store
        .upsert_records("s", &[text("a1", 1, "Good catch.")], false)
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    for change in [
        "UPDATE reply_admissions SET phrases='sorry'",
        "UPDATE reply_admissions SET fact_version=2",
        "UPDATE reply_admissions SET conflicted=1",
        "DELETE FROM reply_admissions",
        "INSERT INTO reply_admissions(record_uuid,session_id,phrases,fact_version)
         VALUES('a1','s','',1)",
    ] {
        let before = revision(&path);
        connection.execute(change, []).unwrap();
        assert!(revision(&path) > before, "{change}");
    }
    // An update that changes nothing moves nothing.
    let before = revision(&path);
    connection
        .execute("UPDATE reply_admissions SET phrases=phrases", [])
        .unwrap();
    assert_eq!(revision(&path), before);
}

/// Migration 30 on a store left at 29: older dismissals stay, the new kind is
/// accepted, and the Claude transcript checkpoints are cleared so the next
/// scan reads every transcript again.
#[test]
fn migration_30_keeps_dismissals_and_clears_claude_checkpoints() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .dismiss_warning("s", "repeated_prompt:p1", Some(2), 10)
        .unwrap();
    store
        .dismiss_warning("s", "correction:p2", None, 20)
        .unwrap();
    let transcript = NativeCheckpoint {
        source: SessionSource::Transcript,
        cursor_key: "claude:/synthetic/session.jsonl".into(),
        generation: "{}".into(),
        position: 4096,
        updated_at: 1,
    };
    let reader = NativeCheckpoint {
        source: SessionSource::ReadersCli,
        cursor_key: "codex:/synthetic/rollout.jsonl".into(),
        ..transcript.clone()
    };
    store.record_native_checkpoint(&transcript).unwrap();
    store.record_native_checkpoint(&reader).unwrap();
    drop(store);
    // Back to 29, with an old repeated-calls dismissal that only 29's table
    // accepted (the kind was removed later).
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(
        "DROP TRIGGER session_warnings_reply_admissions_insert;
         DROP TRIGGER session_warnings_reply_admissions_update;
         DROP TRIGGER session_warnings_reply_admissions_delete;
         DROP TABLE reply_admissions;
         DELETE FROM schema_version WHERE version>=30;
         DELETE FROM migration_fingerprints WHERE version>=30;
         INSERT INTO dismissed_warnings(session_id,warning_id,size,dismissed_at)
         VALUES('s','repeated_calls:r1#0',3,30);",
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        xt_store::LATEST_SCHEMA_VERSION
    );
    let dismissed: Vec<(String, Option<i64>, i64)> = sql
        .prepare("SELECT warning_id,size,dismissed_at FROM dismissed_warnings ORDER BY warning_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        dismissed,
        vec![
            ("correction:p2".into(), None, 20),
            ("repeated_calls:r1#0".into(), Some(3), 30),
            ("repeated_prompt:p1".into(), Some(2), 10),
        ]
    );
    assert!(
        store
            .dismiss_warning("s", "admitted_mistake:a1", None, 40)
            .unwrap()
    );
    assert!(
        store
            .dismiss_warning("s", "admitted_mistake:a2", Some(1), 40)
            .is_err()
    );
    assert_eq!(
        store
            .native_checkpoint(SessionSource::Transcript, &transcript.cursor_key)
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .native_checkpoint(SessionSource::ReadersCli, &reader.cursor_key)
            .unwrap(),
        Some(reader)
    );
}

#[test]
fn a_reply_whose_id_is_too_long_for_the_table_gets_no_row_and_the_write_succeeds() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    let long = "r".repeat(300);
    store
        .upsert_records(
            "s",
            &[
                person("p1", 0, "build it"),
                text(&long, 1, "Sorry, my mistake."),
                text("a2", 2, "Good catch."),
            ],
            false,
        )
        .unwrap();
    // The record itself is kept; only its phrase row is not.
    let kept: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM records WHERE uuid=?1", [&long], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, 1);
    assert_eq!(rows(&path), vec![row("a2", "good_catch")]);
    // A 256-character identifier still fits.
    let longest = "q".repeat(xt_store::reply_admission::MAX_RECORD_ID_CHARS);
    store
        .upsert_records("s", &[text(&longest, 3, "Fair point.")], false)
        .unwrap();
    assert_eq!(rows(&path).len(), 2);
}

#[test]
fn a_whitespace_only_reply_gets_a_row_with_no_phrase() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, false);
    store
        .upsert_records("s", &[text("a1", 1, "\n\n")], false)
        .unwrap();
    assert_eq!(rows(&path), vec![row("a1", "")]);
}
