//! Exact pull-request links for Codex sessions from the Codex app's own
//! `attach_artifact` record.
//!
//! Every file here is written by the test: short synthetic Codex rollouts in
//! a temporary home, indexed exactly as a reader stream indexes them. Nothing
//! here touches Git, GitHub, a network or real Codex history.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use xt_fixtures::TempDb;
use xt_ingest::native::{
    HostStatus,
    codex_pr_links::{PrBacklog, PrProgress, codex_pr_witness, continue_codex_pr_links},
    import_reader_lines,
    readers_cli::ReaderOutcome,
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
    session_titles::TitleLimits,
};
use xt_store::{
    Host, Store,
    pr_link::{PrConfidence, PrIdentity, StoredPrLink},
};

const T: i64 = 1_788_782_400_000;
const OPENER: &str = "019a0000-0000-7000-8000-0000000000aa";
const MERGER: &str = "019a0000-0000-7000-8000-0000000000bb";
const LATE: &str = "019a0000-0000-7000-8000-0000000000cc";
const URL: &str = "https://github.com/Example/Fixture/pull/94";
const SEEN: &str = "2026-09-07T12:00:02.500Z";
/// `SEEN` in UTC milliseconds.
const SEEN_MS: i64 = 1_788_782_402_500;
const LATER: &str = "2026-09-08T09:30:00Z";
const LATER_MS: i64 = 1_788_859_800_000;

fn home(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(home.join(".codex/sessions/2026/09/07")).unwrap();
    home
}

fn rollout_path(home: &Path, native: &str) -> PathBuf {
    home.join(".codex/sessions/2026/09/07")
        .join(format!("rollout-2026-09-07T12-00-00-{native}.jsonl"))
}

fn header(native: &str) -> Value {
    json!({"timestamp": "2026-09-07T12:00:00.000Z", "type": "session_meta",
           "payload": {"id": native, "originator": "Codex Desktop", "cwd": "/repo/fixture",
                       "source": "vscode"}})
}

/// The completed `attach_artifact` item as the Codex app writes it.
fn item(url: &str) -> Value {
    json!({"type": "McpToolCall", "id": "exec-synthetic", "server": "codex_app",
           "tool": "attach_artifact",
           "arguments": {"artifact_type": "pull_request", "url": url},
           "pluginId": "codex-app-tools@openai-bundled", "status": "completed",
           "result": {"content": [{"type": "text", "text": "{\"attached\":true}"}],
                      "isError": false},
           "duration": {"secs": 0, "nanos": 1000}})
}

/// The witness line for `thread`, with `edit` applied to its item.
fn witness_with(thread: &str, timestamp: &str, edit: impl FnOnce(&mut Value)) -> Value {
    let mut item = item(URL);
    edit(&mut item);
    json!({"timestamp": timestamp, "ordinal": 6409, "type": "event_msg",
           "payload": {"type": "item_completed", "thread_id": thread, "turn_id": "turn-synthetic",
                       "item": item, "started_at_ms": 1, "completed_at_ms": 2}})
}

fn witness(thread: &str, timestamp: &str) -> Value {
    witness_with(thread, timestamp, |_| {})
}

