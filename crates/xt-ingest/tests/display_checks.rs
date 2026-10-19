//! The display check of new sessions through the ordinary passes: a new
//! session is checking — not listed as a session of its own — until the
//! supported checks of who created it finish, a known child is a child, and
//! a checked main stays checked across ordinary later messages. Every public
//! snapshot a test takes is the store's own context read, the one the
//! Sessions and Dashboard reads use.
//!
//! Every file is written here: synthetic Codex histories in the shape of the
//! real `codex exec --json` control, and synthetic Claude transcripts.
//! Nothing real is read and no command runs.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource,
    claude_launch::{
        LaunchBacklog, LaunchLimits, SearchGaps, check::CheckProgress, continue_claude_launches,
    },
    import_reader_lines,
    readers_cli::ReaderOutcome,
    session_creation::{SpawnBacklog, SpawnProgress, continue_codex_spawns, spawn_limits},
    session_titles::TitleLimits,
};
use xt_store::{Host, Store, child_check::ChildCheck, session_list};

const T: i64 = 1_791_238_000_000;
const PARENT: &str = "01a00000-0000-7000-8000-0000000000aa";
const CHILD: &str = "01a10000-0000-7000-8000-0000000000c1";
const OTHER: &str = "01a10000-0000-7000-8000-0000000000c2";
const MAIN: &str = "01a10000-0000-7000-8000-0000000000c3";
const LATER: &str = "01a10000-0000-7000-8000-0000000000c4";
const FIRST: &str = "0d000000-0000-4000-8000-0000000000f1";
const HANDLE: u64 = 69773;
/// The launch call's time; everything else is an offset from it.
const AT: i64 = 1_791_238_643_642;

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn js(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

struct Home {
    _temp: tempfile::TempDir,
    home: PathBuf,
    db: PathBuf,
}

impl Home {
    fn new() -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        fs::create_dir_all(home.join(".claude/projects")).unwrap();
        Self {
            db: root.join("index.sqlite"),
            _temp: temp,
            home,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }

    fn day(&self, day: &str) -> PathBuf {
        let path = self.home.join(".codex/sessions").join(day);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// The display state a list reads for each named session, in order: the
    /// public context read, on a connection of its own.
    fn states(&self, sessions: &[&str]) -> Vec<ChildCheck> {
        let connection = rusqlite::Connection::open(&self.db).unwrap();
        let context: Vec<_> = sessions
            .chunks(session_list::MAX_CONTEXT)
            .flat_map(|chunk| session_list::context(&connection, chunk).unwrap())
            .collect();
        sessions
            .iter()
            .map(|id| {
                context
                    .iter()
                    .find(|row| row.id == *id)
                    .map_or(ChildCheck::Checking, |row| row.check)
            })
            .collect()
    }

    fn state(&self, session: &str) -> ChildCheck {
        self.states(&[session])[0]
    }

    fn attempt(&self, store: &Store, session: &str) -> i64 {
        store
            .child_check_attempt(session)
            .unwrap()
            .unwrap()
            .required
    }
}

fn codex(native: &str) -> String {
    format!("codex-{native}")
}

/// The real control's command, with an invented prompt.
const COMMAND: &str = "codex exec --json --ignore-user-config --ignore-rules --sandbox read-only \
    --skip-git-repo-check -m gpt-6-sol -c 'model_reasoning_effort=\"medium\"' \
    'Reply exactly SYNTHETIC-CONTROL. Do not use tools.'";

fn exec(command: &str) -> String {
    format!(
        "tools.exec_command({{cmd:{},workdir:\"/w\",sandbox_permissions:\"require_escalated\",\
         justification:\"Run the synthetic test.\",yield_time_ms:1000,max_output_tokens:2000}})",
        js(command)
    )
}

fn cell(command: &str) -> String {
    format!("text(await {});\n", exec(command))
}

fn call(offset: i64, id: &str, input: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "custom_tool_call", "call_id": id, "name": "exec", "input": input}})
}

fn output(offset: i64, id: &str, results: &[Value]) -> Value {
    let mut items = vec![json!({"type": "input_text",
        "text": "Script completed\nWall time 7.3 seconds\nOutput:\n"})];
    items.extend(
        results
            .iter()
            .map(|result| json!({"type": "input_text", "text": result.to_string()})),
    );
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "custom_tool_call_output", "call_id": id, "output": items}})
}

/// What the control's run had printed by its first result.
fn printed(thread: &str) -> String {
    format!(
        "Reading additional input from stdin...\n{{\"type\":\"thread.started\",\"thread_id\":\"{thread}\"}}\n{{\"type\":\"turn.started\"}}\n"
    )
}

/// The process still running, with what it had printed.
fn running(thread: &str) -> Value {
    json!({"chunk_id": "6f9747", "wall_time_seconds": 1.001, "session_id": HANDLE,
        "original_token_count": 35, "output": printed(thread)})
}

fn message(offset: i64, text: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": {
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]}})
}

fn launch(id: &str, offset: i64) -> Value {
    call(offset, id, &cell(COMMAND))
}

fn write_rows(path: &Path, rows: &[Value]) {
    let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
    fs::write(path, body).unwrap();
}

/// A paginated Codex rollout: its header, then `body` with consecutive
/// ordinals.
fn write_rollout(path: &Path, header: Value, body: &[Value]) {
    let mut rows = vec![header];
    for (index, row) in body.iter().enumerate() {
        let mut row = row.clone();
        row["ordinal"] = json!(index + 1);
        rows.push(row);
    }
    write_rows(path, &rows);
}

fn parent_path(home: &Home) -> PathBuf {
    home.day("2026/09/28")
        .join(format!("rollout-2026-09-28T16-14-04-{PARENT}.jsonl"))
}

fn write_parent(home: &Home, body: &[Value]) -> PathBuf {
    let path = parent_path(home);
    write_rollout(
        &path,
        json!({"timestamp": iso(-86_400_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": PARENT, "session_id": PARENT, "timestamp": iso(-86_400_000),
                "cwd": "/w", "originator": "Codex Desktop", "source": "vscode",
                "history_mode": "paginated"}}),
        body,
    );
    path
}

fn thread_path(home: &Home, thread: &str) -> PathBuf {
    home.day("2026/10/05")
        .join(format!("rollout-2026-10-05T15-17-30-{thread}.jsonl"))
}

/// A thread's own header: `source` `exec` opened at `opened` after the
/// launch, as the control's child has it, or another `source`.
fn header(thread: &str, source: &str, opened: i64, extra: &[(&str, Value)]) -> Value {
    let mut payload = json!({"session_id": thread, "id": thread, "timestamp": iso(opened),
        "cwd": "/w", "originator": "Codex Desktop", "cli_version": "0.157.0",
        "source": source, "thread_source": "user", "model_provider": "openai",
        "history_mode": "paginated"});
    for (key, value) in extra {
        payload[*key] = value.clone();
    }
    json!({"timestamp": iso(opened + 130), "type": "session_meta", "ordinal": 0,
        "payload": payload})
}

fn write_thread(home: &Home, thread: &str, header: Value, prompt_at: i64) -> PathBuf {
    let path = thread_path(home, thread);
    write_rollout(
        &path,
        header,
        &[
            json!({"timestamp": iso(prompt_at), "type": "response_item", "payload": {
                "type": "message", "role": "user", "id": "msg_synthetic",
                "content": [{"type": "input_text", "text": "Reply exactly SYNTHETIC-CONTROL."}]}}),
            message(prompt_at + 1277, "SYNTHETIC-CONTROL"),
        ],
    );
    path
}

/// Index one Codex session as the reader does, with `records` user inputs
/// from `first` (uuid, offset) on.
fn index(store: &mut Store, native: &str, path: &Path, first: (&str, i64), records: usize) {
    let mut lines = vec![
        json!({"type": "session", "host": "codex", "native_session_id": native,
               "conversation_id": codex(native), "source_surface": "codex_cli",
               "started_at": iso(-86_400_000), "cwd": "/w", "git_branch": null, "title": null,
               "path": path.display().to_string(), "mtime": 1791238643.0})
        .to_string(),
    ];
    for n in 0..records {
        let uuid = if n == 0 {
            first.0.to_owned()
        } else {
            format!("{}{:02}", &first.0[..first.0.len() - 2], n)
        };
        lines.push(
            json!({"uuid": uuid, "type": "user", "cwd": "/w",
                   "timestamp": iso(first.1 + n as i64 * 1000),
                   "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic request"}]}})
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

