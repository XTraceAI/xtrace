use super::*;
use std::time::{Duration, Instant};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, retention::RetentionMode};

fn synthetic_state(root: &std::path::Path) -> AppState {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(home),
            ..Default::default()
        },
        || panic!("explicit synthetic data directory"),
        || panic!("explicit synthetic home"),
    )
    .unwrap();
    state.with_store(|store| {
        store.set_retention_mode(RetentionMode::FullContent)?;
        store.upsert_session(&SessionMeta::new("synthetic", "claude", SessionSource::Transcript), true)?;
        let records: Vec<CanonicalRecord> = serde_json::from_value(serde_json::json!([
            {"uuid":"answer","type":"assistant","timestamp":"2026-09-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":"A synthetic answer"}]}},
            {"uuid":"correction","type":"user","timestamp":"2026-09-01T00:01:00Z","message":{"role":"user","content":[{"type":"text","text":"No, use the other file"}]}}
        ])).unwrap();
        store.upsert_records("synthetic", &records, true)?;
        Ok(())
    }).unwrap();
    state
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < until, "warning worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn metric_reads_request_repairs_even_when_the_outer_read_fails() {
    let root = tempfile::tempdir().unwrap();
    let state = synthetic_state(root.path());
    let path = state.warning_cache_path().unwrap();
    let worker = crate::warning_cache::WarningCacheWorker::start(path.clone());
    state.attach_warning_cache(worker.requester());
    let sql = rusqlite::Connection::open(&path).unwrap();
    wait_until(|| {
        xt_metrics::MetricsDb::open(&path)
            .unwrap()
            .session_warning_cache_is_current()
            .unwrap()
    });
    for fail in [false, true] {
        sql.execute(
            "UPDATE saved_session_warnings SET payload_json='broken'",
            [],
        )
        .unwrap();
        let result = state.with_metric_inputs(|metrics, inputs| {
            let saved = metrics.saved_whole_session_warnings(&["synthetic"], &inputs.zone)?;
            assert_eq!(
                saved,
                metrics.whole_session_warnings(
                    &["synthetic"],
                    xt_metrics::RepeatThresholds::default(),
                    &inputs.zone
                )?
            );
            assert_eq!(metrics.session_warning_repairs(), ["synthetic"]);
            if fail {
                Err(StateError::InvalidMetricWindow)
            } else {
                Ok(())
            }
        });
        assert_eq!(result.is_err(), fail);
        wait_until(|| {
            sql.query_row("SELECT payload_json != 'broken' FROM saved_session_warnings WHERE session_id='synthetic'", [], |row| row.get::<_, bool>(0)).unwrap()
        });
    }
    worker.shutdown();
    state.shutdown();
    assert!(matches!(
        state.warning_cache_path(),
        Err(StateError::Closed)
    ));
}

#[test]
fn committed_purge_requests_a_fresh_answer_and_empty_purge_does_not_publish() {
    let root = tempfile::tempdir().unwrap();
    let state = synthetic_state(root.path());
    let path = state.warning_cache_path().unwrap();
    let worker = crate::warning_cache::WarningCacheWorker::start(path.clone());
    state.attach_warning_cache(worker.requester());
    let sql = rusqlite::Connection::open(&path).unwrap();
    wait_until(|| {
        xt_metrics::MetricsDb::open(&path)
            .unwrap()
            .session_warning_cache_is_current()
            .unwrap()
    });
    let before: String = sql
        .query_row(
            "SELECT payload_json FROM saved_session_warnings WHERE session_id='synthetic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut published = false;
    state
        .purge_stored_content(|| {
            published = true;
            state.request_warning_cache(&[]);
        })
        .unwrap();
    assert!(published);
    wait_until(|| {
        xt_metrics::MetricsDb::open(&path)
            .unwrap()
            .session_warning_cache_is_current()
            .unwrap()
    });
    let after: String = sql
        .query_row(
            "SELECT payload_json FROM saved_session_warnings WHERE session_id='synthetic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(before, after);
    state
        .purge_stored_content(|| panic!("empty purge published"))
        .unwrap();
    worker.shutdown();
    state.shutdown();
}