fn prompt() -> Value {
    json!({"timestamp": "2026-09-07T12:00:01Z", "type": "response_item",
           "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "Synthetic prompt never read"}]}})
}

fn body(lines: &[Value]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn write(path: &Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body(lines)).unwrap();
}

fn append(path: &Path, lines: &[Value]) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(body(lines).as_bytes()).unwrap();
}

/// Index Codex sessions exactly as a reader stream does: each session's
/// header names `path`, which becomes its recorded locator.
fn index(store: &mut Store, sessions: &[(&str, &Path)]) {
    let mut lines = Vec::new();
    for (native, path) in sessions {
        lines.push(
            json!({"type": "session", "host": "codex", "native_session_id": native,
                   "conversation_id": format!("codex-{native}"), "source_surface": "codex_cli",
                   "started_at": "2026-09-07T12:00:00.000Z", "cwd": "/repo/fixture",
                   "git_branch": null, "title": null, "path": path.display().to_string(),
                   "mtime": 1788782400.0})
            .to_string(),
        );
        lines.push(
            json!({"uuid": format!("5555{}-5555-4555-8555-000000000000", &native[native.len() - 4..]),
                   "type": "user", "cwd": "/repo/fixture", "timestamp": "2026-09-07T12:00:01Z",
                   "message": {"role": "user", "content": [{"type": "text", "text": "turn"}]}})
            .to_string(),
        );
    }
    let report = import_reader_lines(
        store,
        Host::Codex,
        "test".into(),
        lines.into_iter().map(Ok),
        T,
        || {
            Ok(ReaderOutcome {
                diagnostics: vec![],
                complete: true,
            })
        },
    );
    assert_eq!(report.status, HostStatus::Complete, "{report:?}");
}

/// One pass over `natives` queued in `backlog`, with the default limits.
fn pass(store: &mut Store, home: &Path, backlog: &mut PrBacklog, natives: &[&str]) -> PrProgress {
    backlog.add(natives.iter().copied());
    continue_codex_pr_links(store, home, backlog, spawn_limits(), None, false).unwrap()
}

fn link(native: &str, first: i64, last: i64) -> StoredPrLink {
    StoredPrLink {
        session_id: format!("codex-{native}"),
        pull_request: PrIdentity::from_url(URL).unwrap(),
        confidence: PrConfidence::Exact,
        first_seen_at: first,
        last_seen_at: last,
    }
}

#[test]
fn a_completed_pull_request_attachment_links_its_thread_exactly_at_its_own_time() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, OPENER);
    write(
        &path,
        &[header(OPENER), prompt(), witness(OPENER, SEEN), prompt()],
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &path)]);
    let before = fs::read(&path).unwrap();

    let progress = pass(db.store_mut(), &home, &mut PrBacklog::default(), &[OPENER]);
    assert_eq!((progress.decided, progress.witnesses), (1, 1));
    assert_eq!(progress.changed, 1);
    // The line's own time, never the scan's.
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(OPENER, SEEN_MS, SEEN_MS)]
    );
    // The canonical identity: owner and repository lowercased.
    let stored = db.store().all_pull_requests().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].identity.url(),
        "https://github.com/example/fixture/pull/94"
    );
    // The host file is only read.
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn the_same_url_in_a_compaction_or_a_model_item_links_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, OPENER);
    let copy = witness(OPENER, SEEN);
    let compacted = json!({"timestamp": SEEN, "type": "compacted",
        "payload": {"message": "summary", "replacement_history": [copy.clone(),
            {"type": "message", "role": "assistant", "content": [{"type": "output_text",
             "text": format!("Attached {URL}")}]}]}});
    let response = json!({"timestamp": SEEN, "type": "response_item",
        "payload": {"type": "function_call", "name": "attach_artifact",
                    "arguments": json!({"artifact_type": "pull_request", "url": URL}).to_string(),
                    "item": item(URL)}});
    let echoed = json!({"timestamp": SEEN, "type": "response_item", "payload": copy["payload"]});
    write(&path, &[header(OPENER), compacted, response, echoed]);
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &path)]);

    let progress = pass(db.store_mut(), &home, &mut PrBacklog::default(), &[OPENER]);
    assert_eq!((progress.decided, progress.witnesses), (1, 0));
    assert!(db.store().all_pr_links().unwrap().is_empty());
    assert!(db.store().all_pull_requests().unwrap().is_empty());
}

#[test]
fn an_unfinished_or_failed_attachment_links_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, OPENER);
    let lines = [
        header(OPENER),
        witness_with(OPENER, SEEN, |item| item["status"] = "in_progress".into()),
        witness_with(OPENER, SEEN, |item| item["status"] = "failed".into()),
        witness_with(OPENER, SEEN, |item| {
            item.as_object_mut().unwrap().remove("status");
        }),
        witness_with(OPENER, SEEN, |item| item["result"]["isError"] = true.into()),
    ];
    write(&path, &lines);
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &path)]);

    let progress = pass(db.store_mut(), &home, &mut PrBacklog::default(), &[OPENER]);
    assert_eq!((progress.decided, progress.witnesses), (1, 0));
    assert!(db.store().all_pr_links().unwrap().is_empty());
}

