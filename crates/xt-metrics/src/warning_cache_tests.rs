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
fn old_v2_answer_at_current_revision_cannot_hide_parent_conflict() {
    use crate::session_warnings::{CheckState, Missing};

    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .upsert_session(
            &SessionMeta::new("s", "claude", SessionSource::Transcript),
            false,
        )
        .unwrap();
    let mut record: CanonicalRecord = serde_json::from_value(serde_json::json!({
        "uuid":"r", "type":"user", "timestamp":"2026-09-01T00:00:00Z",
        "message":{"role":"user", "content":[]}
    }))
    .unwrap();
    db.store_mut()
        .upsert_records("s", &[record.clone()], false)
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let mut old = metrics
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap()
        .rows
        .pop()
        .unwrap();
    let mut payload: Payload = serde_json::from_str(&old.payload_json).unwrap();
    let WholeSessionWarnings::Read { warnings, .. } = payload.answer.render(&TimeZone::UTC) else {
        panic!("expected timed warning answer");
    };
    assert_eq!(warnings.checks.repeated_failure, CheckState::Complete);
    assert_eq!(warnings.checks.interruption, CheckState::Complete);
    assert_eq!(warnings.tool_results, 0);

    record.message.content = Some(
        serde_json::from_value(serde_json::json!([
            {"type":"tool_result", "tool_use_id":"t", "is_error":true, "content":"synthetic failure"}
        ]))
        .unwrap(),
    );
    db.store_mut()
        .upsert_records("s", &[record], false)
        .unwrap();
    let connection = Connection::open(db.path()).unwrap();
    let parent: (bool, bool) = connection
        .query_row(
            "SELECT has_conflict,is_tool_result_carrier FROM records WHERE uuid='r'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(parent, (true, false));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM tool_results", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM session_warning_inputs WHERE session_id='s'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(revision > old.revision);
    // Simulate the old reader's valid complete-zero answer after the conflict
    // already existed. Source revision checks alone cannot reject this row.
    old.revision = revision;
    old.rule_version = 2;
    payload.version = 2;
    payload.revision = revision;
    old.payload_json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        db.store_mut()
            .publish_session_warnings(&[old])
            .unwrap()
            .published,
        1
    );
    let saved: (i64, i64) = connection
        .query_row(
            "SELECT revision,rule_version FROM saved_session_warnings WHERE session_id='s'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, (revision, 2));

    assert!(!metrics.session_warning_cache_is_current().unwrap());
    let found = metrics
        .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
        .unwrap();
    let WholeSessionWarnings::Read { warnings, .. } = &found["s"] else {
        panic!("expected timed warning answer");
    };
    let incomplete = CheckState::Incomplete {
        missing: vec![Missing::OutcomeNotSaved { records: 1 }],
    };
    assert_eq!(warnings.checks.repeated_failure, incomplete);
    assert_eq!(warnings.checks.interruption, incomplete);
    assert!(warnings.warnings.is_empty());
    assert!(metrics.session_warning_refresh_needed());
    let refresh = metrics
        .prepare_session_warning_refresh(&[])
        .unwrap()
        .unwrap();
    assert_eq!(refresh.rows.len(), 1);
    assert_eq!(
        (refresh.rows[0].revision, refresh.rows[0].rule_version),
        (revision, SESSION_WARNINGS_CACHE_VERSION)
    );
    db.store_mut()
        .publish_session_warnings(&refresh.rows)
        .unwrap();
    assert!(metrics.session_warning_cache_is_current().unwrap());
    assert_eq!(
        metrics
            .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
            .unwrap(),
        found
    );
    assert!(
        metrics
            .prepare_session_warning_refresh(&[])
            .unwrap()
            .is_none()
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
                table_name: "records" | "record_previews" | "tool_uses" | "tool_results"
                    | "reply_admissions",
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
            .whole_session_warnings(&["s"], &TimeZone::UTC)
            .is_err()
    );
    // A window still reads lightweight timed membership. It must not enter
    // the warning loader, whose result/call/preview reads this guard refuses.
    let window_reader = MetricsDb::open(db.path()).unwrap();
    window_reader
        .connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "record_previews" | "tool_uses" | "tool_results" | "reply_admissions",
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
            .whole_session_warnings(&["s", "other"], &TimeZone::UTC)
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
            .whole_session_warnings(&["s"], &TimeZone::UTC)
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

/// An answer saved before the repeated-calls rule was removed, holding one
/// such warning and its check, exactly as rule version 3 wrote them.
fn with_old_repeated_calls(payload_json: &str, version: i64) -> String {
    let mut value: serde_json::Value = serde_json::from_str(payload_json).unwrap();
    value["version"] = version.into();
    let answer = &mut value["answer"]["warnings"];
    answer["checks"]["repeated_calls"] = serde_json::json!({"state": "complete"});
    answer["warnings"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "kind": "repeated_calls",
            "session_id": "s",
            "anchor": "h1",
            "at_ms": 1_788_220_800_000_i64,
            "record_uuids": ["h1", "a1"],
            "reason": {
                "reason": "repeated_calls",
                "repeats": 5,
                "active_duration_ms": 300_000,
                "worst": null,
                "start_ms": 1_788_220_800_000_i64
            },
            "repeat": null,
            "size": 5
        }));
    serde_json::to_string(&value).unwrap()
}

