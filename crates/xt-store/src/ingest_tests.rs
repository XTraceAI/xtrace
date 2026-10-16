use super::*;
use crate::{CanonicalRecord, SessionMeta, batch::NativeCheckpoint};
use rusqlite::{
    StatementStatus,
    trace::{TraceEvent, TraceEventCodes},
    types::Value,
};
use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn seed(
    store: &mut Store,
    native: Option<&str>,
    conversation: Option<&str>,
    surface: Option<&str>,
) {
    let mut session = SessionMeta::new("A", "claude", SessionSource::Transcript);
    session.native_session_id = native.map(str::to_owned);
    session.surface = Some("cli".into());
    store.upsert_session(&session, false).unwrap();
    if let Some(conversation) = conversation {
        store
            .observe_discovered_session(&DiscoveredSession {
                host: Host::Claude,
                native_session_id: "native".into(),
                conversation_id: (conversation != "unknown").then(|| conversation.into()),
                surface: surface.map(str::to_owned),
                started_at_ms: None,
                last_observed_at: 11,
                discovery_complete: true,
            })
            .unwrap();
    }
    let record: CanonicalRecord = serde_json::from_value(
        serde_json::json!({"uuid":"record","type":"user","message":{"role":"user","content":[]}}),
    )
    .unwrap();
    store.upsert_records("A", &[record], false).unwrap();
    store
        .insert_capture_receipt(
            &CaptureReceipt {
                receipt_id: "receipt".into(),
                session_id: "A".into(),
                surface: Some("cli".into()),
                received_at: 11,
            },
            &[RecordCoverage {
                record_uuid: "record".into(),
                metric_field_mask: 1,
                measurement_revision: "a".repeat(64),
                digest_schema_version: 1,
            }],
        )
        .unwrap();
    store
        .record_native_checkpoint(&NativeCheckpoint {
            source: SessionSource::Transcript,
            cursor_key: "synthetic".into(),
            generation: "{}".into(),
            position: 123,
            updated_at: 11,
        })
        .unwrap();
}
fn snapshot(store: &Store) -> BTreeMap<String, Vec<Vec<Value>>> {
    let tables = store
        .connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            // Released launch tables use WITHOUT ROWID. Compare every stored
            // column in a stable order rather than relying on a rowid alias.
            let width = store
                .connection
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap()
                .column_count();
            let order = (1..=width)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut stmt = store
                .connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .unwrap();
            let rows = stmt
                .query_map([], |r| (0..width).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}
thread_local! {static DISCOVERY_DML:Cell<usize>=const{Cell::new(0)};static IMMEDIATE:Cell<usize>=const{Cell::new(0)};}
fn trace(store: &mut Store) {
    DISCOVERY_DML.set(0);
    IMMEDIATE.set(0);
    store.connection.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT,
        Some(|event| {
            if let TraceEvent::Stmt(statement, _) = event {
                let sql = statement.sql();
                if sql.starts_with("INSERT INTO discovered_sessions")
                    || sql.starts_with("UPDATE discovered_sessions")
                {
                    DISCOVERY_DML.set(DISCOVERY_DML.get() + 1);
                }
                if sql == "BEGIN IMMEDIATE" {
                    IMMEDIATE.set(IMMEDIATE.get() + 1);
                }
            }
        }),
    );
}

#[test]
fn discovery_surface_recovery_changes_only_missing_surface_and_healthy_path_does_no_dml() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, Some("native"), Some("A"), None);
    let mut discovery = store
        .discovered_session(Host::Claude, "native")
        .unwrap()
        .unwrap();
    discovery.started_at_ms = Some(7);
    discovery.discovery_complete = false;
    store.observe_discovered_session(&discovery).unwrap();
    let mut before = snapshot(&store);
    trace(&mut store);
    assert_eq!(
        store
            .repair_claude_discovery_surface("native", "A")
            .unwrap(),
        Some("cli".into())
    );
    discovery.surface = Some("cli".into());
    assert_eq!(
        store.discovered_session(Host::Claude, "native").unwrap(),
        Some(discovery)
    );
    let mut after = snapshot(&store);
    before.remove("discovered_sessions");
    after.remove("discovered_sessions");
    assert_eq!(before, after);
    assert_eq!(DISCOVERY_DML.get(), 1);
    assert_eq!(IMMEDIATE.get(), 1);
    assert_eq!(
        store
            .repair_claude_discovery_surface("native", "A")
            .unwrap(),
        Some("cli".into())
    );
    assert_eq!(
        (DISCOVERY_DML.get(), IMMEDIATE.get()),
        (1, 1),
        "healthy path performs exact reads only"
    );
}

#[test]
fn discovery_recovery_does_not_synthesize_identity_or_overwrite_known_conflicts() {
    for (native, conversation, surface, error) in [
        (None, Some("A"), None, false),
        (Some("native"), None, None, false),
        (Some("native"), Some("unknown"), None, false),
        (Some("other"), Some("A"), None, true),
        (Some("native"), Some("B"), None, true),
        (Some("native"), Some("A"), Some("desktop"), true),
    ] {
        let mut store = Store::open_in_memory().unwrap();
        seed(&mut store, native, conversation, surface);
        let before = snapshot(&store);
        trace(&mut store);
        assert_eq!(
            store
                .repair_claude_discovery_surface("native", "A")
                .is_err(),
            error
        );
        assert_eq!(snapshot(&store), before);
        assert_eq!((DISCOVERY_DML.get(), IMMEDIATE.get()), (0, 0));
    }
}