#[test]
fn another_artifact_server_tool_or_thread_links_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, OPENER);
    let lines = [
        header(OPENER),
        witness_with(OPENER, SEEN, |item| {
            item["arguments"]["artifact_type"] = "branch".into();
        }),
        witness_with(OPENER, SEEN, |item| {
            item["arguments"]
                .as_object_mut()
                .unwrap()
                .remove("artifact_type");
        }),
        witness_with(OPENER, SEEN, |item| item["server"] = "github".into()),
        witness_with(OPENER, SEEN, |item| item["tool"] = "open_artifact".into()),
        witness_with(OPENER, SEEN, |item| item["type"] = "FunctionCall".into()),
        // Not a pull request URL the Store accepts.
        witness_with(OPENER, SEEN, |item| {
            item["arguments"]["url"] = "https://github.com/example/fixture/pull/94/".into();
        }),
        witness_with(OPENER, SEEN, |item| {
            item["arguments"]["url"] = "https://github.com/example/fixture/issues/94".into();
        }),
        // History a fork copied from the thread it was forked from.
        witness(MERGER, SEEN),
        // No time of its own.
        {
            let mut line = witness(OPENER, SEEN);
            line.as_object_mut().unwrap().remove("timestamp");
            line
        },
        // Not a completed item.
        {
            let mut line = witness(OPENER, SEEN);
            line["payload"]["type"] = "item_started".into();
            line
        },
    ];
    write(&path, &lines);
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &path)]);

    let progress = pass(db.store_mut(), &home, &mut PrBacklog::default(), &[OPENER]);
    assert_eq!((progress.decided, progress.witnesses), (1, 0));
    assert!(db.store().all_pr_links().unwrap().is_empty());

    // The rule alone, on the same lines.
    for line in &lines {
        assert_eq!(
            codex_pr_witness(line.to_string().as_bytes(), OPENER),
            None,
            "{line}"
        );
    }
    assert_eq!(
        codex_pr_witness(witness(OPENER, SEEN).to_string().as_bytes(), OPENER),
        Some((PrIdentity::from_url(URL).unwrap(), SEEN_MS))
    );
}

#[test]
fn two_threads_attaching_one_pull_request_both_link_and_a_reread_changes_nothing() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let opener = rollout_path(&home, OPENER);
    let merger = rollout_path(&home, MERGER);
    write(&opener, &[header(OPENER), witness(OPENER, SEEN)]);
    // The later thread attaches it twice: once early, once when it merges.
    write(
        &merger,
        &[
            header(MERGER),
            witness(MERGER, LATER),
            prompt(),
            witness(MERGER, "2026-09-08T10:00:00Z"),
        ],
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &opener), (MERGER, &merger)]);

    let progress = pass(
        db.store_mut(),
        &home,
        &mut PrBacklog::default(),
        &[OPENER, MERGER],
    );
    assert_eq!((progress.decided, progress.witnesses), (2, 3));
    let merged_at = LATER_MS + 30 * 60 * 1000;
    let expected = [
        link(OPENER, SEEN_MS, SEEN_MS),
        link(MERGER, LATER_MS, merged_at),
    ];
    assert_eq!(db.store().all_pr_links().unwrap(), expected);
    assert_eq!(db.store().all_pull_requests().unwrap().len(), 1);

    // A restarted run reads both files whole again: nothing changes.
    let again = pass(
        db.store_mut(),
        &home,
        &mut PrBacklog::default(),
        &[OPENER, MERGER],
    );
    assert_eq!((again.decided, again.witnesses, again.changed), (2, 3, 0));
    assert_eq!(db.store().all_pr_links().unwrap(), expected);
    assert_eq!(db.store().all_pull_requests().unwrap().len(), 1);
}

#[test]
fn a_thread_indexed_later_is_read_then_and_appended_lines_are_read_alone() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, LATE);
    write(&path, &[header(LATE), witness(LATE, SEEN)]);
    let mut db = TempDb::empty().unwrap();
    let mut backlog = PrBacklog::default();

    // Not indexed yet: dropped from the queue, nothing read, nothing linked.
    let early = pass(db.store_mut(), &home, &mut backlog, &[LATE]);
    assert_eq!((early.decided, early.not_indexed), (1, 1));
    assert_eq!((early.witnesses, early.bytes_read), (0, 0));
    assert_eq!(backlog.threads(), 0);
    assert!(db.store().all_pr_links().unwrap().is_empty());

    // The scan that indexes it queues it again; its witness is read then.
    index(db.store_mut(), &[(LATE, &path)]);
    let indexed = pass(db.store_mut(), &home, &mut backlog, &[LATE]);
    assert_eq!((indexed.witnesses, indexed.changed), (1, 1));
    assert_eq!(indexed.bytes_read, fs::metadata(&path).unwrap().len());
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(LATE, SEEN_MS, SEEN_MS)]
    );

    // Unchanged: queued again by the next scan, nothing is read.
    let unchanged = pass(db.store_mut(), &home, &mut backlog, &[LATE]);
    assert_eq!((unchanged.decided, unchanged.bytes_read), (1, 0));

    // Appended lines, the last one torn: only what was appended is read,
    // and the torn line waits for its newline.
    let length = fs::metadata(&path).unwrap().len();
    append(&path, &[prompt(), witness(LATE, LATER)]);
    let torn = witness(LATE, "2026-09-09T00:00:00Z").to_string();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(torn.as_bytes()).unwrap();
    let grown = pass(db.store_mut(), &home, &mut backlog, &[LATE]);
    assert_eq!(grown.witnesses, 1);
    assert_eq!(
        grown.bytes_read,
        fs::metadata(&path).unwrap().len() - length
    );
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(LATE, SEEN_MS, LATER_MS)]
    );
    file.write_all(b"\n").unwrap();
    let finished = pass(db.store_mut(), &home, &mut backlog, &[LATE]);
    assert_eq!(finished.witnesses, 1);
    assert_eq!(finished.bytes_read, torn.len() as u64 + 1);
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(LATE, SEEN_MS, 1_788_912_000_000)]
    );
}

