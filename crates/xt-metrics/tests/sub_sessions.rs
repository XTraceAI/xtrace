//! The verified sub-session walk under listed sessions: every depth, never a
//! named session, and a limit that counts what it leaves out.
use xt_metrics::MetricsDb;
use xt_store::{
    Host, SessionMeta, SessionSource,
    creation::{
        CODEX_THREAD_SPAWN_VERSION, CreationEvidence, CreationWitness, SessionCreationProof,
    },
};

fn spawn(child: &str, parent: &str) -> SessionCreationProof {
    SessionCreationProof {
        child_session_id: format!("codex-{child}"),
        child_host: Host::Codex,
        child_native_session_id: child.into(),
        parent_host: Host::Codex,
        parent_native_session_id: parent.into(),
        evidence_kind: CreationEvidence::CodexThreadSpawn,
        evidence_version: CODEX_THREAD_SPAWN_VERSION,
        witness: CreationWitness::RolloutOpeningSessionMeta,
    }
}

#[test]
fn walks_every_level_and_counts_what_the_limit_leaves_out() {
    let mut db = xt_fixtures::TempDb::empty().unwrap();
    // root → a, b; a → a1; b → (nothing); other → o. "listed" is named too.
    for native in ["root", "a", "b", "a1", "other", "o", "listed"] {
        let mut session =
            SessionMeta::new(format!("codex-{native}"), "codex", SessionSource::Fixture);
        session.native_session_id = Some(native.into());
        db.store_mut().upsert_session(&session, false).unwrap();
    }
    db.store_mut()
        .record_session_creations(
            &[
                spawn("a", "root"),
                spawn("b", "root"),
                spawn("a1", "a"),
                spawn("o", "other"),
                spawn("listed", "root"),
            ],
            1,
        )
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let all = metrics
        .sub_sessions(&["codex-root", "codex-listed"], 10)
        .unwrap();
    // Every level, nearer first; never a named session or another tree.
    let mut first: Vec<_> = all.sessions[..2].to_vec();
    first.sort();
    assert_eq!(first, ["codex-a", "codex-b"]);
    assert_eq!(all.sessions[2], "codex-a1");
    assert_eq!(all.sessions.len(), 3);
    assert!(all.not_shown.is_empty());
    // A limit keeps the nearest and counts the rest under the named session
    // whose walk found them.
    let capped = metrics.sub_sessions(&["codex-root"], 1).unwrap();
    assert_eq!(capped.sessions.len(), 1);
    assert_eq!(
        capped.not_shown.into_iter().collect::<Vec<_>>(),
        [("codex-root".to_owned(), 3)]
    );
}

fn child(db: &mut xt_fixtures::TempDb, native: &str, timestamp: Option<&str>) {
    let mut session = SessionMeta::new(format!("codex-{native}"), "codex", SessionSource::Fixture);
    session.native_session_id = Some(native.into());
    db.store_mut().upsert_session(&session, false).unwrap();
    if let Some(ts) = timestamp {
        let record = serde_json::from_value(serde_json::json!({
            "uuid": format!("{native}-event"), "type": "user", "timestamp": ts,
            "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}
        }))
        .unwrap();
        db.store_mut()
            .upsert_records(&format!("codex-{native}"), &[record], false)
            .unwrap();
    }
}

#[test]
fn capped_children_use_exact_instants_then_id_with_missing_times_last() {
    let mut db = xt_fixtures::TempDb::empty().unwrap();
    child(&mut db, "root", None);
    for (id, ts) in [
        ("offset", Some("2026-09-03T10:00:00-07:00")),
        ("fraction", Some("2026-09-03T17:00:00.0000000001Z")),
        ("a-tie", Some("2026-09-03T17:00:00Z")),
        ("earlier", Some("2026-09-03T16:00:00Z")),
        ("missing", None),
    ] {
        child(&mut db, id, ts);
        db.store_mut()
            .record_session_creations(&[spawn(id, "root")], 1)
            .unwrap();
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let capped = metrics.sub_sessions(&["codex-root"], 1).unwrap();
    assert_eq!(capped.sessions, ["codex-fraction"]);
    assert_eq!(capped.not_shown["codex-root"], 4);
    assert_eq!(
        metrics.sub_sessions(&["codex-root"], 5).unwrap().sessions,
        [
            "codex-fraction",
            "codex-a-tie",
            "codex-offset",
            "codex-earlier",
            "codex-missing"
        ]
    );
}

#[test]
fn actual_dashboard_cap_keeps_the_newest_of_1001_children() {
    let mut db = xt_fixtures::TempDb::empty().unwrap();
    child(&mut db, "root", None);
    child(&mut db, "newest", Some("2026-09-03T10:00:00-07:00"));
    db.store_mut()
        .record_session_creations(&[spawn("newest", "root")], 1)
        .unwrap();
    for n in 0..1000 {
        let id = format!("older-{n:04}");
        child(&mut db, &id, Some("2026-09-03T16:00:00Z"));
        db.store_mut()
            .record_session_creations(&[spawn(&id, "root")], 1)
            .unwrap();
    }
    let result = MetricsDb::open(db.path())
        .unwrap()
        .sub_sessions(&["codex-root"], 1000)
        .unwrap();
    assert_eq!(result.sessions.len(), 1000);
    assert_eq!(result.sessions[0], "codex-newest");
    assert!(!result.sessions.contains(&"codex-older-0999".to_owned()));
    assert_eq!(result.not_shown["codex-root"], 1);
    assert!(result.cut_short.is_empty());
}
