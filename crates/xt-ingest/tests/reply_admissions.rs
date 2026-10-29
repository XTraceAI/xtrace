//! Admission phrases on the import paths: a Claude transcript read by the
//! Rust reader and a Codex session from the pinned reader's output both get
//! one row per agent reply with text, holding phrase names only. A Claude
//! transcript indexed before migration 30 is read again once its checkpoint
//! is cleared, and its replies gain their rows. Synthetic data only.
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{fs, path::Path};
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource, import_native, import_reader_lines,
    readers_cli::{ReaderError, ReaderOutcome},
};
use xt_store::{Host, Store};

const SID: &str = "00000000-0000-4000-8000-0000000000a1";
const CODEX_NATIVE: &str = "019a0000-0000-7000-8000-0000000000a2";

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn stamp(n: u64) -> String {
    format!("2026-09-30T10:{:02}:{:02}.000Z", n / 60, n % 60)
}

/// `(record, phrases, conflicted)` for every kept row.
fn rows(path: &Path) -> Vec<(String, String, bool)> {
    Connection::open(path)
        .unwrap()
        .prepare("SELECT record_uuid,phrases,conflicted FROM reply_admissions ORDER BY 1")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn row(n: u64, phrases: &str) -> (String, String, bool) {
    (uuid(n), phrases.into(), false)
}

fn claude_line(n: u64, parent: Option<u64>, kind: &str, content: Value) -> String {
    json!({"uuid":uuid(n),"type":kind,"parentUuid":parent.map(uuid),"sessionId":SID,
        "timestamp":stamp(n),"cwd":"/repo/fixture","entrypoint":"cli","version":"2.0.0",
        "userType":"external","message":{"role":kind,"model":"fixture-model",
        "content":content}})
    .to_string()
}

fn claude_home(home: &Path) {
    let project = home.join(".claude/projects/-Users-fixture-repo");
    fs::create_dir_all(&project).unwrap();
    let main = [
        claude_line(
            1,
            None,
            "user",
            json!([{"type":"text","text":"The parser drops the last line."}]),
        ),
        claude_line(
            2,
            Some(1),
            "assistant",
            json!([{"type":"thinking","thinking":"sorry, I was wrong","signature":"x"},
                   {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]),
        ),
        claude_line(
            3,
            Some(2),
            "user",
            json!([{"type":"tool_result","tool_use_id":"t1","content":"ok"}]),
        ),
        claude_line(
            4,
            Some(3),
            "assistant",
            json!([{"type":"text","text":"You\u{2019}re right \u{2014} I misread the loop."}]),
        ),
        claude_line(
            5,
            Some(4),
            "assistant",
            json!([{"type":"text","text":"I wrongly assumed the input ended in a newline."}]),
        ),
    ];
    fs::write(project.join(format!("{SID}.jsonl")), main.join("\n") + "\n").unwrap();
    let agent = project.join(SID).join("subagents/agent-a");
    fs::create_dir_all(&agent).unwrap();
    let side = json!({"uuid":uuid(9),"type":"assistant","isSidechain":true,"agentId":"agent-a",
        "parentUuid":uuid(1),"sessionId":SID,"timestamp":stamp(9),"cwd":"/repo/fixture",
        "message":{"role":"assistant","model":"fixture-model",
        "content":[{"type":"text","text":"Sorry, my mistake."}]}});
    fs::write(
        agent.join("agent-a-00000000.jsonl"),
        side.to_string() + "\n",
    )
    .unwrap();
}

fn import_claude(store: &mut Store, home: &Path, at: i64) {
    let report = import_native(
        store,
        &ImportRequest {
            home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Checkout {
                pin: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.plugin-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at: at,
            cancel: None,
        },
    );
    assert_eq!(report.hosts[0].status, HostStatus::Complete, "{report:?}");
}

#[test]
fn claude_replies_with_text_get_phrase_names_and_older_history_fills_after_the_reset() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    claude_home(&home);
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    import_claude(&mut store, &home, 1_790_000_000_000);
    // Thinking and a tool call only: no row. The sidechain reply: no row.
    let expected = vec![row(4, "you_are_right i_misread"), row(5, "")];
    assert_eq!(rows(&path), expected);

    // History indexed before migration 30 has no rows, and its checkpoint
    // stops a resumed scan from reading it again.
    let sql = Connection::open(&path).unwrap();
    sql.execute("DELETE FROM reply_admissions", []).unwrap();
    import_claude(&mut store, &home, 1_790_000_000_001);
    assert!(rows(&path).is_empty());
    // Migration 30's reset: the next scan reads the transcript again.
    sql.execute(
        "DELETE FROM native_checkpoints WHERE source='transcript'",
        [],
    )
    .unwrap();
    import_claude(&mut store, &home, 1_790_000_000_002);
    assert_eq!(rows(&path), expected);
    // And again changes nothing.
    sql.execute(
        "DELETE FROM native_checkpoints WHERE source='transcript'",
        [],
    )
    .unwrap();
    import_claude(&mut store, &home, 1_790_000_000_003);
    assert_eq!(rows(&path), expected);
}

fn clean() -> Result<ReaderOutcome, ReaderError> {
    Ok(ReaderOutcome {
        diagnostics: Vec::new(),
        complete: true,
    })
}

fn codex_lines() -> Vec<std::io::Result<String>> {
    let header = json!({"type":"session","host":"codex","native_session_id":CODEX_NATIVE,
        "conversation_id":format!("codex-{CODEX_NATIVE}"),"source_surface":null,
        "started_at":"2026-09-30T10:00:00Z","cwd":"/synthetic","git_branch":null,"title":null,
        "path":format!("/synthetic/codex/{CODEX_NATIVE}"),"mtime":1.0});
    // The shapes the pinned Codex reader writes: a reply's text as one text
    // block, a reasoning summary as thinking, a call as tool_use.
    let records = [
        json!({"type":"user","uuid":uuid(1),"timestamp":stamp(1),
            "message":{"role":"user","content":"rename the module"}}),
        json!({"type":"assistant","uuid":uuid(2),"timestamp":stamp(2),
            "message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"my mistake"},
                {"type":"tool_use","id":"c1","name":"shell","input":{"command":["ls"]}}]}}),
        json!({"type":"assistant","uuid":uuid(3),"timestamp":stamp(3),
            "message":{"role":"assistant","content":[
                {"type":"text","text":"Good catch. Apologies \u{2014} I mixed up the names."}]}}),
        json!({"type":"assistant","uuid":uuid(4),"timestamp":stamp(4),
            "message":{"role":"assistant","content":[{"type":"text","text":"Renamed it."}]}}),
    ];
    std::iter::once(header)
        .chain(records)
        .map(|line| Ok(line.to_string()))
        .collect()
}

#[test]
fn codex_replies_from_the_reader_get_phrase_names_and_a_full_re_read_changes_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("index.sqlite");
    let mut store = Store::open(&path).unwrap();
    let report = import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        codex_lines(),
        7,
        clean,
    );
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
    let expected = vec![row(3, "good_catch apology i_mixed_up"), row(4, "")];
    assert_eq!(rows(&path), expected);
    // Codex is read whole on every scan.
    import_reader_lines(
        &mut store,
        Host::Codex,
        "test".into(),
        codex_lines(),
        8,
        clean,
    );
    assert_eq!(rows(&path), expected);
    // Nothing of the reply's text is stored with the metadata-only default.
    let kept: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 0);
}