#[test]
fn a_history_larger_than_one_pass_is_read_in_parts_past_an_overlong_line() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let path = rollout_path(&home, OPENER);
    // A compaction far longer than the line bound, with the URL inside it,
    // between two witnesses.
    let huge = json!({"timestamp": SEEN, "type": "compacted",
        "payload": {"message": "x".repeat(40_000), "replacement_history": [witness(OPENER, SEEN)]}});
    write(
        &path,
        &[
            header(OPENER),
            witness(OPENER, SEEN),
            huge,
            prompt(),
            witness(OPENER, LATER),
        ],
    );
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &path)]);
    let limits = TitleLimits {
        max_batch_bytes: 7_000,
        max_line_bytes: 4_096,
        deadline: Duration::from_secs(5),
        ..TitleLimits::default()
    };
    let mut backlog = PrBacklog::default();
    backlog.add([OPENER]);
    let (mut passes, mut witnesses, mut bytes) = (0_u64, 0, 0);
    while backlog.threads() > 0 {
        let progress =
            continue_codex_pr_links(db.store_mut(), &home, &mut backlog, limits, None, false)
                .unwrap();
        assert!(progress.advanced, "{progress:?}");
        assert!(progress.bytes_read <= limits.max_batch_bytes);
        passes += 1;
        witnesses += progress.witnesses;
        bytes += progress.bytes_read;
        assert!(passes < 50);
    }
    assert!(passes > 1, "{passes}");
    // Every byte at least once; only the start of a line a pass ended
    // inside is read again. Both witnesses, the compaction's copy never.
    let length = fs::metadata(&path).unwrap().len();
    assert!(bytes >= length, "{bytes} < {length}");
    assert!(bytes <= length + passes * limits.max_line_bytes as u64);
    assert_eq!(witnesses, 2);
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(OPENER, SEEN_MS, LATER_MS)]
    );
}

#[test]
fn a_watcher_run_sweeps_every_indexed_thread_and_announces_new_links_once() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = home(temp.path());
    let opener = rollout_path(&home, OPENER);
    let merger = rollout_path(&home, MERGER);
    write(&opener, &[header(OPENER), witness(OPENER, SEEN)]);
    write(&merger, &[header(MERGER), prompt()]);
    let mut db = TempDb::empty().unwrap();
    index(db.store_mut(), &[(OPENER, &opener), (MERGER, &merger)]);

    // Nothing queued: the sweep over the index finds both threads.
    let mut spawns = SpawnBacklog::starting();
    let mut changed = 0;
    let mut passes = 0;
    while spawns.pending() {
        let progress =
            continue_codex_spawns(db.store_mut(), &home, &mut spawns, spawn_limits(), None, T)
                .unwrap();
        changed += progress.prs.as_ref().map_or(0, |prs| prs.changed);
        passes += 1;
        assert!(passes < 20);
    }
    assert_eq!(changed, 1);
    assert_eq!(
        db.store().all_pr_links().unwrap(),
        [link(OPENER, SEEN_MS, SEEN_MS)]
    );

    // A later attachment in the other thread, queued by its scan.
    append(&merger, &[witness(MERGER, LATER)]);
    spawns.prs.add([MERGER]);
    let progress =
        continue_codex_spawns(db.store_mut(), &home, &mut spawns, spawn_limits(), None, T).unwrap();
    assert_eq!(progress.prs.as_ref().map(|prs| prs.changed), Some(1));
    // A new link is a change the lists show.
    assert!(progress.shown_changes() >= 1);
    assert_eq!(db.store().all_pr_links().unwrap().len(), 2);
}
