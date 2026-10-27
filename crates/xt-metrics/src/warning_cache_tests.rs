use super::*;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};
use xt_fixtures::TempDb;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

fn seeded() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .upsert_session(
            &SessionMeta::new("s", "claude", SessionSource::Transcript),
            false,
        )
        .unwrap();
    let rows:Vec<CanonicalRecord>=serde_json::from_value(serde_json::json!([
        {"uuid":"h1","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"please fix the synthetic tests"}]}},
        {"uuid":"a1","type":"assistant","timestamp":"2026-09-01T00:01:00Z","message":{"role":"assistant","content":[]}},
        {"uuid":"h2","type":"user","timestamp":"2026-09-01T00:02:00Z","message":{"role":"user","content":[{"type":"text","text":"please fix the synthetic tests"}]}}
    ])).unwrap();
    db.store_mut().upsert_records("s", &rows, false).unwrap();
    db
}
fn warm(db: &mut TempDb) {
    let candidate = MetricsDb::open(db.path())
        .unwrap()
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap();
    assert_eq!(
        db.store_mut()
            .publish_session_warnings(&candidate.rows)
            .unwrap()
            .rejected,
        0
    );
}

#[test]
fn current_saved_reads_and_unchanged_refresh_never_load_history() {
    let mut db = seeded();
    warm(&mut db);
    let metrics = MetricsDb::open(db.path()).unwrap();
    metrics
        .connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "records" | "record_previews" | "tool_uses" | "tool_results",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    assert!(
        metrics
            .prepare_session_warning_refresh(&[])
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        metrics
            .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
            .unwrap()["s"],
        WholeSessionWarnings::Read { .. }
    ));
    assert!(!metrics.session_warning_refresh_needed());
    assert!(
        metrics
            .whole_session_warnings(&["s"], RepeatThresholds::new(1, 1).unwrap(), &TimeZone::UTC)
            .is_err()
    );
    // A window still reads lightweight timed membership. It must not enter
    // the warning loader, whose result/call/preview reads this guard refuses.
    let window_reader = MetricsDb::open(db.path()).unwrap();
    window_reader
        .connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "record_previews" | "tool_uses" | "tool_results",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    let start = "2026-09-01T00:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond();
    window_reader
        .saved_warnings_in_window(Window::new(start, start + 180_000).unwrap(), &TimeZone::UTC)
        .unwrap();
    assert!(!window_reader.session_warning_refresh_needed());
}

#[test]
fn partial_publication_refreshes_only_rejected_sessions_next() {
    let mut db = seeded();
    db.store_mut()
        .upsert_session(
            &SessionMeta::new("other", "codex", SessionSource::Transcript),
            false,
        )
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let candidate = metrics
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap();
    assert_eq!(candidate.rows.len(), 2);
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=0 WHERE uuid='h2'", [])
        .unwrap();
    let result = db
        .store_mut()
        .publish_session_warnings(&candidate.rows)
        .unwrap();
    assert_eq!((result.published, result.rejected), (1, 1));
    let next = metrics
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap();
    assert_eq!(next.rows.len(), 1);
    assert_eq!(next.rows[0].session_id, "s");
    db.store_mut().publish_session_warnings(&next.rows).unwrap();
    assert!(metrics.session_warning_cache_is_current().unwrap());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&["s", "other"], &TimeZone::UTC)
            .unwrap(),
        metrics
            .whole_session_warnings(&["s", "other"], RepeatThresholds::default(), &TimeZone::UTC)
            .unwrap()
    );
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE saved_session_warnings SET rule_version=999", [])
        .unwrap();
    assert_eq!(
        metrics
            .prepare_session_warning_refresh(&[])
            .unwrap()
            .unwrap()
            .rows
            .len(),
        2
    );
}

#[test]
fn reads_and_dismissals_share_snapshot_across_replacement() {
    let mut db = seeded();
    warm(&mut db);
    let metrics = MetricsDb::open(db.path()).unwrap();
    metrics
        .read_snapshot(|snapshot| {
            let before = snapshot.saved_whole_session_warnings(&["s"], &TimeZone::UTC)?;
            Connection::open(db.path())
                .unwrap()
                .execute("UPDATE records SET is_human=0 WHERE uuid='h2'", [])
                .unwrap();
            let fresh = MetricsDb::open(db.path())
                .unwrap()
                .prepare_session_warning_refresh(&[])
                .unwrap()
                .unwrap();
            db.store_mut()
                .dismiss_warning("s", "repeated_prompt:h1", Some(2), 10)
                .unwrap();
            db.store_mut()
                .publish_session_warnings(&fresh.rows)
                .unwrap();
            assert_eq!(
                snapshot.saved_whole_session_warnings(&["s"], &TimeZone::UTC)?,
                before
            );
            assert!(snapshot.dismissed_warnings(&["s"])?.is_empty());
            Ok(())
        })
        .unwrap();
    assert!(!metrics.dismissed_warnings(&["s"]).unwrap().is_empty());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
            .unwrap(),
        metrics
            .whole_session_warnings(&["s"], RepeatThresholds::default(), &TimeZone::UTC)
            .unwrap()
    );
}

#[test]
fn already_cancelled_refresh_leaves_prior_answer_untouched() {
    let mut db = seeded();
    warm(&mut db);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let before = metrics
        .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
        .unwrap();
    assert!(
        metrics
            .prepare_session_warning_refresh_cancellable(
                &["s".into()],
                Arc::new(AtomicBool::new(true))
            )
            .is_err()
    );
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
            .unwrap(),
        before
    );
}

#[test]
fn saved_tables_and_columns_are_part_of_open_contract() {
    for damage in [
        "DROP TABLE saved_session_warnings",
        "ALTER TABLE saved_session_warnings RENAME COLUMN payload_json TO obsolete",
        "ALTER TABLE saved_session_warnings RENAME COLUMN revision TO obsolete",
        "DROP TABLE session_warning_inputs",
        "ALTER TABLE session_warning_inputs RENAME COLUMN revision TO obsolete",
    ] {
        let db = seeded();
        Connection::open(db.path())
            .unwrap()
            .execute_batch(damage)
            .unwrap();
        assert!(MetricsDb::open(db.path()).is_err(), "{damage}");
    }
}
