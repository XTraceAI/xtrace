//! Synthetic native-source integration: full reads still refuse while pages,
//! timeline tool anchors preserve their exact saved record and block.
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    ImportRequest, ProducerSource, import_native,
    readers_cli::{CancelToken, read_pin},
};
use xt_store::{Host, Store};
use xtrace_desktop::{
    dto::{
        ReaderLimit, SessionPageReason, SessionPageSelection, SessionPageStatus,
        SessionSourceReason, SessionSourceStatus,
    },
    native_index::DetailReaders,
    state::{AppState, StartupOptions},
};
const SID: &str = "11111111-2222-4333-8444-555555555555";
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}
#[test]
fn oversized_codex_conversation_pages_and_exact_calls_stay_in_order() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let source = home.join(format!(
        ".codex/sessions/rollout-2026-10-25T12-00-00-{SID}.jsonl"
    ));
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let mut file = fs::File::create(&source).unwrap();
    let mut ordinal = 0;
    let mut emit = |kind: &str, payload: serde_json::Value| {
        let n = ordinal;
        ordinal += 1;
        writeln!(file,"{}",json!({"ordinal":n,"type":kind,"timestamp":format!("2026-10-25T12:{:02}:{:02}Z",(n*2)/60,(n*2)%60),"payload":payload})).unwrap();
    };
    emit(
        "session_meta",
        json!({"id":SID,"timestamp":"2026-10-25T12:00:00Z","originator":"codex_cli",
        "cwd":"/synthetic/project","history_mode":"paginated","history_base":null}),
    );
    emit(
        "response_item",
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"start the updater tests"}]}),
    );
    for _ in 0..12 {
        emit(
            "response_item",
            json!({"type":"message","role":"assistant",
        "content":[{"type":"output_text","text":"synthetic setup"}]}),
        );
    }
    for n in 0..110 {
        emit(
            "response_item",
            json!({"type":"function_call","name":"write_stdin","call_id":format!("call-{n}"),
            "arguments":"{\"session_id\":123,\"chars\":\"\"}"}),
        );
        emit(
            "response_item",
            json!({"type":"function_call_output","call_id":format!("call-{n}"),
            "output":"synthetic updater test process is still running"}),
        );
    }
    for _ in 0..26 {
        emit(
            "event_msg",
            json!({"type":"synthetic_unused_event","padding":"p".repeat(1024*1024)}),
        );
    }
    drop(file);
    assert!(fs::metadata(&source).unwrap().len() > 24 * 1024 * 1024);
    let data = root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let db = data.join("xtrace.db");
    let mut store = Store::open(&db).unwrap();
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &ProducerSource::Bundle {
                pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
                root: repo().join("vendor/agent-plugins"),
            },
            python: None,
            observed_at: 1_793_000_000_000,
            cancel: None,
        },
    );
    assert!(report.complete(), "{report:?}");
    let id = store
        .session(&format!("codex-{SID}"))
        .unwrap()
        .unwrap()
        .meta
        .session_id;
    drop(store);
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(data),
            native_home: Some(home),
            ..Default::default()
        },
        || panic!("explicit data"),
        || panic!("explicit home"),
    )
    .unwrap();
    let readers = DetailReaders::bundle(repo().join("vendor/agent-plugins"), None);
    let token = CancelToken::new();
    assert!(matches!(
        state.session_transcript(&id, &token, &readers).unwrap(),
        SessionSourceStatus::Unavailable {
            reason: SessionSourceReason::ReaderLimit {
                limit: ReaderLimit::SourceBytes
            }
        }
    ));
    let first = state
        .session_transcript_page(&id, &SessionPageSelection::First, &token, &readers)
        .unwrap();
    let SessionPageStatus::Loaded {
        records: first_records,
        start: 0,
        end,
        total_records: total,
        next: Some(next),
        digest,
        ..
    } = first
    else {
        panic!("first {first:?}")
    };
    assert_eq!(first_records.len(), 200);
    assert!(total > end);
    let later = state
        .session_transcript_page(
            &id,
            &SessionPageSelection::Cursor {
                cursor: next.clone(),
            },
            &token,
            &readers,
        )
        .unwrap();
    let SessionPageStatus::Loaded {
        records: later_records,
        start,
        end: final_end,
        previous: Some(previous),
        next: None,
        ..
    } = later
    else {
        panic!("later {later:?}")
    };
    assert_eq!(start, end);
    assert_eq!(final_end, total);
    assert!(
        !later_records
            .iter()
            .any(|r| first_records.iter().any(|p| p.id == r.id))
    );
    let earlier = state
        .session_transcript_page(
            &id,
            &SessionPageSelection::Cursor { cursor: previous },
            &token,
            &readers,
        )
        .unwrap();
    let SessionPageStatus::Loaded {
        records: earlier_records,
        start: 0,
        ..
    } = earlier
    else {
        panic!("earlier {earlier:?}")
    };
    assert_eq!(earlier_records, first_records);
    let target = later_records
        .iter()
        .rev()
        .find(|r| {
            r.blocks.iter().any(|b| {
                matches!(
                    b,
                    xtrace_desktop::transcript_dto::SourceBlock::ToolCall { .. }
                )
            })
        })
        .unwrap();
    let request = SessionPageSelection::Anchor {
        expected: xtrace_desktop::dto::SessionPageTargetKind::ToolCall,
        record_uuid: target.id.clone(),
        block: Some(0),
        generation: Some(digest),
    };
    for _ in 0..2 {
        let anchored = state
            .session_transcript_page(&id, &request, &token, &readers)
            .unwrap();
        let SessionPageStatus::Loaded {
            records,
            target: Some(at),
            ..
        } = anchored
        else {
            panic!("anchor {anchored:?}")
        };
        assert_eq!(&records[at.record as usize], target);
        assert!(at.record > 0);
        assert_eq!(at.block, Some(0));
    }
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(matches!(
        state
            .session_transcript_page(&id, &request, &cancelled, &readers)
            .unwrap(),
        SessionPageStatus::Unavailable {
            reason: SessionPageReason::Read {
                detail: xtrace_desktop::dto::SessionExcerptReason::Source {
                    source: SessionSourceReason::Cancelled
                }
            }
        }
    ));
    let before_bytes = fs::read(&source).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&source)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert!(matches!(
        state
            .session_transcript_page(
                &id,
                &SessionPageSelection::Cursor { cursor: next },
                &token,
                &readers
            )
            .unwrap(),
        SessionPageStatus::Unavailable {
            reason: SessionPageReason::Changed
        }
    ));
    let after_bytes = fs::read(&source).unwrap();
    assert_eq!(&after_bytes[..before_bytes.len()], before_bytes);
}