fn index_parent(store: &mut Store, path: &Path) {
    index(
        store,
        PARENT,
        path,
        ("5555aaaa-5555-4555-8555-000000000000", -2500),
        1,
    );
}

fn index_thread(store: &mut Store, native: &str, path: &Path, prompt_at: i64) {
    let uuid = format!("{}{}", &FIRST[..30], &native[30..]);
    index(store, native, path, (&uuid, prompt_at), 1);
}

/// What a watcher's Codex scan hands on for the threads it imported.
fn imported(spawns: &mut SpawnBacklog, natives: &[&str]) {
    spawns.add(natives.iter().copied());
    spawns.launches.add_threads(natives.iter().copied());
    spawns.launches.add_children(natives.iter().copied());
    spawns.checks.wake();
}

/// Run the watcher's passes until it would go quiet, taking `watch`'s
/// display states after every pass. Returns every pass and every snapshot.
fn drain(
    home: &Home,
    store: &mut Store,
    spawns: &mut SpawnBacklog,
    limits: TitleLimits,
    watch: &[&str],
) -> (Vec<SpawnProgress>, Vec<Vec<ChildCheck>>) {
    let mut passes = Vec::new();
    let mut frames = vec![home.states(watch)];
    for _ in 0..400 {
        if !spawns.pending() {
            return (passes, frames);
        }
        let progress = continue_codex_spawns(store, &home.home, spawns, limits, None, T).unwrap();
        frames.push(home.states(watch));
        passes.push(progress);
    }
    panic!("the worker never went quiet");
}

fn changed(passes: &[SpawnProgress]) -> (usize, usize) {
    (
        passes.iter().map(|pass| pass.changed).sum(),
        passes.iter().map(SpawnProgress::shown_changes).sum(),
    )
}

/// The child's own launch, answered by its running first result, with the
/// child opened after it: grouped, never a main session in any snapshot,
/// whichever is imported first. Its fact and relation are the only proof
/// changes; the parent's finished check is one more refresh; a restart
/// replays nothing.
#[test]
fn a_fresh_exec_child_is_checking_then_a_child_never_a_main_session() {
    let home = Home::new();
    let parent = write_parent(
        &home,
        &[
            message(-2000, "Starting"),
            launch("call_launch", 0),
            output(7319, "call_launch", &[running(CHILD)]),
        ],
    );
    let child = write_thread(&home, CHILD, header(CHILD, "exec", 6766, &[]), 8294);
    let mut store = home.store();
    // The child first, as the e24 control was.
    index_thread(&mut store, CHILD, &child, 8294);
    assert_eq!(home.state(&codex(CHILD)), ChildCheck::Checking);
    index_parent(&mut store, &parent);
    let mut spawns = SpawnBacklog::starting();
    let (passes, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(CHILD), &codex(PARENT)],
    );
    assert!(
        frames.iter().all(|frame| frame[0] != ChildCheck::Checked),
        "{frames:?}"
    );
    assert_eq!(
        frames.last().unwrap(),
        &[ChildCheck::Child, ChildCheck::Checked]
    );
    assert_eq!(
        changed(&passes),
        (2, 3),
        "fact and relation, plus the parent"
    );
    // A restarted worker reads the same own inputs: nothing changes.
    let mut restarted = SpawnBacklog::starting();
    let (passes, frames) = drain(
        &home,
        &mut store,
        &mut restarted,
        spawn_limits(),
        &[&codex(CHILD), &codex(PARENT)],
    );
    assert_eq!(changed(&passes), (0, 0));
    assert!(
        frames
            .iter()
            .all(|frame| frame == &[ChildCheck::Child, ChildCheck::Checked]),
        "{frames:?}"
    );
}

/// The launch is made but its first result is not saved yet when the child
/// is imported: the child stays checking — no main frame — until the result
/// arrives with its parent's next import, and is then grouped.
#[test]
fn a_child_whose_launch_result_arrives_later_is_never_shown_as_a_main_session() {
    let home = Home::new();
    let before = [message(-2000, "Starting"), launch("call_launch", 0)];
    let parent = write_parent(&home, &before);
    let child = write_thread(&home, CHILD, header(CHILD, "exec", 6766, &[]), 8294);
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, CHILD, &child, 8294);
    let mut spawns = SpawnBacklog::starting();
    let watch = [codex(CHILD)];
    let watch: Vec<&str> = watch.iter().map(String::as_str).collect();
    let (_, frames) = drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
    assert!(
        frames.iter().all(|frame| frame[0] == ChildCheck::Checking),
        "{frames:?}"
    );
    // The result is saved and the parent imported again.
    let mut after = before.to_vec();
    after.push(output(7319, "call_launch", &[running(CHILD)]));
    write_parent(&home, &after);
    index_parent(&mut store, &parent);
    imported(&mut spawns, &[PARENT]);
    let (_, frames) = drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
    assert!(
        frames.iter().all(|frame| frame[0] != ChildCheck::Checked),
        "{frames:?}"
    );
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Child);
}

/// A launch whose child check does not fit its allowance decides nothing:
/// no fact, link or finished check, and the child stays checking while the
/// worker goes quiet. A restarted worker looks at it once more and links it.
#[test]
fn a_child_check_set_aside_without_a_decision_keeps_the_child_checking() {
    let home = Home::new();
    let parent = write_parent(
        &home,
        &[
            message(-2000, "Starting"),
            launch("call_launch", 0),
            output(7319, "call_launch", &[running(CHILD)]),
        ],
    );
    // A large own header: its decoding needs more than the tight allowance.
    let padding = "x".repeat(300_000);
    let child = write_thread(
        &home,
        CHILD,
        header(
            CHILD,
            "exec",
            6766,
            &[("base_instructions", json!(padding))],
        ),
        8294,
    );
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, CHILD, &child, 8294);
    let tight = LaunchLimits {
        thread_memory: 600 << 10,
        ..LaunchLimits::default()
    };
    // The worker's pass order, with the launch work's allowance tight.
    let mut spawns = SpawnBacklog::default();
    let recording = xt_ingest::native::session_creation::record_codex_spawns(
        &mut store,
        &home.home,
        &[PARENT, CHILD],
        spawn_limits(),
        None,
        T,
    )
    .unwrap();
    for (session, attempt, opening) in &recording.openings {
        spawns
            .checks
            .note_codex(session, *attempt, opening.fresh_exec, &opening.key);
    }
    spawns.checks.wake();
    let mut quiet = false;
    for _ in 0..100 {
        let launches = continue_claude_launches(
            &mut store,
            &home.home,
            &mut spawns.launches,
            &tight,
            None,
            T,
        )
        .unwrap();
        let mut checks = CheckProgress::default();
        xt_ingest::native::claude_launch::check::continue_checks_into(
            &mut store,
            &home.home,
            &mut spawns.checks,
            &mut spawns.launches,
            &mut spawns.bash,
            &LaunchLimits::default(),
            None,
            &mut checks,
        )
        .unwrap();
        assert_eq!(home.state(&codex(CHILD)), ChildCheck::Checking);
        assert_eq!(launches.linked, 0);
        if !spawns.launches.pending() && !spawns.checks.pending() {
            quiet = true;
            break;
        }
    }
    assert!(quiet, "the worker can go quiet");
    assert!(store.child_facts(&codex(CHILD)).unwrap().is_empty());
    assert!(store.session_creation(&codex(CHILD)).unwrap().is_none());
    assert!(store.claude_launch_child_open(CHILD, Host::Codex).unwrap());
    // A restart looks once more, with room enough: the child is linked.
    let mut restarted = SpawnBacklog::starting();
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut restarted,
        spawn_limits(),
        &[&codex(CHILD)],
    );
    assert!(frames.iter().all(|frame| frame[0] != ChildCheck::Checked));
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Child);
}

