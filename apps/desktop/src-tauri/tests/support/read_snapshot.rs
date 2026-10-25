use rusqlite::{Connection, trace::TraceEvent};
use std::{cell::RefCell, path::Path};
use xt_metrics::MetricsDb;

struct PendingWrite {
    reads: usize,
    write: Option<Box<dyn FnOnce() -> Result<(), String>>>,
    result: Option<Result<(), String>>,
}

thread_local! {
    static PENDING: RefCell<Option<PendingWrite>> = const { RefCell::new(None) };
}

fn observe(event: TraceEvent<'_>) {
    let TraceEvent::Stmt(_, sql) = event else {
        return;
    };
    let first_word = sql.split_whitespace().next().unwrap_or_default();
    if !first_word.eq_ignore_ascii_case("SELECT") && !first_word.eq_ignore_ascii_case("WITH") {
        return;
    }
    let write = PENDING.with_borrow_mut(|pending| {
        let pending = pending.as_mut()?;
        pending.reads += 1;
        (pending.reads == 2).then(|| pending.write.take()).flatten()
    });
    if let Some(write) = write {
        let result = write();
        PENDING.with_borrow_mut(|pending| pending.as_mut().unwrap().result = Some(result));
    }
}

pub struct BetweenReads<'a> {
    db: &'a MetricsDb,
    active: bool,
}

impl BetweenReads<'_> {
    pub fn assert_committed(mut self) {
        self.db.observe_fixture_statements(None);
        let pending = PENDING
            .with_borrow_mut(Option::take)
            .expect("one pending write");
        self.active = false;
        // SQLite discards trace callback panics. Require a recorded successful
        // commit here, outside the callback, so a failed action cannot pass.
        assert!(pending.reads >= 2, "the app must perform successive reads");
        assert_eq!(
            pending.result,
            Some(Ok(())),
            "the between-read write committed"
        );
    }
}

impl Drop for BetweenReads<'_> {
    fn drop(&mut self) {
        if self.active {
            self.db.observe_fixture_statements(None);
            PENDING.with_borrow_mut(Option::take);
        }
    }
}

pub fn change_after_first_read<'a>(db: &'a MetricsDb, path: &Path, sql: &str) -> BetweenReads<'a> {
    let path = path.to_owned();
    let sql = sql.to_owned();
    PENDING.with_borrow_mut(|pending| {
        assert!(pending.is_none(), "a test can schedule only one writer");
        *pending = Some(PendingWrite {
            reads: 0,
            write: Some(Box::new(move || {
                Connection::open(path)
                    .and_then(|writer| writer.execute_batch(&sql))
                    .map_err(|error| error.to_string())
            })),
            result: None,
        });
    });
    db.observe_fixture_statements(Some(observe));
    BetweenReads { db, active: true }
}
