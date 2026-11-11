//! Parent links for Claude sessions a Claude agent launched with `claude -p`
//! from its own `Bash` tool, matched by their exact prompt: a launch in the
//! parent's own transcript run in the background with its output redirected
//! to files through a shell variable, and a launch from one of the parent's
//! helper agents through the absolute launcher path. Each child is the one
//! indexed session whose first input is that literal prompt, saved after the
//! call; it folds under its parent like a Codex-launched child.
//!
//! Every file is written here: synthetic parent, helper and child
//! transcripts. Nothing real is read and no command runs.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    ImportRequest, ProducerSource,
    claude_launch::{
        LaunchBacklog, LaunchLimits,
        bash::{BashBacklog, BashProgress, PromptProgress, continue_claude_bash},
        continue_claude_launches,
    },
    readers_cli::read_pin,
    session_creation::{SpawnBacklog, spawn_limits},
};
use xt_store::{
    Host, Store,
    child_check::{ChildCheck, DETECTOR_VERSIONS},
    child_fact::ChildEvidence,
    creation::{CreationEvidence, CreationWitness},
    session_list,
};

const PARENT: &str = "0d000000-0000-4000-8000-0000000000ca";
const OTHER_PARENT: &str = "0d000000-0000-4000-8000-0000000000cb";
const MAIN_CHILD: &str = "0d000000-0000-4000-8000-0000000000a1";
const HELPER_CHILD: &str = "0d000000-0000-4000-8000-0000000000b1";
const EXTRA: &str = "0d000000-0000-4000-8000-0000000000e1";
const NAMED: &str = "0d000000-0000-4000-8000-0000000000f1";
const PROMPT: &str =
    "In my synthetic project called 'Check copy': fix the spelling. Tell me what changed.";
const HELPER_PROMPT: &str = "Synthetic helper prompt: list the projects.";
const MAIN_CALL: &str = "toolu_01SyntheticMainLaunch";
const HELPER_CALL: &str = "toolu_01SyntheticHelperLaunch";
const AGENT: &str = "a0d00000000000001";
/// The main launch's time; everything else is an offset from it.
const AT: i64 = 1_790_668_716_333;
const T: i64 = 1_790_700_000_000;

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

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
        // The installed launcher a helper names by its absolute path.
        let launcher = home.join(".local/bin/claude");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        fs::write(&launcher, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
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

    fn launcher(&self) -> String {
        self.home.join(".local/bin/claude").display().to_string()
    }
}

fn write_rows(path: &Path, rows: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
    fs::write(path, body).unwrap();
}

/// The parent's launch: a variable, background output files and every
/// neutral option of a real agent's test run, its output shown by a script,
/// and a note naming `claude` given to a program that never runs it.
fn main_command(prompt: &str) -> String {
    format!(
        "S=/w/run && cd \"$S\" && claude -p \"{prompt}\" --name \"synthetic check\" \
         --mcp-config \"$S/mcp.json\" --strict-mcp-config --allowedTools \"mcp__synthetic__*\" \
         --setting-sources project --output-format stream-json --verbose \
         > \"$S/run1.jsonl\" 2> \"$S/run1.err\"; echo \"exit $?\"; \
         python3 \"$S/show.py\" \"$S/run1.jsonl\" 260 | cut -c1-320; cat <<'EOF'\n\
         claude -p \"not a launch\"\nEOF"
    )
}

/// A helper agent's launch: the absolute launcher and unquoted expansions.
fn helper_command(home: &Home, prompt: &str) -> String {
    format!(
        "S=/w; cd $S/run && {} -p \"{prompt}\" --name \"synthetic helper\" \
         --mcp-config $S/run/test.mcp.json --strict-mcp-config --setting-sources project \
         --output-format stream-json --verbose > run1.jsonl 2> run1.err; echo \"exit $?\"",
        home.launcher()
    )
}

fn record(
    native: &str,
    kind: &str,
    uuid: &str,
    offset: i64,
    side: Option<&str>,
    message: Value,
) -> Value {
    let mut row = json!({"type": kind, "uuid": uuid, "parentUuid": null, "timestamp": iso(offset),
        "sessionId": native, "isSidechain": side.is_some(), "cwd": "/w/parent",
        "entrypoint": "claude-desktop", "userType": "external", "message": message});
    if let Some(agent) = side {
        row["agentId"] = json!(agent);
    }
    row
}

/// A transcript holding one `Bash` call run in the background, its result
/// saying only that it runs.
fn caller_rows(
    native: &str,
    side: Option<&str>,
    call: &str,
    offset: i64,
    command: &str,
) -> Vec<Value> {
    let tag = side.unwrap_or("main");
    vec![
        record(
            native,
            "user",
            &format!("{native}-{tag}-ask"),
            offset - 60_000,
            side,
            json!({"role": "user", "content": "Run the synthetic check"}),
        ),
        record(
            native,
            "assistant",
            &format!("{native}-{tag}-{call}"),
            offset,
            side,
            json!({"role": "assistant", "id": format!("msg-{native}-{call}"),
                "model": "fixture-model",
                "content": [{"type": "tool_use", "id": call, "name": "Bash",
                    "input": {"command": command, "description": "Run the check",
                        "run_in_background": true}}],
                "usage": {"input_tokens": 3, "output_tokens": 2}}),
        ),
        record(
            native,
            "user",
            &format!("{native}-{tag}-{call}-result"),
            offset + 50,
            side,
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": call,
                "content": "Command running in background with ID: synthetic. Output is being written to a file."}]}),
        ),
    ]
}

