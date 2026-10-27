//! One coalesced worker for saved session warnings. Callbacks only request work.
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Pending {
    requested: bool,
    repairs: BTreeSet<String>,
    stop: bool,
}

#[derive(Clone)]
pub struct WarningCacheRequest(Arc<(Mutex<Pending>, Condvar)>);

impl WarningCacheRequest {
    pub fn request(&self, repairs: &[String]) {
        let (lock, changed) = &*self.0;
        let mut pending = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.stop {
            return;
        }
        pending.requested = true;
        pending.repairs.extend(repairs.iter().cloned());
        changed.notify_one();
    }
}

pub struct WarningCacheWorker {
    request: WarningCacheRequest,
    cancel: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl WarningCacheWorker {
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(move |repairs, cancelled| {
            let result = (|| -> Result<bool, Box<dyn std::error::Error>> {
                let metrics = xt_metrics::MetricsDb::open(&path)?;
                let Some(candidate) = metrics.prepare_session_warning_refresh_cancellable(
                    &repairs,
                    Arc::clone(&cancelled),
                )?
                else {
                    return Ok(false);
                };
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let mut store = xt_store::Store::open(&path)?;
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let outcome = store
                    .publish_session_warnings_cancellable(&candidate.rows, &|| {
                        cancelled.load(Ordering::Relaxed)
                    })?;
                Ok(outcome.rejected > 0 && !cancelled.load(Ordering::Relaxed))
            })();
            // Failed warm remains retryable on the next request.
            matches!(result, Ok(true))
        })
    }

    fn start_with(
        mut refresh: impl FnMut(Vec<String>, Arc<AtomicBool>) -> bool + Send + 'static,
    ) -> Self {
        let request =
            WarningCacheRequest(Arc::new((Mutex::new(Pending::default()), Condvar::new())));
        let cancel = Arc::new(AtomicBool::new(false));
        let (work, cancelled) = (request.clone(), Arc::clone(&cancel));
        let worker = thread::spawn(move || {
            loop {
                let repairs = {
                    let (lock, changed) = &*work.0;
                    let mut pending = lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while !pending.requested && !pending.stop {
                        pending = changed
                            .wait(pending)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if pending.stop {
                        return;
                    }
                    pending.requested = false;
                    std::mem::take(&mut pending.repairs).into_iter().collect()
                };
                if refresh(repairs, Arc::clone(&cancelled)) {
                    work.request(&[]);
                }
            }
        });
        let service = Self {
            request,
            cancel,
            worker: Mutex::new(Some(worker)),
        };
        service.request.request(&[]);
        service
    }

    pub fn requester(&self) -> WarningCacheRequest {
        self.request.clone()
    }

    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        {
            let (lock, changed) = &*self.request.0;
            lock.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stop = true;
            changed.notify_all();
        }
        let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let until = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
        // A blocked writer owns only its own connection. Cancellation forbids
        // publication after its current SQLite operation returns.
    }
}

