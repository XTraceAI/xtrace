//! What each tool result said about how its call ended is kept as metadata at
//! write time: which call it answers, its error flag and which fixed host
//! message it starts with, whatever the content retention setting says, and
//! with no result text.
use rusqlite::Connection;
use serde_json::{Value, json};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store, retention::RetentionMode};

fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn calls(uuid: &str, ids: &[&str]) -> CanonicalRecord {
    let content: Vec<Value> = ids
        .iter()
        .map(|id| json!({"type":"tool_use","id":id,"name":"Bash","input":{"command":"pnpm test"}}))
        .collect();
    record(
        json!({"uuid":uuid,"type":"assistant","timestamp":"2026-09-07T12:00:00Z",
        "message":{"role":"assistant","content":content}}),
    )
}
fn result(uuid: &str, call: &str, flag: Option<bool>, text: &str) -> CanonicalRecord {
    let mut block = json!({"type":"tool_result","tool_use_id":call,"content":text});
    if let Some(flag) = flag {
        block["is_error"] = flag.into();
    }
    record(
        json!({"uuid":uuid,"type":"user","timestamp":"2026-09-07T12:00:01Z",
        "message":{"role":"user","content":[block]}}),
    )
}
const DECLINED: &str =
    "The user doesn't want to proceed with this tool use. The tool use was rejected.";

fn store(directory: &tempfile::TempDir, host: &str, keep: bool) -> (Store, std::path::PathBuf) {
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
            &SessionMeta::new("s", host, SessionSource::Transcript),
            false,
        )
        .unwrap();
    (store, path)
}

/// `(result record, block, call id, is_error, wording, conflicted)` for every
/// kept result.
type Row = (String, i64, Option<String>, Option<bool>, String, bool);
fn results(path: &std::path::Path) -> Vec<Row> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT record_uuid,block_index,call_id,is_error,wording,conflicted FROM tool_results
             ORDER BY record_uuid,block_index",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn call_ids(path: &std::path::Path) -> Vec<(String, i64, Option<String>, bool)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT uuid,block_index,call_id,call_id_conflict FROM tool_uses ORDER BY uuid,block_index")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn conversation() -> Vec<CanonicalRecord> {
    vec![
        calls("a", &["t1", "t2", "t3", "t4", "t5", "t6"]),
        result("r1", "t1", Some(true), "1 test failed"),
        result("r2", "t2", Some(true), DECLINED),
        result("r3", "t3", Some(false), "ok"),
        result("r4", "t4", None, "ok"),
        result(
            "r5",
            "t5",
            Some(true),
            "[Request interrupted by user for tool use]",
        ),
        result(
            "r6",
            "t6",
            Some(true),
            "Permission for this action was denied. Reason: outside the project.",
        ),
    ]
}

#[test]
fn the_flag_and_the_wording_kind_are_kept_without_content() {
    for keep in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, path) = store(&directory, "claude", keep);
        store.upsert_records("s", &conversation(), keep).unwrap();
        let t = |id: &str| Some(id.to_owned());
        assert_eq!(
            results(&path),
            vec![
                ("r1".into(), 0, t("t1"), Some(true), "other".into(), false),
                (
                    "r2".into(),
                    0,
                    t("t2"),
                    Some(true),
                    "declined".into(),
                    false
                ),
                ("r3".into(), 0, t("t3"), Some(false), "other".into(), false),
                // Claude omits the flag on a success: kept as absent, and
                // the reader decides what that means.
                ("r4".into(), 0, t("t4"), None, "other".into(), false),
                (
                    "r5".into(),
                    0,
                    t("t5"),
                    Some(true),
                    "interrupted".into(),
                    false
                ),
                (
                    "r6".into(),
                    0,
                    t("t6"),
                    Some(true),
                    "permission_denied".into(),
                    false
                ),
            ],
            "keep content: {keep}"
        );
        assert_eq!(
            call_ids(&path),
            (0..6)
                .map(|i| ("a".to_owned(), i, t(&format!("t{}", i + 1)), false))
                .collect::<Vec<_>>()
        );
        // Metadata only: neither the result's words nor the call's input stayed.
        let connection = Connection::open(&path).unwrap();
        let kept: i64 = connection
            .query_row(
                "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, if keep { 7 } else { 0 });
    }
}

#[test]
fn a_replay_changes_nothing_and_a_same_version_contradiction_is_marked() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "claude", false);
    store.upsert_records("s", &conversation(), false).unwrap();
    let before = results(&path);
    store.upsert_records("s", &conversation(), false).unwrap();
    assert_eq!(results(&path), before);
    // The same result block observed with another outcome, and the same call
    // observed with another identifier: both become unknown to readers.
    store
        .upsert_records(
            "s",
            &[
                result("r3", "t3", Some(true), "ok"),
                calls("a", &["t1", "other", "t3", "t4", "t5", "t6"]),
            ],
            false,
        )
        .unwrap();
    let after = results(&path);
    assert_eq!(after[2].3, Some(false), "the first observation is kept");
    assert!(after[2].5, "and marked contradicted");
    assert!(call_ids(&path)[1].3);
}

#[test]
fn a_record_indexed_before_the_mark_gains_it_when_read_again() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "claude", false);
    store.upsert_records("s", &conversation(), false).unwrap();
    // As an index written before migration 27 holds them.
    Connection::open(&path)
        .unwrap()
        .execute_batch("DELETE FROM tool_results; UPDATE tool_uses SET call_id=NULL;")
        .unwrap();
    store.upsert_records("s", &conversation(), false).unwrap();
    assert_eq!(results(&path).len(), 6);
    assert!(call_ids(&path).iter().all(|row| row.2.is_some()));
}