/// A launched session: a title, a queue record, then its first input at
/// `born`, and one answer.
fn child_rows(native: &str, born: i64, first: &str) -> Vec<Value> {
    vec![
        json!({"type": "custom-title", "customTitle": "synthetic check", "sessionId": native}),
        json!({"type": "queue-operation", "operation": "enqueue", "timestamp": iso(born - 20),
            "sessionId": native, "content": first}),
        json!({"type": "user", "uuid": format!("{native}-first"), "parentUuid": null,
            "timestamp": iso(born), "sessionId": native, "cwd": "/w/run", "isSidechain": false,
            "entrypoint": "claude-desktop", "userType": "external",
            "message": {"role": "user", "content": first}}),
        json!({"type": "assistant", "uuid": format!("{native}-answer"),
            "parentUuid": format!("{native}-first"), "timestamp": iso(born + 2000),
            "sessionId": native, "cwd": "/w/run", "isSidechain": false,
            "entrypoint": "claude-desktop",
            "message": {"role": "assistant", "id": format!("msg-{native}"),
                "model": "fixture-model", "content": [{"type": "text", "text": "Done."}],
                "usage": {"input_tokens": 3, "output_tokens": 2}}}),
    ]
}

fn parent_path(home: &Home, native: &str) -> PathBuf {
    home.project("-w-parent").join(format!("{native}.jsonl"))
}

fn write_parent(home: &Home, native: &str, call: &str, offset: i64, command: &str) {
    write_rows(
        &parent_path(home, native),
        &caller_rows(native, None, call, offset, command),
    );
}

fn write_helper(home: &Home, native: &str, command: &str) {
    let path = home
        .project("-w-parent")
        .join(native)
        .join("subagents")
        .join(format!("agent-{AGENT}.jsonl"));
    write_rows(
        &path,
        &caller_rows(native, Some(AGENT), HELPER_CALL, 30_000, command),
    );
}

fn write_child(home: &Home, native: &str, born: i64, first: &str) {
    write_rows(
        &home.project("-w-run").join(format!("{native}.jsonl")),
        &child_rows(native, born, first),
    );
}

/// The parent's own launch and its helper's, and their two children.
fn standard(home: &Home) {
    write_parent(home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_helper(home, PARENT, &helper_command(home, HELPER_PROMPT));
    write_child(home, MAIN_CHILD, 4_800, PROMPT);
    write_child(home, HELPER_CHILD, 30_500, HELPER_PROMPT);
}

/// Index the Claude host through the ordinary importer.
fn scan(home: &Home, store: &mut Store) {
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
        &mut SpawnBacklog::default(),
        spawn_limits(),
    );
    assert!(report.complete(), "{report:?}");
}

/// One pass in the worker's order: the Codex launch work, then the `Bash`
/// work, at `now`.
fn pass_at(
    home: &Home,
    store: &mut Store,
    backlog: &mut BashBacklog,
    launches: &mut LaunchBacklog,
    now: i64,
) -> BashProgress {
    continue_claude_launches(
        store,
        &home.home,
        launches,
        &LaunchLimits::default(),
        None,
        now,
    )
    .unwrap();
    continue_claude_bash(
        store,
        &home.home,
        backlog,
        launches,
        &LaunchLimits::default(),
        None,
        now,
    )
    .unwrap()
}

fn add(total: &mut PromptProgress, p: &PromptProgress) {
    total.files_read += p.files_read;
    total.matches += p.matches;
    total.launches = p.launches.max(total.launches);
    total.linked += p.linked;
    total.already += p.already;
    total.ambiguous += p.ambiguous;
    total.refused += p.refused;
    total.other_input += p.other_input;
    total.held += p.held;
    total.held_hour += p.held_hour;
    total.held_caller_gone += p.held_caller_gone;
    total.held_command_unread += p.held_command_unread;
    total.held_untimed += p.held_untimed;
    total.held_child_unread += p.held_child_unread;
    total.held_codex += p.held_codex;
    total.commands_refused += p.commands_refused;
    total.changed += p.changed;
}

/// Bounded passes until nothing is pending; the literal-prompt counts summed.
fn settle(home: &Home, store: &mut Store, backlog: &mut BashBacklog) -> PromptProgress {
    let mut total = PromptProgress::default();
    let mut launches = LaunchBacklog::starting();
    for _ in 0..1_000 {
        if !backlog.pending() && !launches.pending() {
            return total;
        }
        let pass = pass_at(home, store, backlog, &mut launches, T);
        add(&mut total, &pass.prompt);
    }
    panic!("never settled: {total:?}");
}

/// Index, then the background passes a fresh worker runs.
fn run(home: &Home) -> (Store, PromptProgress) {
    let mut store = home.store();
    scan(home, &mut store);
    let total = settle(home, &mut store, &mut BashBacklog::starting());
    (store, total)
}

fn session(store: &Store, native: &str) -> String {
    store
        .user_sessions_with_native(Host::Claude, native)
        .unwrap()
        .remove(0)
}

/// The parent a list shows for one session, and whether it is a known child.
fn shown(home: &Home, store: &Store, native: &str) -> (bool, Option<String>) {
    let connection = rusqlite::Connection::open(&home.db).unwrap();
    let context = session_list::context(&connection, &[&session(store, native)])
        .unwrap()
        .remove(0);
    (
        context.known_child,
        context.parent.map(|parent| parent.session_id),
    )
}

/// `(kind, source, call, position, accepted)` of one stored fact.
type Stored = (ChildEvidence, String, Option<String>, Option<u32>, bool);

/// Every stored fact of one session, as [`Stored`].
fn facts(store: &Store, native: &str) -> Vec<Stored> {
    store
        .child_facts(&session(store, native))
        .unwrap()
        .into_iter()
        .map(|(fact, accepted)| {
            (
                fact.evidence_kind,
                fact.source_native_session_id,
                fact.launch_call_id,
                fact.launch_operation_index,
                accepted,
            )
        })
        .collect()
}

fn prompt_fact(source: &str, call: &str, accepted: bool) -> Stored {
    (
        ChildEvidence::ClaudePromptLaunch,
        source.to_owned(),
        Some(call.to_owned()),
        Some(0),
        accepted,
    )
}