impl Drop for WarningCacheWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_burst_coalesces_and_a_changed_snapshot_requests_one_follow_up() {
        let (started, calls) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut count = 0;
        let worker = WarningCacheWorker::start_with(move |repairs, _| {
            count += 1;
            started.send((count, repairs)).unwrap();
            if count == 1 {
                wait.recv().unwrap();
                return true;
            }
            false
        });
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            (1, Vec::<String>::new())
        );
        for _ in 0..40 {
            worker.requester().request(&[]);
        }
        worker.requester().request(&["s".into()]);
        release.send(()).unwrap();
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            (2, vec!["s".to_owned()])
        );
        worker.shutdown();
        assert!(calls.try_recv().is_err());
    }

    #[test]
    fn shutdown_cancels_an_active_refresh_and_refuses_more_requests() {
        let (started, calls) = mpsc::channel();
        let worker = WarningCacheWorker::start_with(move |_, cancel| {
            started.send(()).unwrap();
            while !cancel.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            true
        });
        calls.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.shutdown();
        worker.requester().request(&["s".into()]);
        assert!(calls.try_recv().is_err());
    }
    #[test]
    fn a_changed_or_purged_session_rejects_the_old_answer_and_refreshes_again() {
        use xt_store::{
            CanonicalRecord, SessionMeta, SessionSource, Store,
            retention::{ContentRegistry, RetentionMode},
        };
        for purge in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("test.db");
            let mut store = Store::open(&path).unwrap();
            store
                .set_retention_mode(RetentionMode::FullContent)
                .unwrap();
            store
                .upsert_session(
                    &SessionMeta::new("synthetic", "claude", SessionSource::Transcript),
                    true,
                )
                .unwrap();
            let record = |text: &str| -> CanonicalRecord {
                serde_json::from_value(serde_json::json!({"uuid":"r","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":text}]}})).unwrap()
            };
            store
                .upsert_records("synthetic", &[record("a synthetic request")], true)
                .unwrap();
            let (started, reads) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let (published, writes) = mpsc::channel();
            let worker_path = path.clone();
            let mut first = true;
            let worker = WarningCacheWorker::start_with(move |repairs, cancel| {
                let metrics = xt_metrics::MetricsDb::open(&worker_path).unwrap();
                let candidate = metrics
                    .prepare_session_warning_refresh_cancellable(&repairs, Arc::clone(&cancel))
                    .unwrap()
                    .unwrap();
                if first {
                    first = false;
                    started
                        .send(candidate.rows[0].payload_json.clone())
                        .unwrap();
                    wait.recv().unwrap();
                }
                let outcome = Store::open(&worker_path)
                    .unwrap()
                    .publish_session_warnings_cancellable(&candidate.rows, &|| {
                        cancel.load(Ordering::Relaxed)
                    })
                    .unwrap();
                published.send(outcome).unwrap();
                outcome.rejected > 0
            });
            let stale = reads.recv_timeout(Duration::from_secs(2)).unwrap();
            if purge {
                store.purge_content(&ContentRegistry::default()).unwrap();
            } else {
                store
                    .upsert_records("synthetic", &[record("a changed synthetic request")], true)
                    .unwrap();
            }
            release.send(()).unwrap();
            let rejected = writes.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!((rejected.published, rejected.rejected), (0, 1));
            let fresh = writes.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!((fresh.published, fresh.rejected), (1, 0));
            worker.shutdown();
            let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
            assert!(metrics.session_warning_cache_is_current().unwrap());
            let sql = rusqlite::Connection::open(&path).unwrap();
            let payload: String = sql
                .query_row(
                    "SELECT payload_json FROM saved_session_warnings WHERE session_id='synthetic'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_ne!(payload, stale);
        }
    }

    #[test]
    fn startup_warms_old_checkpoint_skipped_sessions_without_source_replay() {
        use xt_store::{
            CanonicalRecord, SessionMeta, SessionSource, Store, batch::NativeCheckpoint,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        let mut store = Store::open(&path).unwrap();
        for id in ["empty", "untimed", "quiet"] {
            store
                .upsert_session(
                    &SessionMeta::new(id, "claude", SessionSource::Transcript),
                    false,
                )
                .unwrap();
        }
        let untimed:CanonicalRecord=serde_json::from_value(serde_json::json!({"uuid":"untimed-r","type":"user","message":{"role":"user","content":[{"type":"text","text":"a synthetic request"}]}})).unwrap();
        let timed:CanonicalRecord=serde_json::from_value(serde_json::json!({"uuid":"quiet-r","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"a synthetic request"}]}})).unwrap();
        store.upsert_records("untimed", &[untimed], false).unwrap();
        store.upsert_records("quiet", &[timed], false).unwrap();
        let checkpoint = NativeCheckpoint {
            source: SessionSource::Transcript,
            cursor_key: "claude:/no-such-synthetic-source.jsonl".into(),
            generation: "{}".into(),
            position: 4096,
            updated_at: 1,
        };
        store.record_native_checkpoint(&checkpoint).unwrap();
        drop(store);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch(include_str!(
            "../../../../crates/xt-store/tests/fixtures/undo_saved_session_warnings.sql"
        ))
        .unwrap();
        sql.execute_batch("DELETE FROM schema_version WHERE version=29; DELETE FROM migration_fingerprints WHERE version=29;").unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .native_checkpoint(SessionSource::Transcript, &checkpoint.cursor_key)
                .unwrap(),
            Some(checkpoint.clone())
        );
        for _ in 0..2 {
            let worker = WarningCacheWorker::start(path.clone());
            let until = Instant::now() + Duration::from_secs(3);
            while !xt_metrics::MetricsDb::open(&path)
                .unwrap()
                .session_warning_cache_is_current()
                .unwrap()
            {
                assert!(Instant::now() < until, "startup did not warm");
                thread::sleep(Duration::from_millis(5));
            }
            worker.shutdown();
            assert_eq!(
                sql.query_row("SELECT count(*) FROM saved_session_warnings", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                3
            );
            assert_eq!(
                store
                    .native_checkpoint(SessionSource::Transcript, &checkpoint.cursor_key)
                    .unwrap(),
                Some(checkpoint.clone())
            );
            sql.execute("UPDATE saved_session_warnings SET rule_version=999", [])
                .unwrap();
        }
    }
}
