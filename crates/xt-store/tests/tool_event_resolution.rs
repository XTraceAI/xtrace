//! Legal observations from the earlier public INSERT API. Actual af60 API
//! upgrade probes are retained separately; these reproduce its unchanged SQL
//! shape without altering migration history or inventing invalid stored rows.
use rusqlite::{Connection, OpenFlags, params, types::Value};
use xt_fixtures::TempDb;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    ingest::{self, ToolEvent, ToolEventResolution, ToolEventUncertainty, ToolKind},
};

fn event(owner: &str, source: SessionSource, id: &str) -> ToolEvent {
    ToolEvent {
        session_id: owner.into(),
        source,
        source_event_id: id.into(),
        timestamp: Some("2026-09-07T12:00:00.1231000000010Z".into()),
        name: "stop_hook_summary".into(),
        kind: ToolKind::Hook,
        server: None,
        tool: None,
        skill: None,
    }
}
fn database() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    for owner in ["A", "B", "C"] {
        db.store_mut()
            .upsert_session(
                &SessionMeta::new(owner, "claude", SessionSource::Transcript),
                false,
            )
            .unwrap();
    }
    db
}
fn legacy_insert(db: &TempDb, event: &ToolEvent) -> i64 {
    let sql = Connection::open(db.path()).unwrap();
    sql.execute("INSERT INTO tool_uses(session_id,source,source_event_id,name,kind,server,tool,skill,event_ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![event.session_id,event.source,event.source_event_id,event.name,event.kind,event.server,event.tool,event.skill,event.timestamp]).unwrap();
    sql.last_insert_rowid()
}
fn raw(db: &TempDb) -> Vec<Vec<Value>> {
    let sql = Connection::open(db.path()).unwrap();
    let mut query = sql.prepare("SELECT * FROM tool_uses ORDER BY id").unwrap();
    let width = query.column_count();
    query
        .query_map([], |r| (0..width).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn canonical(db: &mut TempDb, owner: &str) {
    let row: CanonicalRecord =
        serde_json::from_value(serde_json::json!({"uuid":"shared","type":"user","message":{}}))
            .unwrap();
    db.store_mut().upsert_records(owner, &[row], false).unwrap();
}

#[test]
fn matching_legacy_groups_resolve_once_without_changing_any_observation() {
    for unknown_time in [false, true] {
        for reverse in [false, true] {
            let mut db = database();
            let schema_before = db.store().schema_version().unwrap();
            let mut first = event("A", SessionSource::Transcript, "shared");
            let mut second = event("A", SessionSource::Plugin, "shared");
            second.timestamp = Some("2026-09-07T14:00:00.123100000001+02:00".into());
            if unknown_time {
                first.timestamp = None;
                second.timestamp = None;
            }
            let order = if reverse {
                [&second, &first]
            } else {
                [&first, &second]
            };
            let id = legacy_insert(&db, order[0]);
            legacy_insert(&db, order[1]);
            let before = raw(&db);
            let resolved = db.store().resolved_tool_events().unwrap();
            assert!(
                matches!(&resolved[..],[ToolEventResolution::Resolved(saved)] if saved.id==id && &saved.event==order[0])
            );
            for incoming in [&first, &second] {
                assert_eq!(db.store_mut().insert_tool_event(incoming).unwrap(), id);
            }
            let mut reopened = Store::open(db.path()).unwrap();
            assert_eq!(reopened.schema_version().unwrap(), schema_before);
            assert_eq!(reopened.resolved_tool_events().unwrap(), resolved);
            assert_eq!(reopened.insert_tool_event(&first).unwrap(), id);
            assert_eq!(
                reopened.tool_events("A").unwrap().len(),
                2,
                "raw compatibility reader preserves duplicates"
            );
            assert_eq!(raw(&db), before);
        }
    }
}

#[test]
fn every_disagreeing_fact_is_unresolved_and_neither_replay_can_hide_it() {
    for changed in [
        "name",
        "kind",
        "server",
        "tool",
        "skill",
        "precise_time",
        "missing_time",
    ] {
        let mut db = database();
        let first = event("A", SessionSource::Transcript, "shared");
        let mut second = event("A", SessionSource::Plugin, "shared");
        match changed {
            "name" => second.name = "other".into(),
            "kind" => second.kind = ToolKind::Command,
            "server" => second.server = Some("server".into()),
            "tool" => second.tool = Some("tool".into()),
            "skill" => second.skill = Some("skill".into()),
            "precise_time" => second.timestamp = Some("2026-09-07T12:00:00.123100000002Z".into()),
            "missing_time" => second.timestamp = None,
            _ => unreachable!(),
        }
        legacy_insert(&db, &first);
        legacy_insert(&db, &second);
        let before = raw(&db);
        assert!(
            matches!(&db.store().resolved_tool_events().unwrap()[..],
            [ToolEventResolution::Unresolved {reason:ToolEventUncertainty::ConflictingFacts,observations}] if observations.len()==2),
            "{changed}"
        );
        for incoming in [&first, &second] {
            assert!(
                db.store_mut().insert_tool_event(incoming).is_err(),
                "{changed}"
            );
        }
        assert_eq!(raw(&db), before);
    }
}

#[test]
fn saved_and_canonical_owners_must_all_agree() {
    for (owners, canonical_owner, resolved) in [
        (vec!["A", "B"], None, false),
        (vec!["A", "B"], Some("A"), false),
        (vec!["A", "B"], Some("C"), false),
        (vec!["A", "A"], Some("A"), true),
        (vec!["A"], Some("B"), false),
        (vec!["A"], Some("A"), true),
        (vec!["A"], None, true),
    ] {
        let mut db = database();
        let events = owners
            .iter()
            .enumerate()
            .map(|(i, owner)| {
                event(
                    owner,
                    if i == 0 {
                        SessionSource::Transcript
                    } else {
                        SessionSource::Plugin
                    },
                    "shared",
                )
            })
            .collect::<Vec<_>>();
        for incoming in &events {
            legacy_insert(&db, incoming);
        }
        if let Some(owner) = canonical_owner {
            canonical(&mut db, owner);
        }
        let before = raw(&db);
        let groups = db.store().resolved_tool_events().unwrap();
        if resolved {
            assert!(matches!(&groups[..], [ToolEventResolution::Resolved(_)]));
        } else {
            let [
                ToolEventResolution::Unresolved {
                    reason,
                    observations,
                },
            ] = &groups[..]
            else {
                panic!("{groups:?}")
            };
            assert_eq!(*reason, ToolEventUncertainty::ConflictingOwnership);
            assert_eq!(
                observations
                    .iter()
                    .map(|r| r.event.session_id.as_str())
                    .collect::<Vec<_>>(),
                owners
            );
            assert!(
                observations
                    .iter()
                    .all(|r| r.canonical_owner.as_deref() == canonical_owner)
            );
        }
        for incoming in &events {
            assert_eq!(db.store_mut().insert_tool_event(incoming).is_ok(), resolved);
        }
        assert_eq!(raw(&db), before);
    }
}

#[test]
fn exact_nonblank_ids_stay_separate_and_legacy_blank_rows_are_not_grouped() {
    let mut db = database();
    for id in ["shared", " shared "] {
        legacy_insert(&db, &event("A", SessionSource::Transcript, id));
    }
    for id in ["\t", "\u{a0}"] {
        for source in [SessionSource::Transcript, SessionSource::Plugin] {
            legacy_insert(&db, &event("A", source, id));
        }
    }
    let before = raw(&db);
    let groups = db.store().resolved_tool_events().unwrap();
    assert_eq!(groups.len(), 6);
    let mut known = Vec::new();
    for group in groups {
        match group {
            ToolEventResolution::Resolved(row) => known.push(row.event.source_event_id),
            ToolEventResolution::Unresolved {
                reason,
                observations,
            } => {
                assert_eq!(reason, ToolEventUncertainty::MissingIdentity);
                assert_eq!(
                    observations.len(),
                    1,
                    "blank spellings cannot establish a shared native identity"
                );
                assert!(observations[0].event.source_event_id.trim().is_empty());
            }
        }
    }
    assert_eq!(known, [" shared ", "shared"]);
    for id in ["\t", "\u{a0}"] {
        assert!(
            db.store_mut()
                .insert_tool_event(&event("A", SessionSource::Transcript, id))
                .is_err()
        );
    }
    assert_eq!(raw(&db), before);
}

#[test]
fn resolved_reads_use_the_supplied_read_only_snapshot() {
    let mut db = database();
    let schema_before = db.store().schema_version().unwrap();
    db.store_mut()
        .insert_tool_event(&event("A", SessionSource::Transcript, "first"))
        .unwrap();
    let reader = Connection::open_with_flags(db.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    reader.pragma_update(None, "query_only", true).unwrap();
    reader
        .prepare(ingest::TOOL_EVENT_OBSERVATIONS_QUERY)
        .unwrap();
    let before = raw(&db);
    reader.execute_batch("BEGIN").unwrap();
    let first = ingest::resolved_tool_events(&reader).unwrap();
    assert_eq!(raw(&db), before, "the read changes no stored rows");
    db.store_mut()
        .insert_tool_event(&event("A", SessionSource::Transcript, "later"))
        .unwrap();
    assert_eq!(ingest::resolved_tool_events(&reader).unwrap(), first);
    reader.execute_batch("COMMIT").unwrap();
    assert_eq!(ingest::resolved_tool_events(&reader).unwrap().len(), 2);
    assert_eq!(db.store().schema_version().unwrap(), schema_before);
}