/// Nothing links `native`: no fact, no relation, no parent shown.
fn unlinked(home: &Home, store: &Store, native: &str) {
    assert!(facts(store, native).is_empty(), "{native}");
    assert!(
        store
            .session_creation(&session(store, native))
            .unwrap()
            .is_none(),
        "{native}"
    );
    assert_eq!(shown(home, store, native), (false, None), "{native}");
}

#[test]
fn literal_prompt_launches_link_both_children_to_the_top_level_parent() {
    let home = Home::new();
    standard(&home);
    let (mut store, total) = run(&home);
    assert_eq!(total.launches, 2, "{total:?}");
    assert_eq!(
        (total.linked, total.ambiguous, total.refused),
        (2, 0, 0),
        "{total:?}"
    );
    let parent = session(&store, PARENT);
    for (child, call) in [(MAIN_CHILD, MAIN_CALL), (HELPER_CHILD, HELPER_CALL)] {
        // Shown under the parent, like a Codex-launched child.
        assert_eq!(
            shown(&home, &store, child),
            (true, Some(parent.clone())),
            "{child}"
        );
        assert_eq!(
            facts(&store, child),
            [prompt_fact(PARENT, call, true)],
            "{child}"
        );
        let (relation, conflicted) = store
            .session_creation(&session(&store, child))
            .unwrap()
            .unwrap();
        assert!(!conflicted);
        assert_eq!(relation.evidence_kind, CreationEvidence::ClaudePromptLaunch);
        assert_eq!(relation.witness, CreationWitness::ClaudeBashLiteralPrompt);
        assert_eq!(relation.parent_host, Host::Claude);
        assert_eq!(relation.parent_native_session_id, PARENT);
    }
    // The parent is no child.
    assert_eq!(shown(&home, &store, PARENT), (false, None));
    // A restart reads and matches again and changes nothing.
    let again = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!(
        (again.linked, again.changed, again.already),
        (0, 0, 2),
        "{again:?}"
    );
}

#[test]
fn a_child_whose_display_check_already_finished_is_still_linked() {
    // A session checked before this match existed (migration 32 keeps such
    // checks) stays shown on its own only until the match links it.
    let home = Home::new();
    standard(&home);
    let mut store = home.store();
    scan(&home, &mut store);
    let connection = rusqlite::Connection::open(&home.db).unwrap();
    let child = session(&store, MAIN_CHILD);
    connection
        .execute(
            "UPDATE session_child_checks SET completed_generation=required_generation,
                 detector_versions=?2 WHERE session_id=?1",
            [&child, DETECTOR_VERSIONS],
        )
        .unwrap();
    let check = || {
        session_list::context(&connection, &[&child])
            .unwrap()
            .remove(0)
            .check
    };
    assert_eq!(check(), ChildCheck::Checked);
    let total = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!(total.linked, 2, "{total:?}");
    assert_eq!(check(), ChildCheck::Child);
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_first_input_that_is_not_the_prompt_abstains() {
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    // One character more, and only whitespace different.
    write_child(&home, MAIN_CHILD, 4_800, &format!("{PROMPT}."));
    write_child(&home, EXTRA, 5_800, &format!("{PROMPT} "));
    let (store, total) = run(&home);
    assert_eq!((total.launches, total.linked), (1, 0), "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    unlinked(&home, &store, EXTRA);
}

#[test]
fn a_child_saved_before_the_launch_abstains() {
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, -1_000, PROMPT);
    let (store, total) = run(&home);
    assert_eq!((total.launches, total.linked), (1, 0), "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
}

/// Launches and their children, minutes apart: each launch at `(caller,
/// call, offset)` and each child at `(native, born)`, all of `PROMPT`.
fn launches_and_children(home: &Home, launches: &[(&str, &str, i64)], children: &[(&str, i64)]) {
    for (caller, call, offset) in launches {
        write_parent(home, caller, call, *offset, &main_command(PROMPT));
    }
    for (native, born) in children {
        write_child(home, native, *born, PROMPT);
    }
}

const MINUTE: i64 = 60_000;
const OTHER_CALL: &str = "toolu_01SyntheticOtherLaunch";

#[test]
fn a_retried_prompt_links_each_child_to_the_launch_just_before_it() {
    // A launch at 9:00 and its retry at 9:10, children at 9:00:05 and
    // 9:10:05: each child is its closest launch's, and only that one's.
    let home = Home::new();
    launches_and_children(
        &home,
        &[
            (PARENT, MAIN_CALL, 0),
            (OTHER_PARENT, OTHER_CALL, 10 * MINUTE),
        ],
        &[(MAIN_CHILD, 5_000), (EXTRA, 10 * MINUTE + 5_000)],
    );
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.ambiguous),
        (2, 2, 0),
        "{total:?}"
    );
    assert_eq!(
        facts(&store, MAIN_CHILD),
        [prompt_fact(PARENT, MAIN_CALL, true)]
    );
    assert_eq!(
        facts(&store, EXTRA),
        [prompt_fact(OTHER_PARENT, OTHER_CALL, true)]
    );
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
    assert_eq!(
        shown(&home, &store, EXTRA),
        (true, Some(session(&store, OTHER_PARENT)))
    );
    // Two launches before one child: the later one is its launch.
    let home = Home::new();
    launches_and_children(
        &home,
        &[(PARENT, MAIN_CALL, 0), (OTHER_PARENT, OTHER_CALL, 1_000)],
        &[(MAIN_CHILD, 4_800)],
    );
    let (store, total) = run(&home);
    assert_eq!((total.launches, total.linked), (2, 1), "{total:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, OTHER_PARENT)))
    );
}