/// A launch whose child was read and found not to be its child is decided:
/// the session's check finishes and it is listed as a main session.
#[test]
fn a_decided_mismatch_finishes_the_check() {
    let home = Home::new();
    let parent = write_parent(
        &home,
        &[
            message(-100_000, "Starting"),
            launch("call_launch", 0),
            output(7319, "call_launch", &[running(CHILD)]),
        ],
    );
    // Its first input is older than the launch: not the launch's child.
    let child = write_thread(&home, CHILD, header(CHILD, "exec", 6766, &[]), 8294);
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, CHILD, &child, -60_000);
    let mut spawns = SpawnBacklog::starting();
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(CHILD)],
    );
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Checked, "{frames:?}");
    assert!(!store.claude_launch_child_open(CHILD, Host::Codex).unwrap());
    assert!(store.child_facts(&codex(CHILD)).unwrap().is_empty());

    // Rewriting only the launch time keeps its call, result, offsets and
    // ordinals. The new launch input must still be checked again.
    write_parent(
        &home,
        &[
            message(-100_000, "Starting"),
            launch("call_launch", -70_000),
            output(7319, "call_launch", &[running(CHILD)]),
        ],
    );
    index_parent(&mut store, &parent);
    imported(&mut spawns, &[PARENT]);
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(CHILD)],
    );
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Child, "{frames:?}");
}

/// Another child's launch still waiting for its child, a launch with no
/// result made after a session opened, and a header that no `codex exec`
/// launch writes do not hold a session. A launch with no result made before
/// a fresh `exec` thread opened holds exactly that thread.
#[test]
fn only_launches_that_could_have_created_a_session_hold_it() {
    let home = Home::new();
    let parent = write_parent(
        &home,
        &[
            message(-2000, "Starting"),
            launch("call_other", 0),
            output(7319, "call_other", &[running(OTHER)]),
            launch("call_open", 20_000),
        ],
    );
    // Opened before the open launch: not its child. Opened after it: may be.
    let main = write_thread(&home, MAIN, header(MAIN, "exec", 10_000, &[]), 11_000);
    let later = write_thread(&home, LATER, header(LATER, "exec", 30_000, &[]), 31_000);
    // A fork of the fresh child's shape, and a plain session.
    let forked = write_thread(
        &home,
        CHILD,
        header(CHILD, "exec", 40_000, &[("forked_from_id", json!(PARENT))]),
        41_000,
    );
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, MAIN, &main, 11_000);
    index_thread(&mut store, LATER, &later, 31_000);
    index_thread(&mut store, CHILD, &forked, 41_000);
    let mut spawns = SpawnBacklog::starting();
    let watch = [codex(MAIN), codex(LATER), codex(CHILD), codex(PARENT)];
    let watch: Vec<&str> = watch.iter().map(String::as_str).collect();
    let (_, frames) = drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
    assert_eq!(
        frames.last().unwrap(),
        &[
            ChildCheck::Checked,
            ChildCheck::Checking,
            ChildCheck::Checked,
            ChildCheck::Checked
        ]
    );
    // The launch naming OTHER still waits; it holds no one else.
    assert!(store.claude_launch_child_open(OTHER, Host::Codex).unwrap());
}

/// Ordinary later messages leave a checked main checked in every snapshot,
/// at the same attempt; a rewritten opening reopens only that session, and
/// its old check no longer settles it.
#[test]
fn ordinary_messages_keep_a_main_checked_and_a_rewritten_opening_reopens_it() {
    let home = Home::new();
    let main_path = write_thread(&home, MAIN, header(MAIN, "cli", 10_000, &[]), 11_000);
    let other_path = write_thread(&home, LATER, header(LATER, "cli", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_thread(&mut store, MAIN, &main_path, 11_000);
    index_thread(&mut store, LATER, &other_path, 11_000);
    let mut spawns = SpawnBacklog::starting();
    let watch = [codex(MAIN), codex(LATER)];
    let watch: Vec<&str> = watch.iter().map(String::as_str).collect();
    drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
    assert_eq!(
        home.states(&watch),
        [ChildCheck::Checked, ChildCheck::Checked]
    );
    let attempt = home.attempt(&store, &codex(MAIN));
    let key = store
        .child_check_attempt(&codex(MAIN))
        .unwrap()
        .unwrap()
        .own_check_key;
    for records in 2..6 {
        // The thread's own file grows by a message, and is imported again.
        let mut body: String = fs::read_to_string(&main_path).unwrap();
        body.push_str(&format!("{}\n", {
            let mut row = message(20_000 + records as i64, "Later work");
            row["ordinal"] = json!(records + 1);
            row
        }));
        fs::write(&main_path, body).unwrap();
        index(
            &mut store,
            MAIN,
            &main_path,
            (&format!("{}{}", &FIRST[..30], &MAIN[30..]), 11_000),
            records,
        );
        assert_eq!(
            home.states(&watch),
            [ChildCheck::Checked, ChildCheck::Checked]
        );
        imported(&mut spawns, &[MAIN]);
        let (_, frames) = drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
        assert!(
            frames
                .iter()
                .all(|frame| frame == &[ChildCheck::Checked, ChildCheck::Checked]),
            "{frames:?}"
        );
    }
    assert_eq!(home.attempt(&store, &codex(MAIN)), attempt);
    // The opening line rewritten, the messages unchanged.
    let body = fs::read_to_string(&main_path).unwrap();
    let rest = body.split_once('\n').unwrap().1;
    let mut opening = header(MAIN, "cli", 10_000, &[]);
    opening["payload"]["cwd"] = json!("/elsewhere");
    fs::write(&main_path, format!("{opening}\n{rest}")).unwrap();
    imported(&mut spawns, &[MAIN]);
    let progress =
        continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
            .unwrap();
    let _ = progress;
    let reopened = home.attempt(&store, &codex(MAIN));
    assert!(reopened > attempt);
    // The old attempt and key no longer settle it.
    assert_eq!(
        store
            .settle_child_checks(&[(&codex(MAIN), attempt, key.as_deref().unwrap())])
            .unwrap(),
        0
    );
    let (_, frames) = drain(&home, &mut store, &mut spawns, spawn_limits(), &watch);
    assert!(frames.iter().all(|frame| frame[1] == ChildCheck::Checked));
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Checked);
    assert_eq!(home.attempt(&store, &codex(MAIN)), reopened);
}

/// A check finished under other detector versions runs again.
#[test]
fn other_detector_versions_check_again() {
    let home = Home::new();
    let main_path = write_thread(&home, MAIN, header(MAIN, "cli", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_thread(&mut store, MAIN, &main_path, 11_000);
    drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked);
    rusqlite::Connection::open(&home.db)
        .unwrap()
        .execute(
            "UPDATE session_child_checks SET detector_versions='check0'",
            [],
        )
        .unwrap();
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checking);
    let (passes, _) = drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked);
    assert_eq!(changed(&passes), (0, 1));
}

