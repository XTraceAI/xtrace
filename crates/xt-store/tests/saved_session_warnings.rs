use rusqlite::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    saved_session_warnings::{SavedSessionWarning, WarningPublication},
};

fn revision(sql: &Connection, id: &str) -> i64 {
    sql.query_row(
        "SELECT revision FROM session_warning_inputs WHERE session_id=?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn snapshot(sql: &Connection) -> std::collections::BTreeMap<String, i64> {
    sql.prepare("SELECT session_id,revision FROM session_warning_inputs")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn answer(sql: &Connection, id: &str) -> SavedSessionWarning {
    SavedSessionWarning {
        session_id: id.into(),
        revision: revision(sql, id),
        rule_version: 1,
        payload_json: "{}".into(),
    }
}
fn seed(store: &mut Store, id: &str, host: &str, surface: &str, time: Option<&str>) {
    let mut meta = SessionMeta::new(id, host, SessionSource::Transcript);
    meta.surface = Some(surface.into());
    store.upsert_session(&meta, false).unwrap();
    if let Some(time) = time {
        store
            .upsert_records(id, &[record(&format!("{id}-r"), Some(time))], false)
            .unwrap();
    }
}
fn record(id: &str, time: Option<&str>) -> CanonicalRecord {
    serde_json::from_value(serde_json::json!({"uuid":id,"type":"user","timestamp":time,"message":{"role":"user","content":[{"type":"text","text":"a synthetic request"}]}})).unwrap()
}

#[test]
fn every_tracked_table_changes_only_for_semantic_writes_and_rolls_back() {
    let tracked = [
        ("sessions", "session_id host surface native_session_id kind"),
        (
            "records",
            "uuid session_id type ts ts_ms is_meta is_sidechain role model is_tool_result_carrier text_len tool_use_count content_json has_conflict parent_uuid agent_id subtype is_human is_command is_interrupted is_system_reminder",
        ),
        (
            "tool_uses",
            "uuid session_id block_index name group_version group_key group_conflict call_id call_id_conflict",
        ),
        (
            "tool_results",
            "record_uuid session_id block_index call_id is_error flag_stated wording fact_version conflicted",
        ),
        (
            "record_previews",
            "record_uuid session_id kind text truncated rule_version",
        ),
        ("confirmed_automated_inputs", "record_uuid session_id"),
        ("guardian_turn_inputs", "record_uuid session_id"),
        ("injected_context_inputs", "record_uuid session_id"),
        ("task_notification_inputs", "record_uuid session_id"),
        ("tool_sent_inputs", "record_uuid session_id evidence_kind"),
        (
            "session_creation_relations",
            "child_session_id child_host child_native_session_id state",
        ),
        (
            "human_session_origins",
            "session_id host native_session_id conflicted",
        ),
        (
            "human_input_adjustments",
            "record_uuid session_id original_ts original_length retained_length conflicted",
        ),
    ];
    for (table, relevant) in tracked {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        drop(Store::open(&path).unwrap());
        let sql = Connection::open(path).unwrap();
        sql.pragma_update(None, "foreign_keys", false).unwrap();
        sql.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        // Test the real cache triggers independently of proof-owner immutability.
        let others=sql.prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND name NOT LIKE 'session_warnings_%'").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        for name in others {
            sql.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
        }
        if table != "sessions" {
            sql.execute_batch("INSERT INTO sessions(session_id,host,source,surface) VALUES('fact','fact','fact','fact')").unwrap();
        }
        let columns = sql
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let names = columns
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let values = columns
            .iter()
            .map(|(_, k)| {
                if k.to_ascii_uppercase().contains("INT") {
                    "1"
                } else {
                    "'fact'"
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let before = if table == "sessions" {
            0
        } else {
            revision(&sql, "fact")
        };
        sql.execute_batch(&format!("INSERT INTO {table}({names}) VALUES({values})"))
            .unwrap();
        assert!(revision(&sql, "fact") > before, "{table} insert");
        let before = revision(&sql, "fact");
        let noop = columns
            .iter()
            .map(|(n, _)| format!("{n}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        sql.execute_batch(&format!("UPDATE {table} SET {noop}"))
            .unwrap();
        assert_eq!(revision(&sql, "fact"), before, "{table} replay");
        for (name, kind) in &columns {
            let before = snapshot(&sql);
            let value = if kind.to_ascii_uppercase().contains("INT") {
                format!("{name}+1")
            } else {
                format!("{name}||'x'")
            };
            sql.execute_batch("BEGIN").unwrap();
            sql.execute_batch(&format!("UPDATE {table} SET {name}={value}"))
                .unwrap();
            if relevant.split_whitespace().any(|f| f == name) {
                assert!(revision(&sql, "fact") > before["fact"], "{table}.{name}");
            } else {
                assert_eq!(snapshot(&sql), before, "{table}.{name}");
            }
            sql.execute_batch("ROLLBACK").unwrap();
            assert_eq!(snapshot(&sql), before, "{table} rollback");
        }
        let before = revision(&sql, "fact");
        sql.execute_batch(&format!("DELETE FROM {table}")).unwrap();
        assert!(revision(&sql, "fact") > before, "{table} delete");
    }
}

#[test]
fn bounds_are_eager_and_health_changes_dirty_only_overlapping_matching_surfaces() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(
        &mut store,
        "a",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    seed(
        &mut store,
        "b",
        "claude",
        "cli",
        Some("2026-09-01T12:00:01Z"),
    );
    seed(
        &mut store,
        "c",
        "claude",
        "cli",
        Some("2026-09-10T12:00:00Z"),
    );
    seed(
        &mut store,
        "d",
        "claude",
        "desktop",
        Some("2026-09-01T12:00:00Z"),
    );
    seed(&mut store, "unknown", "claude", "cli", None);
    let before = snapshot(&sql);
    store
        .upsert_records("a", &[record("a-new", Some("2026-09-01T12:00:02Z"))], false)
        .unwrap();
    let after = snapshot(&sql);
    for id in ["a", "b", "unknown"] {
        assert!(after[id] > before[id], "{id}");
    }
    for id in ["c", "d"] {
        assert_eq!(after[id], before[id], "{id}");
    }
    let bounds: (i64, i64) = sql
        .query_row(
            "SELECT first_ms,last_ms FROM session_warning_inputs WHERE session_id='a'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(bounds.1 - bounds.0, 2000);
    let before = snapshot(&sql);
    sql.execute("UPDATE records SET is_human=0 WHERE uuid='a-r'", [])
        .unwrap();
    let after = snapshot(&sql);
    assert!(after["a"] > before["a"]);
    for id in ["b", "c", "d", "unknown"] {
        assert_eq!(after[id], before[id], "classification {id}");
    }
    let before = snapshot(&sql);
    store
        .upsert_records("a", &[record("untimed", None)], false)
        .unwrap();
    let after = snapshot(&sql);
    for id in ["a", "b", "c", "unknown"] {
        assert!(after[id] > before[id], "unknown timing {id}");
    }
    assert_eq!(after["d"], before["d"]);
}

#[test]
fn timestamp_and_session_moves_cover_old_and_new_windows_and_bindings() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    for (id, host, surface, time) in [
        ("a", "claude", "cli", "2026-09-01T12:00:00Z"),
        ("old", "claude", "cli", "2026-09-01T12:00:01Z"),
        ("new", "claude", "cli", "2026-09-10T12:00:00Z"),
        ("other", "codex", "cli", "2026-09-10T12:00:00Z"),
        ("desktop", "codex", "desktop", "2026-09-10T12:00:00Z"),
    ] {
        seed(&mut store, id, host, surface, Some(time));
    }
    let before = snapshot(&sql);
    let at = xt_store::timestamp::parse("2026-09-10T12:00:00Z")
        .unwrap()
        .1;
    sql.execute(
        "UPDATE records SET ts='2026-09-10T12:00:00Z',ts_ms=?1 WHERE uuid='a-r'",
        [at],
    )
    .unwrap();
    let after = snapshot(&sql);
    assert!(after["old"] > before["old"]);
    assert!(after["new"] > before["new"]);
    assert_eq!(after["other"], before["other"]);
    let before = snapshot(&sql);
    sql.execute("UPDATE sessions SET host='codex' WHERE session_id='a'", [])
        .unwrap();
    let after = snapshot(&sql);
    assert!(after["new"] > before["new"]);
    assert!(after["other"] > before["other"]);
    assert_eq!(after["desktop"], before["desktop"]);
    let before = snapshot(&sql);
    sql.execute(
        "UPDATE sessions SET surface='desktop' WHERE session_id='a'",
        [],
    )
    .unwrap();
    let after = snapshot(&sql);
    assert!(after["other"] > before["other"]);
    assert!(after["desktop"] > before["desktop"]);
    let before = snapshot(&sql);
    sql.execute("UPDATE sessions SET kind='judge' WHERE session_id='a'", [])
        .unwrap();
    assert!(revision(&sql, "desktop") > before["desktop"]);
}

#[test]
fn partial_publication_keeps_stable_answers_and_failure_or_rollback_keeps_previous_rows() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(
        &mut store,
        "a",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    seed(
        &mut store,
        "b",
        "claude",
        "cli",
        Some("2026-09-10T12:00:00Z"),
    );
    let rows = [answer(&sql, "a"), answer(&sql, "b")];
    sql.execute("UPDATE records SET is_human=0 WHERE uuid='a-r'", [])
        .unwrap();
    assert_eq!(
        store.publish_session_warnings(&rows).unwrap(),
        WarningPublication {
            published: 1,
            rejected: 1
        }
    );
    assert_eq!(
        sql.query_row("SELECT session_id FROM saved_session_warnings", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "b"
    );
    let mut invalid = answer(&sql, "b");
    invalid.payload_json = "not json".into();
    assert!(
        store
            .publish_session_warnings(&[answer(&sql, "a"), invalid])
            .is_err()
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    store
        .dismiss_warning("b", "correction:r", None, 10)
        .unwrap();
    let before = revision(&sql, "b");
    assert_eq!(before, rows[1].revision);
    store
        .publish_session_warnings(&[answer(&sql, "b")])
        .unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM dismissed_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn cancellation_while_waiting_for_writer_lock_prevents_late_publication() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(&mut store, "a", "claude", "cli", None);
    let row = answer(&sql, "a");
    store
        .publish_session_warnings(std::slice::from_ref(&row))
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&cancel);
    sql.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (started, ready) = mpsc::channel();
    let writing = std::thread::spawn(move || {
        started.send(()).unwrap();
        store
            .publish_session_warnings_cancellable(&[row], &|| cancelled.load(Ordering::SeqCst))
            .unwrap()
    });
    ready.recv().unwrap();
    cancel.store(true, Ordering::SeqCst);
    sql.execute_batch("COMMIT").unwrap();
    assert_eq!(writing.join().unwrap(), WarningPublication::default());
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn deleting_a_session_dirties_peers_before_cascade_removes_its_bounds() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    sql.pragma_update(None, "foreign_keys", true).unwrap();
    seed(&mut store, "a", "claude", "cli", None);
    seed(
        &mut store,
        "b",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    store
        .publish_session_warnings(&[answer(&sql, "a")])
        .unwrap();
    let before = revision(&sql, "b");
    sql.execute("DELETE FROM sessions WHERE session_id='a'", [])
        .unwrap();
    assert!(revision(&sql, "b") > before);
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM session_warning_inputs WHERE session_id='a'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn canonical_ingest_replay_direct_writes_and_content_purge_invalidate_correctly() {
    use xt_store::{
        batch::IngestBatch,
        retention::{ContentRegistry, RetentionMode},
    };
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    store
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let meta = SessionMeta::new("s", "claude", SessionSource::Transcript);
    let rows = [record("input", Some("2026-09-01T00:00:00Z"))];
    store
        .apply_ingest_batch(&IngestBatch::new(&meta, &rows, true))
        .unwrap();
    let first = revision(&sql, "s");
    store
        .apply_ingest_batch(&IngestBatch::new(&meta, &rows, true))
        .unwrap();
    assert_eq!(revision(&sql, "s"), first);
    store.upsert_session(&meta, false).unwrap();
    store.upsert_records("s", &rows, true).unwrap();
    assert_eq!(revision(&sql, "s"), first);
    let mut stale = answer(&sql, "s");
    stale.payload_json = serde_json::json!({"matched_span": "still not working"}).to_string();
    store
        .publish_session_warnings(std::slice::from_ref(&stale))
        .unwrap();
    store.purge_content(&ContentRegistry::default()).unwrap();
    assert_eq!(
        store.publish_session_warnings(&[stale]).unwrap(),
        WarningPublication {
            published: 0,
            rejected: 1
        }
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let purged = revision(&sql, "s");
    assert!(purged > first);
    store.purge_content(&ContentRegistry::default()).unwrap();
    assert_eq!(revision(&sql, "s"), purged);
}

#[test]
fn deleted_and_recreated_identifiers_never_accept_an_old_candidate() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(&path).unwrap();
    sql.pragma_update(None, "foreign_keys", true).unwrap();
    seed(
        &mut store,
        "a",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    store
        .upsert_records(
            "a",
            &[
                record("a-second", Some("2026-09-01T12:01:00Z")),
                record("a-third", Some("2026-09-01T12:02:00Z")),
            ],
            false,
        )
        .unwrap();
    let old = answer(&sql, "a");
    sql.execute_batch(
        "DELETE FROM record_sources WHERE uuid IN (SELECT uuid FROM records WHERE session_id='a');
         DELETE FROM record_previews WHERE session_id='a';
         DELETE FROM usage WHERE uuid IN (SELECT uuid FROM records WHERE session_id='a');
         DELETE FROM records WHERE session_id='a'; DELETE FROM session_sources WHERE session_id='a'; DELETE FROM session_child_checks WHERE session_id='a'; DELETE FROM sessions WHERE session_id='a';",
    )
    .unwrap();
    seed(
        &mut store,
        "a",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    store
        .upsert_records(
            "a",
            &[
                record("replacement-second", Some("2026-09-01T12:01:00Z")),
                record("replacement-third", Some("2026-09-01T12:02:00Z")),
            ],
            false,
        )
        .unwrap();
    assert!(revision(&sql, "a") > old.revision);
    assert_eq!(
        store.publish_session_warnings(&[old]).unwrap(),
        WarningPublication {
            published: 0,
            rejected: 1
        }
    );
}

#[test]
fn allocator_advances_without_invalidating_unrelated_answers_and_rolls_back_with_inputs() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(
        &mut store,
        "a",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    seed(
        &mut store,
        "b",
        "codex",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    let b = answer(&sql, "b");
    store
        .publish_session_warnings(std::slice::from_ref(&b))
        .unwrap();
    let counter = || {
        sql.query_row(
            "SELECT value FROM meta WHERE key='session_warnings_revision_allocator'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    };
    let before = counter();
    sql.execute("UPDATE records SET is_human=0 WHERE uuid='a-r'", [])
        .unwrap();
    assert_ne!(counter(), before);
    assert_eq!(revision(&sql, "b"), b.revision);
    assert_eq!(store.publish_session_warnings(&[b]).unwrap().published, 1);
    let before = counter();
    let rows = snapshot(&sql);
    sql.execute_batch("BEGIN; UPDATE records SET is_human=1 WHERE uuid='a-r'; ROLLBACK;")
        .unwrap();
    assert_eq!(counter(), before);
    assert_eq!(snapshot(&sql), rows);
    for bad in ["", "x", "1oops", "-1", "9223372036854775807"] {
        sql.execute(
            "UPDATE meta SET value=?1 WHERE key='session_warnings_revision_allocator'",
            [bad],
        )
        .unwrap();
        assert!(
            sql.execute("UPDATE records SET is_human=1 WHERE uuid='a-r'", [])
                .is_err(),
            "{bad}"
        );
        assert_eq!(snapshot(&sql), rows);
    }
}

#[test]
fn moving_a_record_invalidates_both_owners_and_both_surface_windows() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    for (id, host, time) in [
        ("a", "claude", "2026-09-01T12:00:00Z"),
        ("old-peer", "claude", "2026-09-01T12:00:01Z"),
        ("b", "codex", "2026-09-10T12:00:00Z"),
        ("new-peer", "codex", "2026-09-10T12:00:01Z"),
        ("unrelated", "claude", "2026-09-20T12:00:00Z"),
    ] {
        seed(&mut store, id, host, "cli", Some(time));
    }
    let before = snapshot(&sql);
    let at = xt_store::timestamp::parse("2026-09-10T12:00:00Z")
        .unwrap()
        .1;
    sql.execute(
        "UPDATE records SET session_id='b',ts='2026-09-10T12:00:00Z',ts_ms=?1 WHERE uuid='a-r'",
        [at],
    )
    .unwrap();
    let after = snapshot(&sql);
    for id in ["a", "b", "old-peer", "new-peer"] {
        assert!(after[id] > before[id], "{id}");
    }
    assert_eq!(after["unrelated"], before["unrelated"]);
}

#[test]
fn migration_backfills_old_bounds_and_growth_widens_them_before_any_saved_answer() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    seed(
        &mut store,
        "old",
        "claude",
        "cli",
        Some("2026-09-01T12:00:00Z"),
    );
    store
        .upsert_records(
            "old",
            &[record("old-last", Some("2026-09-01T12:10:00Z"))],
            false,
        )
        .unwrap();
    drop(store);
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(include_str!("fixtures/undo_saved_session_warnings.sql"))
        .unwrap();
    sql.execute_batch("DELETE FROM schema_version WHERE version=29; DELETE FROM migration_fingerprints WHERE version=29;").unwrap();
    let mut store = Store::open(path).unwrap();
    let bounds = || {
        sql.query_row(
            "SELECT first_ms,last_ms FROM session_warning_inputs WHERE session_id='old'",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap()
    };
    let before = bounds();
    assert_eq!(before.1 - before.0, 600_000);
    assert_eq!(revision(&sql, "old"), 1);
    store
        .upsert_records(
            "old",
            &[record("new-last", Some("2026-09-01T12:20:00Z"))],
            false,
        )
        .unwrap();
    assert_eq!(bounds(), (before.0, before.1 + 600_000));
    assert!(revision(&sql, "old") > 1);
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cache_only_purge_invalidates_and_a_second_purge_is_a_noop() {
    use xt_store::retention::ContentRegistry;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(&mut store, "empty-source", "claude", "cli", None);
    let mut saved = answer(&sql, "empty-source");
    saved.payload_json = serde_json::json!({"matched_span": "still not working"}).to_string();
    store
        .publish_session_warnings(std::slice::from_ref(&saved))
        .unwrap();
    store
        .dismiss_warning("empty-source", "correction:prior", None, 1)
        .unwrap();
    let before = snapshot(&sql);
    let allocator: String = sql
        .query_row(
            "SELECT value FROM meta WHERE key='session_warnings_revision_allocator'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let output = store.purge_content(&ContentRegistry::default()).unwrap();
    assert!(output.invalidate_content);
    assert_eq!(
        output
            .tables
            .iter()
            .find(|table| table.table == "saved_session_warnings")
            .unwrap()
            .rows,
        1
    );
    assert!(
        output
            .tables
            .iter()
            .filter(|table| table.table != "saved_session_warnings")
            .all(|table| table.rows == 0)
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(snapshot(&sql), before);
    assert_eq!(
        sql.query_row(
            "SELECT value FROM meta WHERE key='session_warnings_revision_allocator'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        allocator
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM dismissed_warnings", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let again = store.purge_content(&ContentRegistry::default()).unwrap();
    assert!(!again.invalidate_content);
    assert!(again.tables.iter().all(|table| table.rows == 0));
}

#[test]
fn cache_delete_failure_rolls_back_source_cache_and_revisions() {
    use xt_store::retention::{ContentRegistry, RetentionMode};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    store
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let mut meta = SessionMeta::new("retained", "claude", SessionSource::Transcript);
    meta.title = Some("Retained title".into());
    store.upsert_session(&meta, true).unwrap();
    let retained: CanonicalRecord = serde_json::from_value(serde_json::json!({
        "uuid":"retained-record", "type":"assistant", "timestamp":"2026-09-01T00:00:00Z",
        "message":{"role":"assistant","content":[
            {"type":"text","text":"Retained response"},
            {"type":"tool_use","id":"retained-call","name":"Read","input":{"path":"retained.txt"}}
        ]}
    }))
    .unwrap();
    store.upsert_records("retained", &[retained], true).unwrap();
    let cached = answer(&sql, "retained");
    store
        .publish_session_warnings(std::slice::from_ref(&cached))
        .unwrap();
    let records = store.records("retained").unwrap();
    assert!(records[0].content_json.is_some());
    assert!(records[0].tool_uses[0].input_json.is_some());
    let revisions = snapshot(&sql);
    let allocator: String = sql
        .query_row(
            "SELECT value FROM meta WHERE key='session_warnings_revision_allocator'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    sql.execute_batch(
        "CREATE TRIGGER fail_warning_delete BEFORE DELETE ON saved_session_warnings BEGIN
         SELECT CASE WHEN EXISTS(SELECT 1 FROM records WHERE content_json IS NOT NULL)
           OR EXISTS(SELECT 1 FROM sessions WHERE title IS NOT NULL)
           OR EXISTS(SELECT 1 FROM tool_uses WHERE input_json IS NOT NULL)
           THEN RAISE(ABORT,'cache deletion ran before source clearing') END;
         SELECT RAISE(ABORT,'cache deletion failed after source clearing'); END;",
    )
    .unwrap();
    let failure = store
        .purge_content(&ContentRegistry::default())
        .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("cache deletion failed after source clearing")
    );
    assert_eq!(store.records("retained").unwrap(), records);
    assert_eq!(
        store.session("retained").unwrap().unwrap().meta.title,
        meta.title
    );
    let stored: (i64, i64, String) = sql.query_row("SELECT revision,rule_version,payload_json FROM saved_session_warnings WHERE session_id='retained'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(
        stored,
        (cached.revision, cached.rule_version, cached.payload_json)
    );
    assert_eq!(snapshot(&sql), revisions);
    assert_eq!(
        sql.query_row(
            "SELECT value FROM meta WHERE key='session_warnings_revision_allocator'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        allocator
    );
}

#[test]
fn cancellation_after_one_write_restores_the_previous_saved_answers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel.db");
    let mut store = Store::open(&path).unwrap();
    let sql = Connection::open(path).unwrap();
    seed(&mut store, "a", "claude", "cli", None);
    seed(&mut store, "b", "claude", "cli", None);
    let previous = answer(&sql, "a");
    store
        .publish_session_warnings(std::slice::from_ref(&previous))
        .unwrap();
    let mut replacement = previous.clone();
    replacement.payload_json = "{\"replacement\":true}".into();
    let checks = std::cell::Cell::new(0);
    let output = store
        .publish_session_warnings_cancellable(&[replacement, answer(&sql, "b")], &|| {
            checks.set(checks.get() + 1);
            checks.get() == 3
        })
        .unwrap();
    assert_eq!(output, WarningPublication::default());
    let rows: Vec<(String, String)> = sql
        .prepare("SELECT session_id,payload_json FROM saved_session_warnings ORDER BY session_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, vec![(previous.session_id, previous.payload_json)]);
}