#[test]
fn two_launches_at_the_same_moment_link_nothing() {
    let home = Home::new();
    launches_and_children(
        &home,
        &[(PARENT, MAIN_CALL, 0), (OTHER_PARENT, OTHER_CALL, 0)],
        &[(MAIN_CHILD, 4_800)],
    );
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.ambiguous),
        (2, 0, 2),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
}

#[test]
fn one_launch_with_two_children_in_its_hour_links_nothing() {
    // No second launch between them: either could be the launch's child.
    let home = Home::new();
    launches_and_children(
        &home,
        &[(PARENT, MAIN_CALL, 0)],
        &[(MAIN_CHILD, 4_800), (EXTRA, 40 * MINUTE)],
    );
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.ambiguous),
        (1, 0, 1),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
    unlinked(&home, &store, EXTRA);
}

#[test]
fn a_session_more_than_an_hour_after_the_launch_is_not_its_child() {
    // Alone two hours later: no link.
    let home = Home::new();
    launches_and_children(&home, &[(PARENT, MAIN_CALL, 0)], &[(EXTRA, 120 * MINUTE)]);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.ambiguous),
        (1, 0, 0),
        "{total:?}"
    );
    unlinked(&home, &store, EXTRA);
    // Beside the launch's own child, it does not stop that link; an hour
    // exactly is still inside.
    let home = Home::new();
    launches_and_children(
        &home,
        &[(PARENT, MAIN_CALL, 0)],
        &[(MAIN_CHILD, 60 * MINUTE), (EXTRA, 120 * MINUTE)],
    );
    let (store, total) = run(&home);
    assert_eq!((total.linked, total.ambiguous), (1, 0), "{total:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
    unlinked(&home, &store, EXTRA);
}

#[test]
fn a_named_session_launch_links_only_the_session_it_names() {
    // The launch names its session; another session typed the same prompt
    // later, which this launch could not have made.
    let home = Home::new();
    let command = format!("claude -p --session-id {NAMED} \"{PROMPT}\" > /w/out.json");
    write_parent(&home, PARENT, MAIN_CALL, 0, &command);
    write_child(&home, NAMED, 4_800, PROMPT);
    write_child(&home, EXTRA, 60_000, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(total.linked, 1, "{total:?}");
    assert_eq!(
        shown(&home, &store, NAMED),
        (true, Some(session(&store, PARENT)))
    );
    unlinked(&home, &store, EXTRA);
}

#[test]
fn resume_continue_and_fork_are_not_launches() {
    for command in [
        format!("claude -p --resume {NAMED} \"{PROMPT}\""),
        format!("claude -p -r {NAMED} \"{PROMPT}\""),
        format!("claude -c -p \"{PROMPT}\""),
        format!("claude --continue --print \"{PROMPT}\""),
        format!("claude -p --resume {NAMED} --fork-session \"{PROMPT}\""),
        // Not run: only text a here-document or `echo` holds.
        format!("cat <<'EOF'\nclaude -p \"{PROMPT}\"\nEOF"),
        format!("echo claude -p \"{PROMPT}\""),
        // The prompt is not literal.
        "P=x; claude -p \"$P\"".to_owned(),
    ] {
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &command);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (store, total) = run(&home);
        assert_eq!(
            (total.launches, total.linked),
            (0, 0),
            "{command}: {total:?}"
        );
        unlinked(&home, &store, MAIN_CHILD);
    }
}

#[test]
fn a_stored_link_is_never_withdrawn_or_moved() {
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (mut store, total) = run(&home);
    assert_eq!(total.linked, 1, "{total:?}");
    // Another session's launch of the same prompt, made after that launch
    // and before the child, is saved after the link was decided. The link
    // was decided once its hour was over, and stays as it is.
    write_parent(
        &home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherLaunch",
        1_000,
        &main_command(PROMPT),
    );
    scan(&home, &mut store);
    let again = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!((again.linked, again.changed), (0, 0), "{again:?}");
    assert_eq!(
        facts(&store, MAIN_CHILD),
        [prompt_fact(PARENT, MAIN_CALL, true)]
    );
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_record_of_another_session_in_a_transcript_is_not_its_launch() {
    // The helper file's records name another session: it is not the
    // parent's, so its launch is not read as the parent's.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, "echo ready");
    let path = home
        .project("-w-parent")
        .join(PARENT)
        .join("subagents")
        .join(format!("agent-{AGENT}.jsonl"));
    write_rows(
        &path,
        &caller_rows(
            OTHER_PARENT,
            Some(AGENT),
            HELPER_CALL,
            30_000,
            &helper_command(&home, HELPER_PROMPT),
        ),
    );
    write_child(&home, HELPER_CHILD, 30_500, HELPER_PROMPT);
    let (store, total) = run(&home);
    assert_eq!((total.launches, total.linked), (0, 0), "{total:?}");
    unlinked(&home, &store, HELPER_CHILD);
}

/// Index, change the sources with `then`, then the passes of a fresh worker;
/// the worker's backlog is kept for later passes.
fn run_after(home: &Home, then: impl FnOnce()) -> (Store, BashBacklog, PromptProgress) {
    let mut store = home.store();
    scan(home, &mut store);
    then();
    let mut backlog = BashBacklog::starting();
    let total = settle(home, &mut store, &mut backlog);
    (store, backlog, total)
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn helper_path(home: &Home, native: &str) -> PathBuf {
    home.project("-w-parent")
        .join(native)
        .join("subagents")
        .join(format!("agent-{AGENT}.jsonl"))
}

fn child_path(home: &Home, native: &str) -> PathBuf {
    home.project("-w-run").join(format!("{native}.jsonl"))
}

#[test]
fn a_launch_given_other_input_links_nothing_but_still_stops_a_link() {
    // Its child's first input may be more than the prompt.
    for command in [
        format!("git diff | claude -p \"{PROMPT}\" > /w/out.txt"),
        format!("claude -p \"{PROMPT}\" < /w/in.txt > /w/out.txt"),
        format!("claude -p \"{PROMPT}\" <<'EOF' > /w/out.txt\nmore input\nEOF"),
        format!("cat /w/in.txt | while read l; do claude -p \"{PROMPT}\"; done"),
    ] {
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &command);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (store, total) = run(&home);
        assert_eq!(
            (total.launches, total.linked),
            (1, 0),
            "{command}: {total:?}"
        );
        unlinked(&home, &store, MAIN_CHILD);
    }
    // Made after a launch that reads only its own input, it is the child's
    // closest launch: neither links.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_parent(
        &home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherLaunch",
        1_000,
        &format!("git diff | claude -p \"{PROMPT}\""),
    );
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.other_input),
        (2, 0, 1),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
}

