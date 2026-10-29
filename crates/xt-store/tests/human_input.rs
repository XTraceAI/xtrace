use serde_json::json;
use xt_store::{
    CanonicalRecord, Host, SessionMeta, Store,
    human_input::{InputAdjustment, OriginManifest, SessionOrigin},
};
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("index.db")).unwrap();
    let session: SessionMeta=serde_json::from_value(json!({"session_id":"codex-child","host":"codex","source":"readers_cli","native_session_id":"child"})).unwrap();
    store.upsert_session(&session, false).unwrap();
    let input: CanonicalRecord=serde_json::from_value(json!({"uuid":"input","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"hello pasted"}]}})).unwrap();
    store
        .upsert_records("codex-child", &[input], false)
        .unwrap();
    (dir, store)
}
fn origin() -> SessionOrigin {
    SessionOrigin {
        session_id: "codex-child".into(),
        host: Host::Codex,
        native_session_id: "child".into(),
        parent_host: Host::Codex,
        parent_native_session_id: "parent".into(),
        method: "explicit_session_id".into(),
        evidence_id: "audit-1".into(),
        launch_id: "call-1".into(),
    }
}
#[test]
fn origin_is_bound_idempotent_and_does_not_claim_exact_proof() {
    let (dir, mut store) = setup();
    rusqlite::Connection::open(dir.path().join("index.db"))
        .unwrap()
        .execute(
            "UPDATE sessions SET has_conflict=1 WHERE session_id='codex-child'",
            [],
        )
        .unwrap();
    let before = store.records("codex-child").unwrap().remove(0);
    let manifest = OriginManifest {
        version: 1,
        sessions: vec![origin()],
    };
    assert_eq!(
        store
            .import_human_session_origins(&manifest)
            .unwrap()
            .applied,
        1
    );
    assert_eq!(
        store
            .import_human_session_origins(&manifest)
            .unwrap()
            .unchanged,
        1
    );
    let after = store.records("codex-child").unwrap().remove(0);
    assert_eq!(after.human_is_eligible, Some(false));
    assert!(after.human_excluded);
    assert!(!after.confirmed_automated_input);
    assert_eq!(before.classification, after.classification);
    assert_eq!(before.text_len, after.text_len);
    assert_eq!(before.content_json, after.content_json);
    assert!(after.content_json.is_none());
    let mut wrong = origin();
    wrong.native_session_id = "other".into();
    assert!(
        store
            .import_human_session_origins(&OriginManifest {
                version: 1,
                sessions: vec![wrong]
            })
            .is_err()
    );
    let mut conflict = origin();
    conflict.parent_native_session_id = "another-parent".into();
    assert_eq!(
        store
            .import_human_session_origins(&OriginManifest {
                version: 1,
                sessions: vec![conflict]
            })
            .unwrap()
            .conflicted,
        1
    );
    assert_eq!(
        store.records("codex-child").unwrap()[0].human_is_eligible,
        Some(true)
    );
}
#[test]
fn adjustment_preserves_raw_length_and_conflict_withholds_it() {
    let (_dir, mut store) = setup();
    let mut input = InputAdjustment {
        record_uuid: "input".into(),
        session_id: "codex-child".into(),
        original_ts: "2026-09-01T00:00:00Z".into(),
        original_length: 12,
        retained_length: Some(5),
        reason: "question_reply".into(),
        native_item_id: None,
    };
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input.clone()])
            .unwrap()
            .applied,
        1
    );
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input.clone()])
            .unwrap()
            .unchanged,
        1
    );
    let row = &store.records("codex-child").unwrap()[0];
    assert_eq!(row.human_text_len, Some(5));
    assert_eq!(row.text_len, Some(12));
    assert_eq!(row.human_is_eligible, Some(true));
    input.original_length = 13;
    assert_eq!(
        store
            .apply_human_input_adjustments(&[input])
            .unwrap()
            .conflicted,
        1
    );
    assert_eq!(
        store.records("codex-child").unwrap()[0].human_text_len,
        Some(12)
    );
}

#[test]
fn human_origins_and_input_adjustments_invalidate_saved_warnings_on_change() {
    let (dir, mut store) = setup();
    let sql = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    let revision = || {
        sql.query_row(
            "SELECT revision FROM session_warning_inputs WHERE session_id=?1",
            ["codex-child"],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = revision();
    let manifest = OriginManifest {
        version: 1,
        sessions: vec![origin()],
    };
    store.import_human_session_origins(&manifest).unwrap();
    let changed = revision();
    assert!(changed > before);
    store.import_human_session_origins(&manifest).unwrap();
    assert_eq!(Some(revision()), Some(changed));
    let adjustment = InputAdjustment {
        record_uuid: "input".into(),
        session_id: "codex-child".into(),
        original_ts: "2026-09-01T00:00:00Z".into(),
        original_length: 12,
        retained_length: Some(5),
        reason: "question_reply".into(),
        native_item_id: None,
    };
    store
        .apply_human_input_adjustments(std::slice::from_ref(&adjustment))
        .unwrap();
    let changed_again = revision();
    assert!(changed_again > changed);
    store.apply_human_input_adjustments(&[adjustment]).unwrap();
    assert_eq!(Some(revision()), Some(changed_again));
}

#[test]
fn record_persons_reads_the_same_m02_answer_as_records() {
    let (_dir, mut store) = setup();
    // A reply saved under the assistant role, and an injected reminder saved
    // under the user role: neither is a person's message.
    let reply: CanonicalRecord = serde_json::from_value(json!({"uuid":"reply","type":"assistant","timestamp":"2026-09-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}})).unwrap();
    let reminder: CanonicalRecord = serde_json::from_value(json!({"uuid":"reminder","type":"user","isMeta":true,"timestamp":"2026-09-01T00:00:02Z","message":{"role":"user","content":[{"type":"text","text":"<system-reminder>synthetic</system-reminder>"}]}})).unwrap();
    store
        .upsert_records("codex-child", &[reply, reminder], false)
        .unwrap();
    let persons = store
        .record_persons("codex-child", &["input", "reply", "reminder", "missing"])
        .unwrap();
    // Known answers, not just agreement with `records()`.
    assert_eq!(persons["input"].person, Some(true));
    assert_eq!(persons["input"].whole_message, Some(true));
    assert_eq!(persons["reminder"].person, Some(false));
    for stored in store.records("codex-child").unwrap() {
        let facts = persons[&stored.uuid];
        assert_eq!(facts.person, stored.human_is_eligible, "{}", stored.uuid);
        assert_eq!(
            facts.whole_message,
            stored
                .human_text_len
                .zip(stored.text_len)
                .map(|(human, text)| human == text),
            "{}",
            stored.uuid
        );
    }
    assert!(!persons.contains_key("missing"));
    // Only part of the message is the person's: a reply to a question kept
    // its first five characters.
    store
        .apply_human_input_adjustments(&[InputAdjustment {
            record_uuid: "input".into(),
            session_id: "codex-child".into(),
            original_ts: "2026-09-01T00:00:00Z".into(),
            original_length: 12,
            retained_length: Some(5),
            reason: "question_reply".into(),
            native_item_id: None,
        }])
        .unwrap();
    let persons = store.record_persons("codex-child", &["input"]).unwrap();
    assert_eq!(persons["input"].person, Some(true));
    assert_eq!(persons["input"].whole_message, Some(false));
    assert!(
        store
            .record_persons("another-session", &["input"])
            .unwrap()
            .is_empty()
    );
}