#[test]
fn a_saved_answer_holding_an_old_repeated_calls_warning_is_recounted_never_shown() {
    let mut db = seeded();
    warm(&mut db);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let direct = metrics
        .whole_session_warnings(&["s"], &TimeZone::UTC)
        .unwrap();
    let connection = Connection::open(db.path()).unwrap();
    let saved: String = connection
        .query_row(
            "SELECT payload_json FROM saved_session_warnings WHERE session_id='s'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Saved by the old rules (version 3), and, should one ever carry the
    // current version, the same content: neither is shown, neither fails.
    for version in [3, SESSION_WARNINGS_CACHE_VERSION] {
        connection
            .execute(
                "UPDATE saved_session_warnings SET payload_json=?1, rule_version=?2",
                rusqlite::params![with_old_repeated_calls(&saved, version), version],
            )
            .unwrap();
        let reader = MetricsDb::open(db.path()).unwrap();
        assert_eq!(
            reader
                .saved_whole_session_warnings(&["s"], &TimeZone::UTC)
                .unwrap(),
            direct,
            "{version}"
        );
        assert!(matches!(
            reader
                .current_saved_whole_session_warnings(&["s"], &TimeZone::UTC)
                .unwrap()["s"],
            WholeSessionWarnings::Missing
        ));
        assert!(reader.session_warning_refresh_needed(), "{version}");
        let start = 1_788_220_800_000;
        let window = Window::new(start - 86_400_000, start + 86_400_000 * 30).unwrap();
        let shown = reader
            .saved_warnings_in_window(window, &TimeZone::UTC)
            .unwrap();
        assert!(
            shown
                .iter()
                .flat_map(|session| &session.warnings)
                .all(|warning| warning.kind.as_str() != "repeated_calls")
        );
        // The refresh writes a four-rule answer that is then current.
        let refresh = reader
            .prepare_session_warning_refresh(&reader.session_warning_repairs())
            .unwrap()
            .unwrap();
        assert_eq!(refresh.rows.len(), 1);
        assert!(!refresh.rows[0].payload_json.contains("repeated_calls"));
        db.store_mut()
            .publish_session_warnings(&refresh.rows)
            .unwrap();
        assert!(metrics.session_warning_cache_is_current().unwrap());
        assert_eq!(
            MetricsDb::open(db.path())
                .unwrap()
                .current_saved_whole_session_warnings(&["s"], &TimeZone::UTC)
                .unwrap(),
            direct
        );
    }
}