#[test]
fn text_a_command_does_not_run_is_no_launch() {
    for command in [
        // A `)` in a here-document inside `$(…)` closes nothing.
        format!("x=$(cat <<'EOF'\n) ; claude -p \"{PROMPT}\" ; (\nEOF\n)"),
        format!("msg=$(cat <<'EOF'\nfoo) claude -p '{PROMPT}'\nEOF\n)"),
        // An array's words, and a function's body.
        format!("x=( claude -p \"{PROMPT}\" )"),
        format!("f() {{ claude -p \"{PROMPT}\"; }}"),
    ] {
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &command);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (store, total) = run(&home);
        assert_eq!(
            (total.launches, total.linked),
            (0, 0),
            "{command:?}: {total:?}"
        );
        unlinked(&home, &store, MAIN_CHILD);
    }
}

/// The parent's launch, and another caller whose helper agent launched the
/// same prompt before the one child.
fn rival_in_helper(home: &Home) {
    write_parent(home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_parent(
        home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherCall",
        -2_000,
        "echo ready",
    );
    write_rows(
        &helper_path(home, OTHER_PARENT),
        &caller_rows(
            OTHER_PARENT,
            Some(AGENT),
            HELPER_CALL,
            1_000,
            &helper_command(home, PROMPT),
        ),
    );
    write_child(home, MAIN_CHILD, 4_800, PROMPT);
}

#[test]
fn an_unreadable_caller_file_or_folder_links_nothing_until_it_is_read() {
    // The rival launch is in a helper file that cannot be read.
    let home = Home::new();
    rival_in_helper(&home);
    let helper = helper_path(&home, OTHER_PARENT);
    let (mut store, mut backlog, total) = run_after(&home, || set_mode(&helper, 0o000));
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    // Readable again and read after the next import: the rival, made
    // after the parent's launch, is the child's launch.
    set_mode(&helper, 0o644);
    backlog.add_sessions([PARENT]);
    let again = settle(&home, &mut store, &mut backlog);
    assert_eq!(again.linked, 1, "{again:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, OTHER_PARENT)))
    );

    // The same with the helper folder that cannot be listed.
    let home = Home::new();
    rival_in_helper(&home);
    let folder = helper_path(&home, OTHER_PARENT)
        .parent()
        .unwrap()
        .to_path_buf();
    let (store, _, total) = run_after(&home, || set_mode(&folder, 0o000));
    set_mode(&folder, 0o755);
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);

    // An unreadable helper file holding no launch holds the link back until
    // it is read.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_helper(&home, OTHER_PARENT, "echo nothing to launch with claude");
    write_parent(
        &home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherCall",
        -2_000,
        "echo ready",
    );
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let helper = helper_path(&home, OTHER_PARENT);
    let (mut store, mut backlog, total) = run_after(&home, || set_mode(&helper, 0o000));
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    set_mode(&helper, 0o644);
    backlog.add_sessions([PARENT]);
    let again = settle(&home, &mut store, &mut backlog);
    assert_eq!(again.linked, 1, "{again:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn an_unreadable_possible_child_links_nothing_until_it_is_read() {
    // A second session with the same first input cannot be read.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child(&home, EXTRA, 9_000, PROMPT);
    let extra = child_path(&home, EXTRA);
    let (mut store, mut backlog, total) = run_after(&home, || set_mode(&extra, 0o000));
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    set_mode(&extra, 0o644);
    backlog.add_sessions([PARENT]);
    let again = settle(&home, &mut store, &mut backlog);
    assert_eq!((again.linked, again.ambiguous), (0, 1), "{again:?}");
    unlinked(&home, &store, MAIN_CHILD);
    unlinked(&home, &store, EXTRA);

    // One whose first input is another holds the link back until it is read.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child(&home, EXTRA, 9_000, HELPER_PROMPT);
    let extra = child_path(&home, EXTRA);
    let (mut store, mut backlog, total) = run_after(&home, || set_mode(&extra, 0o000));
    assert_eq!(total.linked, 0, "{total:?}");
    set_mode(&extra, 0o644);
    backlog.add_sessions([EXTRA]);
    let again = settle(&home, &mut store, &mut backlog);
    assert_eq!(again.linked, 1, "{again:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
    unlinked(&home, &store, EXTRA);
}

#[test]
fn what_could_not_be_read_stops_only_the_links_it_could_change() {
    let linked = |home: &Home, store: &Store| {
        assert_eq!(
            shown(home, store, MAIN_CHILD),
            (true, Some(session(store, PARENT)))
        );
    };
    // A caller first recorded after the child was saved hid no launch of it.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_parent(
        &home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherCall",
        200_000,
        "echo ready",
    );
    write_rows(
        &helper_path(&home, OTHER_PARENT),
        &caller_rows(
            OTHER_PARENT,
            Some(AGENT),
            HELPER_CALL,
            210_000,
            "echo later",
        ),
    );
    let helper = helper_path(&home, OTHER_PARENT);
    let (store, _, total) = run_after(&home, || set_mode(&helper, 0o000));
    set_mode(&helper, 0o644);
    assert_eq!(total.linked, 1, "{total:?}");
    linked(&home, &store);

    // A session saved before the launch, unreadable, was not its child.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child(&home, EXTRA, -5_000, PROMPT);
    let extra = child_path(&home, EXTRA);
    let (store, _, total) = run_after(&home, || set_mode(&extra, 0o000));
    set_mode(&extra, 0o644);
    assert_eq!(total.linked, 1, "{total:?}");
    linked(&home, &store);

    // A caller whose transcript was deleted is a gap only over the time its
    // indexed records span: all before the launch, it stops nothing.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_parent(
        &home,
        OTHER_PARENT,
        "toolu_01SyntheticOtherCall",
        -2_000,
        "echo ready",
    );
    let (store, _, total) = run_after(&home, || {
        fs::remove_file(parent_path(&home, OTHER_PARENT)).unwrap();
    });
    assert_eq!(total.linked, 1, "{total:?}");
    linked(&home, &store);

    // A possible child whose transcript was deleted, saved after the
    // launch, may have been its child: nothing links.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child(&home, EXTRA, 9_000, PROMPT);
    let (store, _, total) = run_after(&home, || {
        fs::remove_file(child_path(&home, EXTRA)).unwrap();
    });
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
}