#[test]
fn discovery_recovery_rechecks_current_fields_after_waiting_for_writer_lock() {
    static WAITING: AtomicBool = AtomicBool::new(false);
    for filled in [None, Some("cli"), Some("desktop")] {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("race.sqlite");
        let mut store = Store::open(&path).unwrap();
        seed(&mut store, Some("native"), Some("A"), None);
        let mut concurrent = Connection::open(&path).unwrap();
        let transaction = concurrent
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        WAITING.store(false, Ordering::SeqCst);
        store
            .connection
            .busy_handler(Some(|attempt| {
                WAITING.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(1));
                attempt < 2000
            }))
            .unwrap();
        let worker = std::thread::spawn(move || {
            trace(&mut store);
            let result = store.repair_claude_discovery_surface("native", "A");
            (
                result,
                store.discovered_session(Host::Claude, "native").unwrap(),
                DISCOVERY_DML.get(),
            )
        });
        let started = Instant::now();
        while !WAITING.load(Ordering::SeqCst) {
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
        // Same observation clock, newer completeness and newly known start:
        // replaying the pre-lock row would incorrectly restore complete=true.
        let current = DiscoveredSession {
            host: Host::Claude,
            native_session_id: "native".into(),
            conversation_id: Some("A".into()),
            surface: filled.map(str::to_owned),
            started_at_ms: Some(88),
            last_observed_at: 11,
            discovery_complete: false,
        };
        observe_discovered_session(&transaction, &current).unwrap();
        transaction.commit().unwrap();
        let (result, saved, dml) = worker.join().unwrap();
        assert_eq!(result.is_err(), filled == Some("desktop"));
        let mut expected = current;
        if filled.is_none() {
            expected.surface = Some("cli".into());
        }
        assert_eq!(saved, Some(expected));
        assert_eq!(dml, usize::from(filled.is_none()));
    }
}

#[test]
fn lookup_index_is_narrow_idempotent_and_restored_without_data_history_changes() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, Some("native"), Some("A"), None);
    let before = snapshot(&store);
    let version: i64 = store
        .connection
        .query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap();
    store.migrate().unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        version
    );
    for sql in [
        "DROP INDEX tool_uses_structural_event_id",
        "DROP INDEX tool_uses_structural_event_id; CREATE UNIQUE INDEX tool_uses_structural_event_id ON tool_uses(name)",
    ] {
        store.connection.execute_batch(sql).unwrap();
        store.migrate().unwrap();
        let definition: String = store
            .connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='tool_uses_structural_event_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            definition,
            "CREATE INDEX tool_uses_structural_event_id ON tool_uses(source_event_id) WHERE uuid IS NULL"
        );
        assert_eq!(snapshot(&store), before);
    }
    store.connection.execute_batch("DROP INDEX tool_uses_structural_event_id; DROP VIEW v_records; CREATE VIEW v_records AS SELECT 1 AS marker; CREATE TABLE tool_uses_structural_event_id(marker)").unwrap();
    let view: String = store
        .connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='v_records'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before = snapshot(&store);
    assert!(store.migrate().is_err());
    assert_eq!(snapshot(&store), before);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='v_records'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        view,
        "failed index maintenance rolls back changed views in the same transaction"
    );
}

#[test]
fn lookup_index_concurrent_open_installs_once_and_read_only_resolution_does_not_repair() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    seed(&mut store, Some("native"), Some("A"), None);
    let before = snapshot(&store);
    store
        .connection
        .execute_batch("DROP INDEX tool_uses_structural_event_id")
        .unwrap();
    let version: i64 = store
        .connection
        .query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap();
    let reader =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    resolved_tool_events(&reader).unwrap();
    assert_eq!(
        reader
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='tool_uses_structural_event_id'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(reader);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(path).unwrap();
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        version + 1
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn exact_lookup_seeks_the_managed_index_and_reads_all_matching_observations() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, Some("native"), Some("A"), None);
    for n in 0..1000 {
        store.connection.execute("INSERT INTO tool_uses(session_id,source,source_event_id,name,kind) VALUES('A','transcript',?1,'hook','hook')",[format!("unrelated-{n}")]).unwrap();
    }
    for source in [
        SessionSource::Transcript,
        SessionSource::Plugin,
        SessionSource::ReadersCli,
    ] {
        store.connection.execute("INSERT INTO tool_uses(session_id,source,source_event_id,name,kind) VALUES('A',?1,'shared','hook','hook')",[source]).unwrap();
    }
    let query = tool_event_query(true);
    let plan = store
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .unwrap()
        .query_map(["shared"], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter().any(|p| p
            .contains("SEARCH t USING INDEX tool_uses_structural_event_id (source_event_id=?)")),
        "{plan:?}"
    );
    for (id, expected) in [("shared", 3), ("absent", 0)] {
        let mut statement = store.connection.prepare(&query).unwrap();
        let rows = statement
            .query_map([id], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), expected);
        assert!(
            statement.get_status(StatementStatus::VmStep) < 200,
            "fixed K=3/0 must not scan 1000 unrelated events"
        );
        assert_eq!(
            tool_event_observations(&store.connection, Some(id))
                .unwrap()
                .len(),
            expected
        );
    }
    let before = snapshot(&store);
    store.migrate().unwrap();
    assert_eq!(
        snapshot(&store),
        before,
        "nonunique access path preserves every historical observation"
    );
}
