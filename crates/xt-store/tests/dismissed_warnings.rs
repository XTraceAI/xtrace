//! Dismissed session warnings (migration 28): a dismissal is stored once and
//! keeps its first time, undoing deletes it, the table's checks refuse what
//! no warning could be named, and the content purge leaves dismissals.
//! Synthetic disposable databases only.

use rusqlite::Connection;
use std::path::Path;
use tempfile::TempDir;
use xt_store::Store;

/// One stored dismissal: its session, warning identity, size and time.
type Dismissed = (String, String, Option<i64>, i64);

fn dismissed(session: &str, warning: &str, size: Option<i64>, at: i64) -> Dismissed {
    (session.into(), warning.into(), size, at)
}

/// One session's stored dismissals, by identity, read straight from the
/// table: the app reads them through `xt_metrics`, not the store.
fn stored(path: &Path, session: &str) -> Vec<Dismissed> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT session_id,warning_id,size,dismissed_at FROM dismissed_warnings
             WHERE session_id=?1 ORDER BY warning_id",
        )
        .unwrap()
        .query_map([session], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn migration_28_adds_an_empty_table() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        xt_store::LATEST_SCHEMA_VERSION
    );
    assert!(stored(&path, "s").is_empty());
}

#[test]
fn a_warning_is_dismissed_once_and_undone_by_deleting_its_row() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    let mut store = Store::open(&path).unwrap();
    assert!(
        store
            .dismiss_warning("s", "correction:r1", None, 10)
            .unwrap()
    );
    assert!(
        store
            .dismiss_warning("s", "repeated_failure:r2#0", Some(2), 20)
            .unwrap()
    );
    // The same warning of another session is its own row.
    assert!(
        store
            .dismiss_warning("t", "correction:r1", None, 30)
            .unwrap()
    );
    // A second dismissal at the same size changes nothing and keeps the
    // first time.
    assert!(
        !store
            .dismiss_warning("s", "correction:r1", None, 40)
            .unwrap()
    );
    assert!(
        !store
            .dismiss_warning("s", "repeated_failure:r2#0", Some(2), 45)
            .unwrap()
    );
    assert_eq!(
        stored(&path, "s"),
        vec![
            dismissed("s", "correction:r1", None, 10),
            dismissed("s", "repeated_failure:r2#0", Some(2), 20),
        ]
    );
    assert!(store.restore_warning("s", "correction:r1").unwrap());
    assert!(!store.restore_warning("s", "correction:r1").unwrap());
    assert_eq!(
        stored(&path, "s"),
        vec![dismissed("s", "repeated_failure:r2#0", Some(2), 20)]
    );
    assert_eq!(
        stored(&path, "t"),
        vec![dismissed("t", "correction:r1", None, 30)]
    );
    // Dismissed again after an undo: a new time.
    assert!(
        store
            .dismiss_warning("s", "correction:r1", None, 50)
            .unwrap()
    );
    assert_eq!(
        stored(&path, "s")[0],
        dismissed("s", "correction:r1", None, 50)
    );
}

#[test]
fn a_warning_that_grew_is_dismissed_again_at_its_new_size() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    let mut store = Store::open(&path).unwrap();
    assert!(
        store
            .dismiss_warning("s", "repeated_prompt:p1", Some(2), 10)
            .unwrap()
    );
    assert!(
        store
            .dismiss_warning("s", "repeated_prompt:p1", Some(3), 20)
            .unwrap()
    );
    assert_eq!(
        stored(&path, "s"),
        vec![dismissed("s", "repeated_prompt:p1", Some(3), 20)]
    );
    // A delayed dismissal read at size 2 must not overwrite a newer size 3.
    assert!(
        !store
            .dismiss_warning("s", "repeated_prompt:p1", Some(2), 10)
            .unwrap()
    );
    assert!(
        !store
            .dismiss_warning("s", "repeated_prompt:p1", Some(3), 30)
            .unwrap()
    );
    assert_eq!(
        stored(&path, "s"),
        vec![dismissed("s", "repeated_prompt:p1", Some(3), 20)]
    );
    assert!(store.restore_warning("s", "repeated_prompt:p1").unwrap());
    assert!(stored(&path, "s").is_empty());
    assert!(
        store
            .dismiss_warning("s", "repeated_prompt:p1", Some(2), 40)
            .unwrap()
    );
    assert_eq!(
        stored(&path, "s"),
        vec![dismissed("s", "repeated_prompt:p1", Some(2), 40)]
    );
}

#[test]
fn the_table_refuses_rows_no_warning_could_name() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    let mut store = Store::open(&path).unwrap();
    for (kind, size) in [
        ("correction", None),
        ("interruption", None),
        ("repeated_calls", Some(1)),
        ("repeated_prompt", Some(2)),
        ("repeated_failure", Some(2)),
        ("admitted_mistake", None),
    ] {
        assert!(
            store
                .dismiss_warning("s", &format!("{kind}:r#1"), size, 0)
                .unwrap(),
            "{kind}"
        );
    }
    let long_session = "s".repeat(257);
    let long_warning = format!("correction:{}", "r".repeat(600));
    for (session, warning, size, at) in [
        ("", "correction:r", None, 0),
        (" s", "correction:r", None, 0),
        ("s ", "correction:r", None, 0),
        (long_session.as_str(), "correction:r", None, 0),
        ("s", "", None, 0),
        ("s", "correction:", None, 0),
        ("s", "circling:r", None, 0),
        ("s", "Correction:r", None, 0),
        ("s", "r", None, 0),
        ("s", long_warning.as_str(), None, 0),
        ("s", "correction:r", None, -1),
        // A one-event kind has no size; one that grows always has one, of
        // at least 1.
        ("s", "correction:r", Some(2), 0),
        ("s", "interruption:r", Some(1), 0),
        ("s", "repeated_prompt:r", None, 0),
        ("s", "repeated_failure:r", Some(0), 0),
        ("s", "repeated_calls:r", None, 0),
        ("s", "repeated_calls:r", Some(u64::MAX), 0),
        ("s", "admitted_mistake:r", Some(1), 0),
        ("s", "admitted_mistake:", None, 0),
    ] {
        assert!(
            store.dismiss_warning(session, warning, size, at).is_err(),
            "{session:?} {warning:?} {size:?} {at}"
        );
    }
    // A time or a size that is not a whole number is refused too.
    let connection = Connection::open(&path).unwrap();
    for row in [
        "('s','correction:x',NULL,1.5)",
        "('s','repeated_prompt:x',2.5,1)",
        "('s','repeated_prompt:x','two',1)",
    ] {
        assert!(
            connection
                .execute(
                    &format!(
                        "INSERT INTO dismissed_warnings(session_id,warning_id,size,dismissed_at) VALUES{row}"
                    ),
                    []
                )
                .is_err(),
            "{row}"
        );
    }
    assert_eq!(stored(&path, "s").len(), 6);
}

#[test]
fn the_content_purge_keeps_dismissals() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("x.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .dismiss_warning("s", "repeated_failure:r1#0", Some(2), 10)
        .unwrap();
    store
        .purge_content(&xt_store::retention::ContentRegistry::default())
        .unwrap();
    assert_eq!(
        stored(&path, "s"),
        vec![dismissed("s", "repeated_failure:r1#0", Some(2), 10)]
    );
}