fn write_child_in(home: &Home, folder: &str, native: &str, born: i64, first: &str) {
    write_rows(
        &home.project(folder).join(format!("{native}.jsonl")),
        &child_rows(native, born, first),
    );
}

#[test]
fn a_possible_child_whose_project_folder_is_gone_stops_the_links_it_could_change() {
    // A second session of the prompt in the launch's hour, in a project
    // folder deleted since it was indexed: either could be the child.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child_in(&home, "-w-gone", EXTRA, 9_000, PROMPT);
    let (store, _, total) = run_after(&home, || {
        fs::remove_dir_all(home.project("-w-gone")).unwrap();
    });
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    // Saved before the launch, it could not be its child: the link is made.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    write_child_in(&home, "-w-gone", EXTRA, -9_000, PROMPT);
    let (store, _, total) = run_after(&home, || {
        fs::remove_dir_all(home.project("-w-gone")).unwrap();
    });
    assert_eq!(total.linked, 1, "{total:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_stored_link_stays_when_a_rival_caller_is_deleted() {
    // Two launches half an hour apart, each with its own child.
    let home = Home::new();
    launches_and_children(
        &home,
        &[
            (PARENT, MAIN_CALL, 0),
            (OTHER_PARENT, OTHER_CALL, 30 * MINUTE),
        ],
        &[(MAIN_CHILD, MINUTE), (EXTRA, 31 * MINUTE)],
    );
    let (mut store, total) = run(&home);
    assert_eq!(total.linked, 2, "{total:?}");
    // The rival's transcript is deleted, as Claude's clean-up does: after a
    // restart its launch is unknown, and the first launch could now seem to
    // keep both children. Neither stored link is withdrawn or moved.
    fs::remove_file(parent_path(&home, OTHER_PARENT)).unwrap();
    let again = settle(&home, &mut store, &mut BashBacklog::starting());
    assert_eq!((again.linked, again.changed), (0, 0), "{again:?}");
    assert_eq!(
        facts(&store, MAIN_CHILD),
        [prompt_fact(PARENT, MAIN_CALL, true)]
    );
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
    assert_eq!(
        shown(&home, &store, EXTRA),
        (true, Some(session(&store, OTHER_PARENT)))
    );
}

#[test]
fn a_deleted_rival_caller_blocks_a_child_not_linked_yet() {
    // The child's own launch is the rival's, whose transcript (begun before
    // the first launch) is deleted before any match: the earlier launch
    // must not take its child.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    let mut rival = caller_rows(
        OTHER_PARENT,
        None,
        "toolu_01SyntheticOtherCall",
        -10 * MINUTE,
        "echo ready",
    );
    rival.extend(caller_rows(
        OTHER_PARENT,
        None,
        OTHER_CALL,
        30 * MINUTE,
        &main_command(PROMPT),
    ));
    write_rows(&parent_path(&home, OTHER_PARENT), &rival);
    write_child(&home, EXTRA, 31 * MINUTE, PROMPT);
    let (store, _, total) = run_after(&home, || {
        fs::remove_file(parent_path(&home, OTHER_PARENT)).unwrap();
    });
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, EXTRA);
}