/// A session whose own rollout is gone stays checking, and the worker goes
/// quiet rather than asking for it again and again. A scan after the file is
/// back — with the reader unable to run — reads it from the index's own
/// locator, and the session is checked.
#[test]
fn a_missing_own_source_stays_checking_until_a_scan_finds_it_back() {
    let home = Home::new();
    let main_path = write_thread(&home, MAIN, header(MAIN, "cli", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_thread(&mut store, MAIN, &main_path, 11_000);
    drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked);
    let saved = fs::read(&main_path).unwrap();
    fs::remove_file(&main_path).unwrap();
    let mut spawns = SpawnBacklog::starting();
    let (passes, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert!(passes.len() < 30, "{} passes", passes.len());
    assert_eq!(frames[0][0], ChildCheck::Checked);
    assert!(
        frames
            .iter()
            .skip(1)
            .all(|frame| frame[0] == ChildCheck::Checking)
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checking);
    assert_eq!(passes.iter().map(|pass| pass.reopened).sum::<usize>(), 1);
    assert_eq!(passes.iter().map(|pass| pass.changed).sum::<usize>(), 0);
    assert_eq!(
        passes
            .iter()
            .filter(|pass| pass.shown_changes() > 0)
            .count(),
        1
    );
    // Timed passes alone ask for nothing more.
    for _ in 0..3 {
        let progress =
            continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
                .unwrap();
        assert!(!progress.advanced(), "{progress:?}");
    }
    fs::write(&main_path, saved).unwrap();
    let report = xt_ingest::native::scan_native_continued(
        &mut store,
        &ImportRequest {
            home: &home.home,
            hosts: &[Host::Codex],
            producer: &ProducerSource::Checkout {
                pin: home.home.join("no-such-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at: T,
            cancel: None,
        },
        xt_ingest::native::ScanMode::Resume,
        &mut |_| {},
        &mut spawns,
        spawn_limits(),
    );
    assert!(!report.complete(), "the reader cannot run: {report:?}");
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Checked);
}

/// One whole page of Codex sessions whose headers are read only because a
/// check asked, and after them Cursor sessions, which need no read: the
/// headers come back after the page that asked for them went by, and every
/// session is checked before the worker first goes quiet.
#[test]
fn every_session_past_one_page_is_checked_before_the_worker_goes_quiet() {
    use xt_ingest::native::claude_launch::check::CHECK_PAGE;
    let home = Home::new();
    let mut store = home.store();
    let natives: Vec<String> = (0..CHECK_PAGE)
        .map(|n| format!("01a20000-0000-7000-8000-{n:012x}"))
        .collect();
    for native in &natives {
        let path = thread_path(&home, native);
        write_rollout(
            &path,
            header(native, "cli", 10_000, &[]),
            &[message(11_000, "Work")],
        );
        index(
            &mut store,
            native,
            &path,
            (
                &format!("0d000000-0000-4000-8000-{}", &native[24..]),
                11_000,
            ),
            1,
        );
    }
    let cursor: Vec<String> = (0..20).map(|n| format!("cursor-native-{n:04}")).collect();
    for id in &cursor {
        let native = &id["cursor-".len()..];
        let lines = vec![
            json!({"type": "session", "host": "cursor", "native_session_id": native,
                   "conversation_id": id, "source_surface": "cursor",
                   "started_at": iso(0), "cwd": "/w", "git_branch": null, "title": null,
                   "path": "/w/cursor.db", "mtime": 1791238643.0})
            .to_string(),
            json!({"uuid": format!("{id}-input"), "type": "user", "cwd": "/w",
                   "timestamp": iso(1000),
                   "message": {"role": "user", "content": [{"type": "text", "text": "Synthetic"}]}})
            .to_string(),
        ];
        let report = import_reader_lines(
            &mut store,
            Host::Cursor,
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
    // No startup sweep, and the one-time pass over indexed sources already
    // done: each header is read only because a check asked.
    store
        .advance_session_creation_bootstrap(
            xt_store::creation::CreationEvidence::CodexThreadSpawn,
            xt_store::creation::CODEX_THREAD_SPAWN_VERSION,
            &xt_store::creation::CreationBootstrap {
                after_locator: None,
                complete: true,
            },
        )
        .unwrap();
    let mut spawns = SpawnBacklog::default();
    spawns.checks.wake();
    drain(&home, &mut store, &mut spawns, spawn_limits(), &[]);
    let ids: Vec<String> = natives
        .iter()
        .map(|native| codex(native))
        .chain(cursor.iter().cloned())
        .collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let states = home.states(&ids);
    assert!(
        states.iter().all(|state| *state == ChildCheck::Checked),
        "{} of {} checked",
        states
            .iter()
            .filter(|state| **state == ChildCheck::Checked)
            .count(),
        states.len()
    );
}

/// A Codex history that cannot be read — a row that is not JSON, or a
/// header naming another thread than its file — is skipped and counted: it
/// holds no fresh `exec` session, the worker does not spin on it, and it is
/// not read again until it changes. Readable again and imported, it is
/// searched and the count drops.
#[test]
fn a_history_that_cannot_be_read_is_skipped_and_holds_no_session() {
    for case in ["not json", "another thread's header"] {
        let home = Home::new();
        let parent = parent_path(&home);
        write_parent(&home, &[message(-2000, "Starting")]);
        let body = fs::read_to_string(&parent).unwrap();
        let broken = match case {
            "not json" => format!("{body}{{\"not\": \"a row\"\n"),
            _ => body.replacen(
                &format!("\"id\":\"{PARENT}\""),
                &format!("\"id\":\"{OTHER}\""),
                1,
            ),
        };
        assert_ne!(broken, body, "{case}");
        fs::write(&parent, &broken).unwrap();
        let main = write_thread(&home, MAIN, header(MAIN, "exec", 10_000, &[]), 11_000);
        let mut store = home.store();
        index_parent(&mut store, &parent);
        index_thread(&mut store, MAIN, &main, 11_000);
        let mut spawns = SpawnBacklog::starting();
        let (passes, frames) = drain(
            &home,
            &mut store,
            &mut spawns,
            spawn_limits(),
            &[&codex(MAIN)],
        );
        assert!(passes.len() < 40, "{case}: {} passes", passes.len());
        assert_eq!(frames.last().unwrap()[0], ChildCheck::Checked, "{case}");
        assert_eq!(
            store.claude_launch_summary().unwrap().groups_invalid,
            1,
            "{case}"
        );
        assert_eq!(
            spawns.launches.search_gaps(),
            Some(SearchGaps {
                unsearched: 1,
                forks_not_searched: 0,
                unfinished_line_skipped: 0,
                ..SearchGaps::default()
            }),
            "{case}"
        );
        // Nothing left to do: not read again until it changes.
        for _ in 0..3 {
            let progress =
                continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
                    .unwrap();
            assert!(!progress.advanced(), "{case}: {progress:?}");
        }
        write_parent(&home, &[message(-2000, "Starting")]);
        index_parent(&mut store, &parent);
        imported(&mut spawns, &[PARENT]);
        drain(
            &home,
            &mut store,
            &mut spawns,
            spawn_limits(),
            &[&codex(MAIN)],
        );
        assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked, "{case}");
        assert_eq!(
            spawns.launches.search_gaps(),
            Some(SearchGaps::default()),
            "{case}"
        );
    }
}

/// A fork whose header marks none of its rows as copied is a layout this
/// version does not read: it holds no session, and Settings counts it as a
/// fork not searched, never as a history that could not be read.
#[test]
fn a_copied_fork_is_counted_apart_from_unreadable_histories() {
    let home = Home::new();
    let parent = parent_path(&home);
    write_rollout(
        &parent,
        json!({"timestamp": iso(-86_400_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": PARENT, "session_id": PARENT, "timestamp": iso(-86_400_000),
                "cwd": "/w", "originator": "Codex Desktop", "source": "vscode",
                "history_mode": "paginated", "forked_from_id": OTHER}}),
        &[message(-2000, "Starting")],
    );
    let main = write_thread(&home, MAIN, header(MAIN, "exec", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, MAIN, &main, 11_000);
    let mut spawns = SpawnBacklog::starting();
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(frames.last().unwrap()[0], ChildCheck::Checked);
    assert_eq!(
        spawns.launches.search_gaps(),
        Some(SearchGaps {
            unsearched: 0,
            forks_not_searched: 1,
            unfinished_line_skipped: 0,
            ..SearchGaps::default()
        })
    );
}

/// A launch in a readable history still makes its child a child while
/// another history cannot be read; the unrelated fresh `exec` session is
/// checked.
#[test]
fn a_launch_in_a_readable_history_marks_its_child_while_another_is_skipped() {
    let home = Home::new();
    let parent = write_parent(
        &home,
        &[
            message(-2000, "Starting"),
            launch("call_launch", 0),
            output(7319, "call_launch", &[running(CHILD)]),
        ],
    );
    // Another thread's history, with a row that is not JSON.
    let broken = home
        .day("2026/09/29")
        .join(format!("rollout-2026-09-29T10-00-00-{LATER}.jsonl"));
    fs::write(
        &broken,
        format!(
            "{}\n{{\"not\": \"a row\"\n",
            json!({"timestamp": iso(-80_000_000), "type": "session_meta", "ordinal": 0,
                "payload": {"id": LATER, "session_id": LATER, "source": "vscode",
                    "history_mode": "paginated"}})
        ),
    )
    .unwrap();
    let child = write_thread(&home, CHILD, header(CHILD, "exec", 6766, &[]), 8294);
    let main = write_thread(&home, MAIN, header(MAIN, "exec", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, CHILD, &child, 8294);
    index_thread(&mut store, MAIN, &main, 11_000);
    let mut spawns = SpawnBacklog::starting();
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(CHILD), &codex(MAIN)],
    );
    assert!(
        frames.iter().all(|frame| frame[0] != ChildCheck::Checked),
        "{frames:?}"
    );
    assert_eq!(
        frames.last().unwrap(),
        &[ChildCheck::Child, ChildCheck::Checked]
    );
    assert_eq!(
        spawns.launches.search_gaps().map(|gaps| gaps.unsearched),
        Some(1)
    );
}

/// A Codex `/review` subagent's own history, as Codex 0.146 writes it (its
/// `session_id` names the thread it reviews), is read: it holds no session,
/// its own display check finishes, and it is not shown under its parent.
#[test]
fn a_review_subagent_history_is_read_and_its_own_check_finishes() {
    let home = Home::new();
    let parent = write_parent(&home, &[message(-2000, "Starting")]);
    let review = header(
        LATER,
        "exec",
        5_000,
        &[
            ("session_id", PARENT.into()),
            ("parent_thread_id", PARENT.into()),
            ("source", json!({"subagent": "review"})),
            ("thread_source", "subagent".into()),
            ("originator", "codex_exec".into()),
            ("multi_agent_version", "disabled".into()),
            ("subagent_history_start_ordinal", 3.into()),
        ],
    );
    let path = write_thread(&home, LATER, review, 6_000);
    let main = write_thread(&home, MAIN, header(MAIN, "exec", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_parent(&mut store, &parent);
    index_thread(&mut store, LATER, &path, 6_000);
    index_thread(&mut store, MAIN, &main, 11_000);
    let mut spawns = SpawnBacklog::starting();
    let (_, frames) = drain(
        &home,
        &mut store,
        &mut spawns,
        spawn_limits(),
        &[&codex(LATER), &codex(MAIN)],
    );
    assert_eq!(
        frames.last().unwrap(),
        &[ChildCheck::Checked, ChildCheck::Checked]
    );
    assert_eq!(
        store.claude_launch_group(LATER).unwrap().unwrap().status,
        xt_store::claude_launch::GroupStatus::Valid
    );
    assert_eq!(spawns.launches.search_gaps(), Some(SearchGaps::default()));
}

/// A spawned helper's inherited context starting with the header it copied,
/// below its verified boundary, is read; launches and results in the
/// inherited rows are not its own; its own launch after the boundary is
/// found. A second header among its own rows, or with no verified boundary,
/// still refuses the history.
#[test]
fn an_inherited_header_is_read_only_below_the_verified_boundary() {
    let home = Home::new();
    let root = "01a30000-0000-7000-8000-0000000000aa";
    let spawned = |inherited_below: Option<i64>| {
        let mut payload = json!({"id": PARENT, "session_id": root, "timestamp": iso(-86_400_000),
            "cwd": "/w", "originator": "Codex Desktop", "history_mode": "paginated",
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": root, "depth": 1}}},
            "parent_thread_id": root, "forked_from_id": root});
        if let Some(below) = inherited_below {
            payload["subagent_history_start_ordinal"] = json!(below);
        }
        json!({"timestamp": iso(-86_400_000), "type": "session_meta", "ordinal": 0,
            "payload": payload})
    };
    let copied_header = json!({"timestamp": iso(-90_000_000), "type": "session_meta",
        "payload": {"id": root, "session_id": root, "source": "vscode"}});
    // Inherited: the copied header, the root's own launch of OTHER and its
    // result. Own: a launch of CHILD and its result.
    let inherited = [
        copied_header.clone(),
        launch("call_inherited", -80_000_000),
        output(-79_990_000, "call_inherited", &[running(OTHER)]),
    ];
    let own = [
        launch("call_launch", 0),
        output(7319, "call_launch", &[running(CHILD)]),
    ];
    let child = write_thread(&home, CHILD, header(CHILD, "exec", 6766, &[]), 8294);
    let other = write_thread(&home, OTHER, header(OTHER, "exec", 6766, &[]), 8294);
    for (case, below, body, valid) in [
        (
            "boundary",
            Some(4),
            [&inherited[..], &own[..]].concat(),
            true,
        ),
        (
            "own second header at the boundary",
            Some(4),
            [&inherited[..], &[copied_header.clone()][..], &own[..]].concat(),
            false,
        ),
        (
            "no boundary",
            None,
            [&inherited[..], &own[..]].concat(),
            false,
        ),
    ] {
        let path = parent_path(&home);
        write_rollout(&path, spawned(below), &body);
        let mut store = home.store();
        index_parent(&mut store, &path);
        index_thread(&mut store, CHILD, &child, 8294);
        index_thread(&mut store, OTHER, &other, 8294);
        let mut backlog = LaunchBacklog::starting();
        for _ in 0..50 {
            if !backlog.pending() {
                break;
            }
            continue_claude_launches(
                &mut store,
                &home.home,
                &mut backlog,
                &LaunchLimits::default(),
                None,
                T,
            )
            .unwrap();
        }
        let summary = store.claude_launch_summary().unwrap();
        let status = store.claude_launch_group(PARENT).unwrap().unwrap().status;
        if valid {
            assert_eq!(
                (status, summary.candidates_codex),
                (xt_store::claude_launch::GroupStatus::Valid, 1),
                "{case}: {summary:?}"
            );
            assert!(!store.claude_launch_child_open(OTHER, Host::Codex).unwrap());
            assert!(
                store.child_facts(&codex(OTHER)).unwrap().is_empty(),
                "{case}"
            );
            assert_eq!(store.child_facts(&codex(CHILD)).unwrap().len(), 1, "{case}");
        } else {
            assert_eq!(
                (status, summary.candidates_codex),
                (xt_store::claude_launch::GroupStatus::Invalid, 0),
                "{case}: {summary:?}"
            );
        }
        drop(store);
        fs::remove_file(&home.db).unwrap();
    }
}

mod claude {
    //! Claude sessions: the first eligible input of each one's own
    //! transcript, the Codex launches that could name it, and the Claude
    //! `Bash` calls that could have launched it.
    use super::{T, iso, write_rows};
    use serde_json::{Value, json};
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    use xt_ingest::native::{
        ImportRequest, ProducerSource,
        claude_launch::{
            LaunchLimits, SearchGaps,
            bash::continue_claude_bash,
            check::{CheckProgress, continue_checks_into},
            continue_claude_launches,
        },
        readers_cli::read_pin,
        session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
    };
    use xt_store::{
        Host, Store,
        child_check::{ChildCheck, LAUNCH_VALIDATION_VERSION},
        claude_launch::{GroupStatus, MemberRecord},
        session_list,
    };

    const CALLER: &str = "0c000000-0000-4000-8000-0000000000ca";
    const ALPHA: &str = "0c000000-0000-4000-8000-0000000000a1";
    const BETA: &str = "0c000000-0000-4000-8000-0000000000b1";
    const NEIGHBOUR: &str = "0c000000-0000-4000-8000-0000000000e2";
    const MAIN: &str = "0c000000-0000-4000-8000-0000000000e3";
    const QUESTION: &str = "Synthetic install-rule question?";
    const CALL: &str = "toolu_01SyntheticLoop";

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn bundle() -> ProducerSource {
        ProducerSource::Bundle {
            pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
            root: repo().join("vendor/agent-plugins"),
        }
    }

    struct Home {
        _temp: tempfile::TempDir,
        home: PathBuf,
        db: PathBuf,
    }

    impl Home {
        fn new() -> Self {
            let temp = tempfile::TempDir::new().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let home = root.join("home");
            fs::create_dir_all(home.join(".claude/projects")).unwrap();
            fs::create_dir_all(home.join(".codex/sessions")).unwrap();
            Self {
                db: root.join("index.sqlite"),
                _temp: temp,
                home,
            }
        }

        fn store(&self) -> Store {
            Store::open(&self.db).unwrap()
        }

        fn project(&self, folder: &str) -> PathBuf {
            let path = self.home.join(".claude/projects").join(folder);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn states(&self, sessions: &[&str]) -> Vec<ChildCheck> {
            let connection = rusqlite::Connection::open(&self.db).unwrap();
            let context = session_list::context(&connection, sessions).unwrap();
            sessions
                .iter()
                .map(|id| {
                    context
                        .iter()
                        .find(|row| row.id == *id)
                        .map_or(ChildCheck::Checking, |row| row.check)
                })
                .collect()
        }
    }

    fn command() -> String {
        format!(
            "Q=\"{QUESTION}\"; for d in /w/alpha /w/beta; do echo \"== $d\"; \
             (cd \"$d\" && claude -p \"$Q\" --max-turns 1 2>&1 | tail -4); done"
        )
    }

    fn printed(alpha: &str, beta: &str) -> String {
        format!("== /w/alpha\n{alpha}\n== /w/beta\n{beta}\n")
    }

    /// A caller transcript: a person's request, the Bash call, and its own
    /// complete, clean result when `result` is given.
    fn caller_rows(result: Option<(i64, &str)>) -> Vec<Value> {
        let line = |kind: &str, uuid: &str, offset: i64, message: Value| {
            json!({"type": kind, "uuid": uuid, "parentUuid": null, "timestamp": iso(offset),
                "sessionId": CALLER, "isSidechain": false, "cwd": "/w/caller",
                "entrypoint": "cli", "message": message})
        };
        let mut rows = vec![
            line(
                "user",
                &format!("{CALLER}-ask"),
                -60_000,
                json!({"role": "user", "content": "Ask two fresh sessions"}),
            ),
            line(
                "assistant",
                &format!("{CALLER}-call"),
                0,
                json!({"role": "assistant", "id": format!("msg-{CALLER}"),
                    "model": "fixture-model",
                    "content": [{"type": "tool_use", "id": CALL, "name": "Bash",
                        "input": {"command": command(), "description": "Ask fresh sessions"}}],
                    "usage": {"input_tokens": 3, "output_tokens": 2}}),
            ),
        ];
        if let Some((offset, stdout)) = result {
            let mut row = line(
                "user",
                &format!("{CALLER}-result"),
                offset,
                json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": CALL, "content": stdout}]}),
            );
            row["toolUseResult"] =
                json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false});
            rows.push(row);
        }
        rows
    }

    fn session_rows(
        native: &str,
        cwd: &str,
        born: i64,
        question: &str,
        answer: &str,
    ) -> Vec<Value> {
        vec![
            json!({"type": "user", "uuid": format!("{native}-first"), "parentUuid": null,
                "timestamp": iso(born), "sessionId": native, "cwd": cwd,
                "entrypoint": "sdk-cli", "promptSource": "sdk",
                "message": {"role": "user", "content": question}}),
            json!({"type": "assistant", "uuid": format!("{native}-answer"),
                "parentUuid": format!("{native}-first"), "timestamp": iso(born + 2000),
                "sessionId": native, "cwd": cwd, "entrypoint": "sdk-cli",
                "message": {"role": "assistant", "id": format!("msg-{native}"),
                    "model": "fixture-model", "content": [{"type": "text", "text": answer}],
                    "usage": {"input_tokens": 3, "output_tokens": 2}}}),
        ]
    }

    fn write_session(home: &Home, folder: &str, native: &str, rows: &[Value]) -> PathBuf {
        let path = home.project(folder).join(format!("{native}.jsonl"));
        write_rows(&path, rows);
        path
    }

    fn scan(home: &Home, store: &mut Store, spawns: &mut SpawnBacklog) {
        let report = xt_ingest::native::scan_native_continued(
            store,
            &ImportRequest {
                home: &home.home,
                hosts: &[Host::Claude],
                producer: &bundle(),
                python: None,
                observed_at: T,
                cancel: None,
            },
            xt_ingest::native::ScanMode::Resume,
            &mut |_| {},
            spawns,
            spawn_limits(),
        );
        assert!(report.complete(), "{report:?}");
    }

    fn drain(
        home: &Home,
        store: &mut Store,
        spawns: &mut SpawnBacklog,
        watch: &[&str],
    ) -> Vec<Vec<ChildCheck>> {
        let mut frames = vec![home.states(watch)];
        for _ in 0..400 {
            if !spawns.pending() {
                return frames;
            }
            continue_codex_spawns(store, &home.home, spawns, spawn_limits(), None, T).unwrap();
            frames.push(home.states(watch));
        }
        panic!("the worker never went quiet");
    }

    /// The two sessions a caller's `Bash` loop launched are checking, then
    /// children, in every snapshot; the caller itself is a checked main.
    #[test]
    fn bash_launched_children_are_never_shown_as_main_sessions() {
        let home = Home::new();
        write_session(
            &home,
            "-w-caller",
            CALLER,
            &caller_rows(Some((12_000, &printed("Answer alpha", "Answer beta")))),
        );
        write_session(
            &home,
            "-w-alpha",
            ALPHA,
            &session_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
        );
        write_session(
            &home,
            "-w-beta",
            BETA,
            &session_rows(BETA, "/w/beta", 8_000, QUESTION, "Answer beta"),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let frames = drain(&home, &mut store, &mut spawns, &[ALPHA, BETA, CALLER]);
        assert!(
            frames
                .iter()
                .all(|frame| frame[0] != ChildCheck::Checked && frame[1] != ChildCheck::Checked),
            "{frames:?}"
        );
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Child, ChildCheck::Child, ChildCheck::Checked]
        );
    }

    #[test]
    fn escaped_launches_repair_previous_bash_negative_checks_without_source_changes() {
        let home = Home::new();
        let caller = write_session(
            &home,
            "-w-caller",
            CALLER,
            &caller_rows(Some((12_000, &printed("Answer alpha", "Answer beta")))),
        );
        let raw = fs::read_to_string(&caller)
            .unwrap()
            .replace("tool_use", "\\u0074ool_use");
        fs::write(&caller, &raw).unwrap();
        write_session(
            &home,
            "-w-alpha",
            ALPHA,
            &session_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
        );
        write_session(
            &home,
            "-w-beta",
            BETA,
            &session_rows(BETA, "/w/beta", 8_000, QUESTION, "Answer beta"),
        );
        let mut store = home.store();
        // Import only, then seed the negative checks the old raw-text detector
        // could finish against exactly these unchanged source bytes.
        scan(&home, &mut store, &mut SpawnBacklog::default());
        let connection = rusqlite::Connection::open(&home.db).unwrap();
        connection.execute("UPDATE session_child_checks SET completed_generation=required_generation, detector_versions='check1.spawn2.scan11.claude6.cli1.bash2'", []).unwrap();
        assert_eq!(
            home.states(&[ALPHA, BETA]),
            [ChildCheck::Checking, ChildCheck::Checking]
        );
        let frames = drain(
            &home,
            &mut store,
            &mut SpawnBacklog::starting(),
            &[ALPHA, BETA, CALLER],
        );
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Child, ChildCheck::Child, ChildCheck::Checked]
        );
        assert_eq!(fs::read_to_string(&caller).unwrap(), raw);
        let count = store.child_fact_summary().unwrap().children;
        drain(
            &home,
            &mut store,
            &mut SpawnBacklog::starting(),
            &[ALPHA, BETA, CALLER],
        );
        assert_eq!(store.child_fact_summary().unwrap().children, count);
    }

    /// A `Bash` launch with no result yet holds the session whose first
    /// input is its prompt, in its folder, born after it; a session with
    /// another first input is checked. The result arrives and the held one
    /// is a child.
    #[test]
    fn an_open_bash_call_holds_only_the_session_it_could_have_launched() {
        let home = Home::new();
        let caller = write_session(&home, "-w-caller", CALLER, &caller_rows(None));
        write_session(
            &home,
            "-w-alpha",
            ALPHA,
            &session_rows(ALPHA, "/w/alpha", 3_000, QUESTION, "Answer alpha"),
        );
        write_session(
            &home,
            "-w-alpha",
            NEIGHBOUR,
            &session_rows(
                NEIGHBOUR,
                "/w/alpha",
                4_000,
                "Something else entirely",
                "Fine",
            ),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let frames = drain(&home, &mut store, &mut spawns, &[ALPHA, NEIGHBOUR]);
        assert!(
            frames.iter().all(|frame| frame[0] == ChildCheck::Checking),
            "{frames:?}"
        );
        assert_eq!(frames.last().unwrap()[1], ChildCheck::Checked);
        // The call's result is saved: one launch, one possible child.
        write_rows(
            &caller,
            &caller_rows(Some((12_000, &printed("Answer alpha", "Answer beta")))),
        );
        scan(&home, &mut store, &mut spawns);
        let frames = drain(&home, &mut store, &mut spawns, &[ALPHA, NEIGHBOUR]);
        assert!(
            frames.iter().all(|frame| frame[0] != ChildCheck::Checked),
            "{frames:?}"
        );
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Child, ChildCheck::Checked]
        );
    }

    /// A transcript whose first input lies past many rows that are not one.
    fn long_opening(native: &str, rows: usize, question: &str) -> Vec<Value> {
        let mut lines: Vec<Value> = (0..rows)
            .map(|n| {
                json!({"type": "queue-operation", "operation": "enqueue",
                    "timestamp": iso(-10_000 + n as i64), "sessionId": native,
                    "content": "x".repeat(1000)})
            })
            .collect();
        lines.extend(session_rows(native, "/w/main", 3_000, question, "Answer"));
        lines
    }

    /// The worker's pass order, with display checks given `bytes` a pass.
    fn tight_pass(
        home: &Home,
        store: &mut Store,
        spawns: &mut SpawnBacklog,
        bytes: u64,
    ) -> CheckProgress {
        continue_claude_launches(
            store,
            &home.home,
            &mut spawns.launches,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
        continue_claude_bash(
            store,
            &home.home,
            &mut spawns.bash,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
        let mut progress = CheckProgress::default();
        continue_checks_into(
            store,
            &home.home,
            &mut spawns.checks,
            &mut spawns.launches,
            &mut spawns.bash,
            &LaunchLimits::remaining(bytes, std::time::Duration::from_secs(5)),
            None,
            &mut progress,
        )
        .unwrap();
        progress
    }

    fn key(store: &Store, session: &str) -> Option<String> {
        store
            .child_check_attempt(session)
            .unwrap()
            .unwrap()
            .own_check_key
    }

    const HISTORY: &str = "01a40000-0000-7000-8000-0000000000aa";

    /// A Codex history file of a thread nothing indexed, holding `body`.
    fn codex_history(home: &Home, body: &[u8]) -> PathBuf {
        let day = home.home.join(".codex/sessions/2026/10/01");
        fs::create_dir_all(&day).unwrap();
        let path = day.join(format!("rollout-2026-10-01T09-00-00-{HISTORY}.jsonl"));
        fs::write(&path, body).unwrap();
        path
    }

    /// Its own header, and one complete row.
    fn codex_rows() -> String {
        let header = json!({"timestamp": iso(-90_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": HISTORY, "session_id": HISTORY, "source": "vscode",
                "history_mode": "paginated"}});
        let row = json!({"timestamp": iso(-80_000), "type": "response_item", "ordinal": 1,
            "payload": {"type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": "Working"}]}});
        format!("{header}\n{row}\n")
    }

    /// Set a file's modification time `seconds` ago.
    fn age(path: &Path, seconds: u64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(seconds))
            .unwrap();
    }

    /// One Claude session beside one Codex history, through the scan and
    /// the worker's passes: its display state, and the skipped histories.
    fn main_beside(home: &Home) -> (Store, SpawnBacklog) {
        write_session(
            home,
            "-w-main",
            MAIN,
            &session_rows(MAIN, "/w/main", 3_000, "Hello", "Hi"),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(home, &mut store, &mut spawns);
        (store, spawns)
    }

    /// A Codex history that cannot be read — a row that is not JSON, a
    /// header naming another thread, or a file this user may not read — is
    /// skipped and counted, and holds no Claude session; an empty one is
    /// read with nothing found, and is not counted.
    #[test]
    fn a_codex_history_that_cannot_be_read_holds_no_claude_session() {
        use std::os::unix::fs::PermissionsExt;
        for case in ["not json", "another thread's header", "unreadable", "empty"] {
            let home = Home::new();
            let body = match case {
                "not json" => format!("{}{{\"not\": \"a row\"\n", codex_rows()),
                "another thread's header" => codex_rows().replacen(
                    &format!("\"id\":\"{HISTORY}\""),
                    "\"id\":\"01a40000-0000-7000-8000-0000000000bb\"",
                    1,
                ),
                "empty" => String::new(),
                _ => codex_rows(),
            };
            let path = codex_history(&home, body.as_bytes());
            if case == "unreadable" {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            }
            // A user who may read every file (root) reads this one too.
            let unreadable = case == "unreadable" && fs::File::open(&path).is_err();
            let (mut store, mut spawns) = main_beside(&home);
            let frames = drain(&home, &mut store, &mut spawns, &[MAIN]);
            assert!(frames.len() < 40, "{case}: {} passes", frames.len());
            assert_eq!(frames.last().unwrap(), &[ChildCheck::Checked], "{case}");
            let skipped = usize::from(case != "empty" && (case != "unreadable" || unreadable));
            assert_eq!(
                spawns.launches.search_gaps().map(|gaps| gaps.unsearched),
                Some(skipped),
                "{case}"
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    /// A Codex history whose last line has no newline holds Claude sessions
    /// while it is still changing, whether that line is its only one or not.
    /// Once unmodified for the quiet period, its complete lines are read
    /// without that line, the session is checked and the history is counted
    /// as searched without its unfinished line.
    #[test]
    fn an_unfinished_codex_line_holds_claude_sessions_only_while_it_changes() {
        for case in ["after a complete row", "as its only line"] {
            let home = Home::new();
            let torn = "{\"timestamp\": \"2026-10-01T09:00:00.000Z\", \"type\": \"respon";
            let body = match case {
                "after a complete row" => format!("{}{torn}", codex_rows()),
                _ => codex_rows().lines().next().unwrap().to_owned(),
            };
            let path = codex_history(&home, body.as_bytes());
            // Five seconds short of the five-minute quiet period
            // (`UNFINISHED_QUIET`): still changing.
            age(&path, 295);
            let (mut store, mut spawns) = main_beside(&home);
            for _ in 0..5 {
                continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
                    .unwrap();
            }
            assert_eq!(home.states(&[MAIN]), [ChildCheck::Checking], "{case}");
            assert!(spawns.pending(), "{case}: the hold is retried");
            if case == "after a complete row" {
                // Time alone ends the hold: the file is not touched again.
                std::thread::sleep(std::time::Duration::from_secs(6));
            } else {
                // Unmodified for an hour.
                age(&path, 3600);
            }
            let frames = drain(&home, &mut store, &mut spawns, &[MAIN]);
            assert!(frames.len() < 40, "{case}: {} passes", frames.len());
            assert_eq!(frames.last().unwrap(), &[ChildCheck::Checked], "{case}");
            assert_eq!(
                spawns.launches.search_gaps(),
                Some(SearchGaps {
                    unsearched: 0,
                    forks_not_searched: 0,
                    unfinished_line_skipped: 1,
                    ..SearchGaps::default()
                }),
                "{case}"
            );
            // The line completed later is read again, whole.
            fs::write(&path, codex_rows()).unwrap();
            spawns.launches.add_threads([HISTORY]);
            drain(&home, &mut store, &mut spawns, &[MAIN]);
            assert_eq!(spawns.launches.search_gaps(), Some(SearchGaps::default()));
        }
    }

    /// What an earlier version left on the coworker's Mac, with no version
    /// change since: the real `/review` subagent's history published invalid
    /// (its header refused), its own check observed with no key, and every
    /// Claude session held. The next run reads the invalid history again (an
    /// invalid validation is current only while the run that made it knows
    /// why), the Claude session is checked, and the review thread's own
    /// header gets its key and is checked.
    #[test]
    fn a_review_history_an_earlier_run_refused_is_read_again_after_a_restart() {
        use std::os::unix::fs::MetadataExt;
        const REVIEW: &str = "01a50000-0000-7000-8000-0000000000aa";
        const REVIEWED: &str = "01a50000-0000-7000-8000-0000000000bb";
        let home = Home::new();
        let day = home.home.join(".codex/sessions/2026/09/16");
        fs::create_dir_all(&day).unwrap();
        let path = day.join(format!("rollout-2026-09-16T19-22-54-{REVIEW}.jsonl"));
        let mut rows = vec![
            json!({"timestamp": iso(-60_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": REVIEW, "session_id": REVIEWED, "parent_thread_id": REVIEWED,
                "timestamp": iso(-60_000), "cwd": "/w", "originator": "codex_exec",
                "cli_version": "0.146.0", "source": {"subagent": "review"},
                "thread_source": "subagent", "history_mode": "paginated",
                "multi_agent_version": "disabled", "subagent_history_start_ordinal": 3}}),
        ];
        for ordinal in 1..3 {
            rows.push(
                json!({"timestamp": iso(-59_000 + ordinal), "type": "response_item",
                "ordinal": ordinal, "payload": {"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "Reviewing"}]}}),
            );
        }
        write_rows(&path, &rows);
        write_session(
            &home,
            "-w-main",
            MAIN,
            &session_rows(MAIN, "/w/main", 3_000, "Hello", "Hi"),
        );
        let mut store = home.store();
        super::index(
            &mut store,
            REVIEW,
            &path,
            ("5555cccc-5555-4555-8555-000000000000", -58_000),
            1,
        );
        scan(&home, &mut store, &mut SpawnBacklog::starting());
        // The earlier run's results, as the store kept them.
        let meta = fs::metadata(&path).unwrap();
        let generation = xt_store::claude_launch::SegmentGeneration {
            device: meta.dev() as i64,
            inode: meta.ino() as i64,
            length: meta.len() as i64,
            mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
            ctime_ns: meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
        };
        let allocation = store
            .begin_claude_launch_validation(REVIEW, LAUNCH_VALIDATION_VERSION)
            .unwrap();
        assert!(
            store
                .publish_claude_launch_validation(
                    REVIEW,
                    allocation,
                    GroupStatus::Invalid,
                    &[MemberRecord {
                        rollout_id: REVIEW.into(),
                        generation,
                        scanned_length: 0,
                        valid: false,
                    }],
                    0,
                )
                .unwrap()
        );
        let review = format!("codex-{REVIEW}");
        store.observe_own_check(&review, None).unwrap();
        assert_eq!(
            home.states(&[MAIN, &review]),
            [ChildCheck::Checking, ChildCheck::Checking]
        );
        // The next run.
        let mut spawns = SpawnBacklog::starting();
        let frames = drain(&home, &mut store, &mut spawns, &[MAIN, &review]);
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Checked, ChildCheck::Checked]
        );
        assert_eq!(
            store.claude_launch_group(REVIEW).unwrap().unwrap().status,
            GroupStatus::Valid
        );
        assert!(
            store
                .child_check_attempt(&review)
                .unwrap()
                .unwrap()
                .own_check_key
                .is_some()
        );
    }

    /// A first input past more than one pass's bytes is read over bounded
    /// passes, each resuming where the last stopped: every byte before it
    /// is read about once, and the session is checked.
    #[test]
    fn a_long_opening_is_read_over_bounded_passes_without_starting_again() {
        let home = Home::new();
        let path = write_session(&home, "-w-main", MAIN, &long_opening(MAIN, 400, "Hello"));
        let size = fs::metadata(&path).unwrap().len();
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let (mut bytes, mut reads, mut passes) = (0, 0, 0);
        while home.states(&[MAIN])[0] != ChildCheck::Checked {
            let progress = tight_pass(&home, &mut store, &mut spawns, 64 << 10);
            bytes += progress.bytes_read;
            reads += progress.own_reads;
            passes += 1;
            assert!(passes < 100, "never checked");
        }
        assert!(reads > 3, "{reads} reads");
        assert!(bytes <= size + (256 << 10), "{bytes} bytes of {size}");
    }

    /// The transcript replaced while its read is paused: the paused read
    /// cannot settle the session, which is checked from the new file.
    #[test]
    fn a_transcript_replaced_during_a_paused_read_is_read_again() {
        // What the original file's own-check key is, read without a pause.
        let control = Home::new();
        write_session(&control, "-w-main", MAIN, &long_opening(MAIN, 400, "Hello"));
        let mut store = control.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&control, &mut store, &mut spawns);
        drain(&control, &mut store, &mut spawns, &[MAIN]);
        let original = key(&store, MAIN);
        assert!(original.is_some());

        let home = Home::new();
        let path = write_session(&home, "-w-main", MAIN, &long_opening(MAIN, 400, "Hello"));
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        // Passes until the read has begun and paused.
        let mut begun = false;
        for _ in 0..20 {
            if tight_pass(&home, &mut store, &mut spawns, 64 << 10).own_reads > 0 {
                begun = true;
                break;
            }
        }
        assert!(begun);
        assert_eq!(home.states(&[MAIN])[0], ChildCheck::Checking);
        // Replaced by another file: another first input.
        let replacement = path.with_extension("tmp");
        write_rows(&replacement, &long_opening(MAIN, 400, "Goodbye"));
        fs::rename(&replacement, &path).unwrap();
        let mut passes = 0;
        while home.states(&[MAIN])[0] != ChildCheck::Checked {
            tight_pass(&home, &mut store, &mut spawns, 64 << 10);
            passes += 1;
            assert!(passes < 100, "never checked");
        }
        let settled = key(&store, MAIN);
        assert!(settled.is_some());
        assert_ne!(
            settled, original,
            "settled from the paused read of the old file"
        );
    }

    /// A previously checked Claude session whose transcript disappears is
    /// withdrawn at restart. An ordinary appended neighbor stays checked.
    #[test]
    fn a_checked_missing_transcript_reopens_quietly_and_recovers_on_scan() {
        let home = Home::new();
        let path = write_session(
            &home,
            "-w-main",
            MAIN,
            &session_rows(MAIN, "/w/main", 3_000, "Hello", "Hi"),
        );
        let neighbor = write_session(
            &home,
            "-w-neighbor",
            NEIGHBOUR,
            &session_rows(NEIGHBOUR, "/w/neighbor", 4_000, "Hello", "Hi"),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        let frames = drain(&home, &mut store, &mut spawns, &[MAIN, NEIGHBOUR]);
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Checked, ChildCheck::Checked]
        );
        let saved = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        use std::io::Write;
        writeln!(
            fs::OpenOptions::new().append(true).open(&neighbor).unwrap(),
            "{}",
            json!({"type":"assistant","uuid":format!("{NEIGHBOUR}-later"),
                "parentUuid":format!("{NEIGHBOUR}-answer"),"timestamp":iso(9_000),
                "sessionId":NEIGHBOUR,"cwd":"/w/neighbor","entrypoint":"sdk-cli",
                "message":{"role":"assistant","content":"Later work"}})
        )
        .unwrap();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        assert_eq!(
            home.states(&[MAIN, NEIGHBOUR]),
            [ChildCheck::Checking, ChildCheck::Checked]
        );
        let frames = drain(&home, &mut store, &mut spawns, &[MAIN, NEIGHBOUR]);
        assert!(frames.len() < 40, "{} passes", frames.len());
        assert!(
            frames
                .iter()
                .all(|frame| frame == &[ChildCheck::Checking, ChildCheck::Checked])
        );
        for _ in 0..3 {
            let progress =
                continue_codex_spawns(&mut store, &home.home, &mut spawns, spawn_limits(), None, T)
                    .unwrap();
            assert!(!progress.advanced(), "{progress:?}");
        }
        fs::write(&path, saved).unwrap();
        scan(&home, &mut store, &mut spawns);
        let frames = drain(&home, &mut store, &mut spawns, &[MAIN, NEIGHBOUR]);
        assert_eq!(
            frames.last().unwrap(),
            &[ChildCheck::Checked, ChildCheck::Checked]
        );
    }

    #[test]
    fn a_transcript_removed_after_enumeration_withdraws_its_old_completion() {
        let home = Home::new();
        let path = write_session(
            &home,
            "-w-main",
            MAIN,
            &session_rows(MAIN, "/w/main", 3_000, "Hello", "Hi"),
        );
        let mut store = home.store();
        let mut spawns = SpawnBacklog::starting();
        scan(&home, &mut store, &mut spawns);
        drain(&home, &mut store, &mut spawns, &[MAIN]);
        assert_eq!(home.states(&[MAIN]), [ChildCheck::Checked]);
        let (files, _) =
            xt_ingest::native::claude_fs::enumerate(&home.home.join(".claude/projects")).unwrap();
        let file = files
            .into_iter()
            .find(|file| file.session_id == MAIN)
            .unwrap();
        fs::remove_file(&path).unwrap();
        assert!(
            xt_ingest::native::claude_fs::import_file(
                &mut store,
                &file,
                T,
                xt_ingest::native::ScanMode::Resume,
            )
            .is_err()
        );
        assert_eq!(home.states(&[MAIN]), [ChildCheck::Checking]);
    }
}

#[test]
fn previous_bash_detector_checks_again_once() {
    let home = Home::new();
    let main_path = write_thread(&home, MAIN, header(MAIN, "cli", 10_000, &[]), 11_000);
    let mut store = home.store();
    index_thread(&mut store, MAIN, &main_path, 11_000);
    drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked);
    rusqlite::Connection::open(&home.db)
        .unwrap()
        .execute(
            "UPDATE session_child_checks SET detector_versions='check1.spawn2.scan11.claude6.cli1.bash2'",
            [],
        )
        .unwrap();
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checking);
    let (passes, _) = drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(home.state(&codex(MAIN)), ChildCheck::Checked);
    assert_eq!(changed(&passes), (0, 1));
    let (again, _) = drain(
        &home,
        &mut store,
        &mut SpawnBacklog::starting(),
        spawn_limits(),
        &[&codex(MAIN)],
    );
    assert_eq!(changed(&again), (0, 0));
}