fn result_observation(kind: &str) -> CanonicalRecord {
    let block = |id| json!({"type":"tool_result","tool_use_id":id,"is_error":true,"content":""});
    let content = match kind {
        "unknown" => Value::Null,
        "empty" => json!([]),
        "one" => json!([block("t1")]),
        "two" => json!([block("t1"), block("t2")]),
        "replaced" => json!([block("t1"), {"type":"text","text":""}]),
        _ => unreachable!(),
    };
    record(
        json!({"uuid":"r","type":"user","timestamp":"2026-09-07T12:00:01Z",
        "message":{"role":"user","content":content}}),
    )
}

#[test]
fn complete_result_position_changes_are_sticky_in_both_orders() {
    for (first, second) in [
        ("two", "one"),
        ("one", "two"),
        ("two", "replaced"),
        ("replaced", "two"),
    ] {
        for same_batch in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut store, path) = store(&directory, "claude", false);
            let observations = [result_observation(first), result_observation(second)];
            if same_batch {
                store.upsert_records("s", &observations, false).unwrap();
            } else {
                for observation in &observations {
                    store
                        .upsert_records("s", std::slice::from_ref(observation), false)
                        .unwrap();
                }
            }
            let expected = vec![
                (
                    "r".into(),
                    0,
                    Some("t1".into()),
                    Some(true),
                    "other".into(),
                    false,
                ),
                (
                    "r".into(),
                    1,
                    Some("t2".into()),
                    Some(true),
                    "other".into(),
                    true,
                ),
            ];
            assert_eq!(
                results(&path),
                expected,
                "{first} -> {second}, same batch: {same_batch}"
            );
            let parent_conflict: bool = Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT has_conflict FROM records WHERE uuid='r'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                !parent_conflict,
                "only the changed result position is contradicted"
            );
            store.upsert_records("s", &observations, false).unwrap();
            assert_eq!(
                results(&path),
                expected,
                "reappearance cannot clear a conflict"
            );
        }
    }
}

#[test]
fn unknown_content_and_unchanged_arrays_do_not_dispute_results() {
    for (first, second) in [("unknown", "two"), ("two", "unknown"), ("two", "two")] {
        for same_batch in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut store, path) = store(&directory, "claude", false);
            let observations = [result_observation(first), result_observation(second)];
            if same_batch {
                store.upsert_records("s", &observations, false).unwrap();
            } else {
                for observation in &observations {
                    store
                        .upsert_records("s", std::slice::from_ref(observation), false)
                        .unwrap();
                }
            }
            let rows = results(&path);
            assert_eq!(rows.len(), 2);
            assert!(
                rows.iter().all(|row| !row.5),
                "{first} -> {second}, same batch: {same_batch}"
            );
        }
    }
}

#[test]
fn an_older_derivation_does_not_change_a_newer_missing_result() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "claude", false);
    store
        .upsert_records("s", &[result_observation("two")], false)
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE tool_results SET fact_version=?1 WHERE block_index=1",
            [xt_store::tool_result::FACT_VERSION + 1],
        )
        .unwrap();
    let before = results(&path);
    store
        .upsert_records("s", &[result_observation("one")], false)
        .unwrap();
    assert_eq!(results(&path), before);
}

#[test]
fn a_foreign_session_cannot_remove_owned_result_positions() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "claude", false);
    store
        .upsert_records("s", &[result_observation("two")], false)
        .unwrap();
    let before = results(&path);
    store
        .upsert_session(
            &SessionMeta::new("other", "claude", SessionSource::Transcript),
            false,
        )
        .unwrap();
    store
        .upsert_records("other", &[result_observation("empty")], false)
        .unwrap();
    assert_eq!(results(&path), before);
}

#[test]
fn result_position_conflicts_roll_back_when_a_later_result_write_fails() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "claude", false);
    store
        .upsert_records("s", &[result_observation("two")], false)
        .unwrap();
    let before = results(&path);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_late_result BEFORE INSERT ON tool_results
         WHEN NEW.record_uuid='late' BEGIN SELECT RAISE(ABORT,'expected rollback'); END;",
        )
        .unwrap();
    assert!(
        store
            .upsert_records(
                "s",
                &[
                    result_observation("one"),
                    result("late", "t1", Some(true), ""),
                ],
                false
            )
            .is_err()
    );
    assert_eq!(results(&path), before);
    let late: i64 = connection
        .query_row(
            "SELECT count(*) FROM records WHERE uuid='late'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(late, 0);
}

#[test]
fn a_missing_flag_is_kept_as_absent_and_another_session_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, path) = store(&directory, "codex", false);
    store
        .upsert_records(
            "s",
            &[
                calls("a", &["c1"]),
                result("r1", "c1", None, "Exit code: 1"),
            ],
            false,
        )
        .unwrap();
    assert_eq!(results(&path)[0].3, None);
    // A record another session owns is not this session's to mark.
    store
        .upsert_session(
            &SessionMeta::new("other", "codex", SessionSource::Transcript),
            false,
        )
        .unwrap();
    store
        .upsert_records("other", &[result("r1", "c1", Some(true), "x")], false)
        .unwrap();
    let rows = results(&path);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].3, rows[0].5), (None, false));
}