#[test]
fn a_command_naming_claude_that_cannot_be_read_stops_the_links_it_could_change() {
    for rival in [
        format!("sudo claude -p \"{PROMPT}\" > /w/out.txt"),
        format!("CLAUDE=claude; $CLAUDE -p \"{PROMPT}\""),
        format!("bash -euc 'claude -p \"{PROMPT}\"'"),
        format!("printf '%s' \"{PROMPT}\" | xargs -0 claude -p"),
        format!("f() {{ claude -p \"$1\"; }}; f \"{PROMPT}\""),
        "P=$(cat /w/prompt.txt); claude -p \"$P\"".to_owned(),
        "cat /w/prompt.txt | claude -p > /w/out.txt".to_owned(),
        format!("claude -p --no-such-option \"{PROMPT}\""),
        // Text the shell runs that the reader skips, and text given to a
        // program that may run it.
        format!("OUTPUT=${{OUTPUT:-$(claude -p '{PROMPT}')}}"),
        format!("n=$(( $(claude -p '{PROMPT}' | wc -c) ))"),
        format!("bash -euc \"claude -p '{PROMPT}'\""),
        format!("tmux new-session -d 'claude -p \"{PROMPT}\"'"),
        format!("S=/w; tmux new-session -d \"cd $S && claude -p '{PROMPT}'\""),
        format!(
            "python3 - <<'EOF'\nimport subprocess\nsubprocess.run([\"claude\", \"-p\", \"{PROMPT}\"])\nEOF"
        ),
    ] {
        // Made after the launch and before the child: it could be the
        // child's closest launch.
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
        write_parent(&home, OTHER_PARENT, OTHER_CALL, 1_000, &rival);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (store, total) = run(&home);
        assert_eq!(total.linked, 0, "{rival}: {total:?}");
        unlinked(&home, &store, MAIN_CHILD);
        // Made before the launch, it is not: the link is made.
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
        write_parent(&home, OTHER_PARENT, OTHER_CALL, -2_000, &rival);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (store, total) = run(&home);
        assert_eq!(total.linked, 1, "{rival}: {total:?}");
        assert_eq!(
            shown(&home, &store, MAIN_CHILD),
            (true, Some(session(&store, PARENT)))
        );
    }
    // Commands that only name the folder or the word run no launch.
    for harmless in [
        "ls ~/.claude/projects && cat ~/.claude/settings.json".to_owned(),
        format!("echo claude -p \"{PROMPT}\""),
        format!("echo \"claude -p {PROMPT}\""),
        "grep -rn claude /w/notes.md".to_owned(),
        "grep 'claude -p' /w/notes.md".to_owned(),
        format!("# claude -p \"{PROMPT}\"\ntrue"),
        format!("cat > /w/note.md <<'EOF'\nclaude -p \"{PROMPT}\"\nEOF"),
    ] {
        let home = Home::new();
        write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
        write_parent(&home, OTHER_PARENT, OTHER_CALL, 1_000, &harmless);
        write_child(&home, MAIN_CHILD, 4_800, PROMPT);
        let (_, total) = run(&home);
        assert_eq!(total.linked, 1, "{harmless}: {total:?}");
    }
}

#[test]
fn a_launch_fed_through_a_process_substitution_links_nothing_but_still_stops_a_link() {
    // What `printf` writes reaches the launch's standard input: its child's
    // first input may be more than the prompt.
    let fed = format!("printf extra > >(claude -p \"{PROMPT}\")");
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &fed);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.other_input),
        (1, 0, 1),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
    // Made after a launch that reads only its own input, it is the child's
    // closest launch: neither links.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_parent(&home, OTHER_PARENT, OTHER_CALL, 1_000, &fed);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.other_input),
        (2, 0, 1),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
}

#[test]
fn a_launch_word_the_shell_may_split_links_nothing_and_stops_the_links_it_could_change() {
    // With `MODEL='sonnet --continue'` it resumes a session: it is read as
    // a command the reader cannot read.
    let split = format!("claude --model $MODEL -p \"{PROMPT}\"");
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &split);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.commands_refused),
        (0, 0, 1),
        "{total:?}"
    );
    unlinked(&home, &store, MAIN_CHILD);
    // Made after the launch and before the child, it stops the link.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_parent(&home, OTHER_PARENT, OTHER_CALL, 1_000, &split);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!((total.launches, total.linked), (1, 0), "{total:?}");
    assert!(total.held_command_unread > 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    // Made before the launch, it does not.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    write_parent(&home, OTHER_PARENT, OTHER_CALL, -2_000, &split);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(total.linked, 1, "{total:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_launch_whose_prompt_holds_shell_words_still_reads_its_variable_as_one_word() {
    // Words of the prompt are never programs: `.`, `source`, `eval` and
    // `read` in it do not make the unquoted `$S` unreadable.
    let prompt = "Synthetic check: read the source . Then eval the result.";
    let home = Home::new();
    write_parent(
        &home,
        PARENT,
        MAIN_CALL,
        0,
        &format!(
            "S=/w; cd $S/run && claude -p \"{prompt}\" --mcp-config $S/run/c.json \
             --strict-mcp-config > run1.jsonl 2> run1.err"
        ),
    );
    write_child(&home, MAIN_CHILD, 4_800, prompt);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.commands_refused),
        (1, 1, 0),
        "{total:?}"
    );
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_launch_whose_prompt_names_claude_links() {
    let prompt = "Synthetic check: ask claude to review the diff.";
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(prompt));
    write_child(&home, MAIN_CHILD, 4_800, prompt);
    let (store, total) = run(&home);
    assert_eq!(
        (total.launches, total.linked, total.commands_refused),
        (1, 1, 0),
        "{total:?}"
    );
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}

#[test]
fn a_launch_record_with_no_time_stops_the_links_its_caller_could_change() {
    let untimed = |home: &Home, at: i64| {
        let mut rows = caller_rows(
            OTHER_PARENT,
            None,
            "toolu_01SyntheticOtherCall",
            at - 3_000,
            "echo ready",
        );
        let mut launch = caller_rows(OTHER_PARENT, None, OTHER_CALL, at, &main_command(PROMPT));
        launch[1].as_object_mut().unwrap().remove("timestamp");
        rows.push(launch.remove(1));
        rows.push(launch.remove(1));
        write_rows(&parent_path(home, OTHER_PARENT), &rows);
    };
    // Its caller was writing when the launch was made: no link.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    untimed(&home, 1_000);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (store, total) = run(&home);
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    // Its caller's whole history ended before the launch: the link is made.
    let home = Home::new();
    write_parent(&home, PARENT, MAIN_CALL, 0, &main_command(PROMPT));
    untimed(&home, -120_000);
    write_child(&home, MAIN_CHILD, 4_800, PROMPT);
    let (_, total) = run(&home);
    assert_eq!(total.linked, 1, "{total:?}");
}

#[test]
fn a_link_waits_until_its_launch_hour_is_over() {
    // A launch ten minutes ago: another session of its prompt may still
    // appear within its hour.
    let home = Home::new();
    let launched = T - AT - 10 * MINUTE;
    write_parent(&home, PARENT, MAIN_CALL, launched, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, launched + 4_800, PROMPT);
    let mut store = home.store();
    scan(&home, &mut store);
    let mut backlog = BashBacklog::starting();
    let mut launches = LaunchBacklog::starting();
    let mut passes = |store: &mut Store, backlog: &mut BashBacklog, now: i64| {
        let mut total = PromptProgress::default();
        for _ in 0..50 {
            let pass = pass_at(&home, store, backlog, &mut launches, now);
            add(&mut total, &pass.prompt);
            if !pass.advanced {
                break;
            }
        }
        total
    };
    let waiting = passes(&mut store, &mut backlog, T);
    assert_eq!(waiting.linked, 0, "{waiting:?}");
    assert!(waiting.held_hour >= 1, "{waiting:?}");
    unlinked(&home, &store, MAIN_CHILD);
    assert_eq!(
        backlog
            .prompt_held()
            .iter()
            .map(|held| held.by.clone())
            .collect::<Vec<_>>(),
        [vec![xt_ingest::native::claude_launch::bash::HeldBy::Hour]]
    );
    // It is looked at again once due, and holds no display check meanwhile.
    assert!(backlog.pending());
    assert!(!backlog.prompt_holds_checks());
    // The hour is over.
    let done = passes(&mut store, &mut backlog, AT + launched + 61 * MINUTE);
    assert_eq!(done.linked, 1, "{done:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
    assert!(!backlog.pending());
}

#[test]
fn a_session_imported_while_a_match_waits_is_not_missed() {
    // A launch almost an hour ago and its child. The first match lists its
    // possible children, then waits for the Codex launch work; meanwhile a
    // second session of the prompt, saved inside the launch's hour, is
    // imported. The match must not decide from the list it took before.
    let home = Home::new();
    let launched = T - AT - 59 * MINUTE - MINUTE / 2;
    write_parent(&home, PARENT, MAIN_CALL, launched, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, launched + 4_800, PROMPT);
    let mut store = home.store();
    scan(&home, &mut store);
    let mut backlog = BashBacklog::starting();
    let mut launches = LaunchBacklog::starting();
    let first = pass_at(&home, &mut store, &mut backlog, &mut launches, T);
    assert!(first.prompt.codex_waits >= 1, "{:?}", first.prompt);
    assert_eq!(first.prompt.linked, 0, "{:?}", first.prompt);
    write_child(&home, EXTRA, launched + 59 * MINUTE, PROMPT);
    scan(&home, &mut store);
    backlog.add_sessions([EXTRA]);
    // Later passes, once the launch's hour is over.
    let mut total = PromptProgress::default();
    for _ in 0..50 {
        let pass = pass_at(
            &home,
            &mut store,
            &mut backlog,
            &mut launches,
            AT + launched + 61 * MINUTE,
        );
        add(&mut total, &pass.prompt);
        if !backlog.pending() && !launches.pending() {
            break;
        }
    }
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
    unlinked(&home, &store, EXTRA);
}

#[test]
fn a_deleted_helper_file_in_a_folder_still_there_blocks_a_child_not_linked_yet() {
    // The child's own launch is in a rival's helper file, deleted before any
    // match while its helper folder stays: the earlier launch must not take
    // its child.
    let home = Home::new();
    rival_in_helper(&home);
    let helper = helper_path(&home, OTHER_PARENT);
    let (store, _, total) = run_after(&home, || fs::remove_file(&helper).unwrap());
    assert!(helper.parent().unwrap().is_dir());
    assert_eq!(total.linked, 0, "{total:?}");
    assert!(total.held_caller_gone >= 1, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);

    // The same with another helper file of the rival still listed.
    let home = Home::new();
    rival_in_helper(&home);
    let helper = helper_path(&home, OTHER_PARENT);
    write_rows(
        &helper
            .parent()
            .unwrap()
            .join("agent-a0d00000000000002.jsonl"),
        &caller_rows(
            OTHER_PARENT,
            Some("a0d00000000000002"),
            "toolu_01SyntheticOtherHelper",
            -1_500,
            "echo ready",
        ),
    );
    let (store, _, total) = run_after(&home, || fs::remove_file(&helper).unwrap());
    assert_eq!(total.linked, 0, "{total:?}");
    unlinked(&home, &store, MAIN_CHILD);
}

#[test]
fn looking_at_a_waiting_link_again_holds_no_display_check() {
    // A launch ten minutes ago waits for its hour. When it is looked at
    // again, nothing new was read: its match holds no display check, and
    // the link is still written.
    let home = Home::new();
    let launched = T - AT - 10 * MINUTE;
    write_parent(&home, PARENT, MAIN_CALL, launched, &main_command(PROMPT));
    write_child(&home, MAIN_CHILD, launched + 4_800, PROMPT);
    let mut store = home.store();
    scan(&home, &mut store);
    let mut backlog = BashBacklog::starting();
    let mut launches = LaunchBacklog::starting();
    for _ in 0..50 {
        let pass = pass_at(&home, &mut store, &mut backlog, &mut launches, T);
        if !pass.advanced {
            break;
        }
    }
    assert!(!backlog.prompt_holds_checks());
    assert_eq!(
        backlog
            .prompt_held()
            .iter()
            .map(|held| held.by.clone())
            .collect::<Vec<_>>(),
        [vec![xt_ingest::native::claude_launch::bash::HeldBy::Hour]]
    );
    let after = AT + launched + 61 * MINUTE;
    let mut total = PromptProgress::default();
    for _ in 0..50 {
        let pass = pass_at(&home, &mut store, &mut backlog, &mut launches, after);
        add(&mut total, &pass.prompt);
        assert!(!backlog.prompt_holds_checks(), "{total:?}");
        if !backlog.pending() && !launches.pending() {
            break;
        }
    }
    assert_eq!(total.linked, 1, "{total:?}");
    assert_eq!(
        shown(&home, &store, MAIN_CHILD),
        (true, Some(session(&store, PARENT)))
    );
}
