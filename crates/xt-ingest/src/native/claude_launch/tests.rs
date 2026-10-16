//! Launch passes over synthetic histories, with what only the crate can
//! see: every byte any read returned, and store failures injected at the
//! write boundaries.

use super::*;
use crate::native::{
    ImportRequest, ProducerSource, ScanMode, import_reader_lines, readers_cli::ReaderOutcome,
    scan_native,
};
use serde_json::{Value, json};
use std::{cell::RefCell, fs, os::unix::fs::PermissionsExt};

thread_local! {
    /// A store failure to inject at a named write boundary, after skipping
    /// that many earlier arrivals there.
    static FAULT: RefCell<Option<(&'static str, usize)>> = const { RefCell::new(None) };
}

fn inject(point: &'static str, skip: usize) {
    FAULT.with(|fault| *fault.borrow_mut() = Some((point, skip)));
}

pub(super) fn fault(point: &'static str) -> xt_store::Result<()> {
    FAULT.with(|fault| {
        let mut fault = fault.borrow_mut();
        match fault.as_mut() {
            Some((at, 0)) if *at == point => {
                *fault = None;
                Err(xt_store::Error::InvalidInput("injected store failure"))
            }
            Some((at, skip)) if *at == point => {
                *skip -= 1;
                Ok(())
            }
            _ => Ok(()),
        }
    })
}

const T: i64 = 1_790_700_000_000;
const AT: i64 = 1_790_668_716_333;
const PROMPT: &str = "SYNTHETIC worker brief";

fn iso(offset: i64) -> String {
    chrono::DateTime::from_timestamp_millis(AT + offset)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    store: Store,
}

fn js(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn launch_cell(claude: &str, child: &str) -> String {
    let command = format!(
        "{claude} -p --session-id {child} --output-format json '{PROMPT}' > /w/result.json"
    );
    format!(
        "text(await tools.exec_command({{cmd: {}, workdir: \"/w\"}}))",
        js(&command)
    )
}

fn row(offset: i64, payload: Value) -> Value {
    json!({"timestamp": iso(offset), "type": "response_item", "payload": payload})
}

/// Parents with one direct-exit launch each, a long opening header (larger
/// than a small pass), a continuation whose base cutoff must be checked,
/// and a child transcript larger than a small pass.
fn fixture(parents: &[(&str, &str)]) -> Fixture {
    fixture_with(parents, None)
}

/// As [`fixture`]; with `forked`, each parent is a fork of another,
/// unindexed conversation, whose first header states that fork cutoff.
fn fixture_with(parents: &[(&str, &str)], forked: Option<u64>) -> Fixture {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().canonicalize().unwrap().join("home");
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
    let claude = bin.join("claude").display().to_string();
    let project = home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    let day = home.join(".codex/sessions/2026/09/07");
    fs::create_dir_all(&day).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    for (index, (parent, child)) in parents.iter().enumerate() {
        let original = day.join(format!("rollout-2026-09-07T0{index}-00-00-{parent}.jsonl"));
        let mut header = json!({"timestamp": iso(-10_000), "type": "session_meta", "ordinal": 0,
            "payload": {"id": parent, "cwd": "/w", "source": "cli", "history_mode": "paginated",
                "history_base": null, "instructions": "long instructions ".repeat(1_000)}});
        // A forked parent's history continues the other conversation's two
        // rows, which are never its own.
        let mut start = 0;
        if let Some(stated) = forked {
            let from = format!("01a00000-0000-7000-8000-0000000000f{index}");
            let rows = [
                json!({"timestamp": iso(-20_000), "type": "session_meta", "ordinal": 0,
                    "payload": {"id": from, "cwd": "/w", "source": "cli",
                        "history_mode": "paginated"}}),
                json!({"timestamp": iso(-19_000), "type": "event_msg", "ordinal": 1,
                    "payload": {"type": "note", "text": "before the fork"}}),
            ];
            let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
            fs::write(
                day.join(format!("rollout-2026-09-06T0{index}-00-00-{from}.jsonl")),
                &body,
            )
            .unwrap();
            start = 2;
            header["ordinal"] = json!(start);
            header["payload"]["forked_from_id"] = json!(from);
            header["payload"]["forked_from_ordinal_exclusive"] = json!(stated);
            header["payload"]["history_base"] = json!({"thread_id": from, "end_byte_offset": body.len(), "end_ordinal_exclusive": 2});
        }
        let first = [
            header,
            json!({"timestamp": iso(-9_000), "type": "event_msg", "ordinal": start + 1,
                "payload": {"type": "note", "text": "filler ".repeat(2_000)}}),
        ];
        let first_body: String = first.iter().map(|row| format!("{row}\n")).collect();
        fs::write(&original, &first_body).unwrap();
        let continuation = day.join(format!(
            "rollout-2026-09-07T0{index}-30-00-{parent}_01b00000-0000-7000-8000-00000000000{index}.jsonl"
        ));
        let mut launch = row(
            0,
            json!({"type": "custom_tool_call", "call_id": "call_launch",
            "name": "exec", "input": launch_cell(&claude, child)}),
        );
        launch["ordinal"] = json!(start + 3);
        let mut done = row(
            5,
            json!({"type": "custom_tool_call_output", "call_id": "call_launch",
            "output": [{"type": "input_text", "text": "Script completed\nOutput:\n"},
                       {"type": "input_text", "text": json!({"exit_code": 0, "output": ""}).to_string()}]}),
        );
        done["ordinal"] = json!(start + 4);
        let second = [
            json!({"timestamp": iso(-5_000), "type": "session_meta", "ordinal": start + 2,
                "payload": {"id": parent, "cwd": "/w", "source": "cli", "history_mode": "paginated",
                    "history_base": {"thread_id": parent, "end_byte_offset": first_body.len(),
                        "end_ordinal_exclusive": start + 2}}}),
            launch,
            done,
        ];
        let body: String = second.iter().map(|row| format!("{row}\n")).collect();
        fs::write(&continuation, body).unwrap();
        let lines = vec![
            json!({"type": "session", "host": "codex", "native_session_id": parent,
                   "conversation_id": format!("codex-{parent}"), "source_surface": "codex_cli",
                   "started_at": iso(-10_000), "cwd": "/w", "git_branch": null, "title": null,
                   "path": original.display().to_string(), "mtime": 1790668716.0})
            .to_string(),
            json!({"uuid": format!("5555aaaa-5555-4555-8555-00000000000{index}"), "type": "user",
                   "cwd": "/w", "timestamp": iso(-2500),
                   "message": {"role": "user", "content": [{"type": "text", "text": "request"}]}})
            .to_string(),
        ];
        import_reader_lines(
            &mut store,
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
        let mut rows = vec![
            json!({"type": "user", "uuid": format!("0d000000-0000-4000-8000-00000000000{index}"),
            "parentUuid": null, "timestamp": iso(2), "sessionId": child, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ];
        for n in 0..20 {
            rows.push(json!({"type": "assistant", "uuid": format!("0e000000-0000-4000-8000-0{index}{n:010}"),
                "timestamp": iso(10 + n), "sessionId": child, "cwd": "/w",
                "message": {"role": "assistant", "id": format!("msg_{n}"), "model": "fixture-model",
                    "content": [{"type": "text", "text": "answer ".repeat(100)}],
                    "usage": {"input_tokens": 1, "output_tokens": 1}}}));
        }
        let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
        fs::write(project.join(format!("{child}.jsonl")), body).unwrap();
    }
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Checkout {
                pin: PathBuf::from(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at: T,
            cancel: None,
        },
        ScanMode::Resume,
    );
    assert!(report.complete(), "{report:?}");
    Fixture {
        _temp: temp,
        home,
        store,
    }
}

const PARENT_A: &str = "01a00000-0000-7000-8000-0000000000aa";
const PARENT_B: &str = "01a00000-0000-7000-8000-0000000000bb";
const CHILD_A: &str = "0c000000-0000-4000-8000-0000000000c1";
const CHILD_B: &str = "0c000000-0000-4000-8000-0000000000c2";

fn relations(store: &Store) -> u64 {
    store.claude_launch_summary().unwrap().relations_accepted
}

/// A Codex fork's own history starts at its first rollout, which continues
/// another conversation's: the launch it made is linked to it, and the
/// other conversation's rows are not read for it. A first header whose
/// stated fork cutoff disagrees with its reference is not one history.
#[test]
fn a_forked_parent_links_its_own_launch() {
    for (stated, linked) in [(2, true), (3, false)] {
        let mut fixture = fixture_with(&[(PARENT_A, CHILD_A)], Some(stated));
        let mut backlog = LaunchBacklog::starting();
        for _ in 0..100 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &LaunchLimits::default(),
                None,
                T,
            )
            .unwrap();
            if !backlog.pending() {
                break;
            }
        }
        let summary = fixture.store.claude_launch_summary().unwrap();
        if linked {
            assert_eq!(
                (summary.relations_accepted, summary.groups_invalid),
                (1, 0),
                "{summary:?}"
            );
        } else {
            assert_eq!(
                (summary.relations_accepted, summary.groups_invalid),
                (0, 1),
                "{summary:?}"
            );
        }
    }
}

/// Every byte any read returned, pass by pass, is what the pass was
/// charged, and never more than its limit: the opening line longer than a
/// pass, the base cutoff, the recorded lines and the child's transcript
/// included.
#[test]
fn every_byte_read_is_charged_and_within_each_pass() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let limits = LaunchLimits {
        max_pass_bytes: 3_000,
        ..LaunchLimits::default()
    };
    let mut backlog = LaunchBacklog::starting();
    let mut passes = 0;
    let mut total = 0;
    while backlog.pending() && passes < 10_000 {
        let before = source::READ.with(std::cell::Cell::get);
        let progress = continue_claude_launches(
            &mut fixture.store,
            &fixture.home,
            &mut backlog,
            &limits,
            None,
            T,
        )
        .unwrap();
        let read = source::READ.with(std::cell::Cell::get) - before;
        assert_eq!(read, progress.bytes_read, "pass {passes}");
        assert!(read <= limits.max_pass_bytes, "pass {passes}: {read}");
        total += read;
        passes += 1;
    }
    assert_eq!(relations(&fixture.store), 1);
    assert!(passes > 20, "the work spread over {passes} passes");
    assert!(total > 30_000, "{total}");
}

/// A store failure while staging, or between staging and publishing: the
/// error is returned, the thread stays queued and pending (nothing linked,
/// nothing published), and a later pass publishes and links.
#[test]
fn a_failure_while_staging_or_publishing_publishes_nothing_and_recovers() {
    for point in ["stage", "publish"] {
        let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
        let mut backlog = LaunchBacklog::starting();
        inject(point, 0);
        let mut failed = false;
        for _ in 0..10 {
            let mut progress = LaunchProgress::default();
            let result = continue_claude_launches_into(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &LaunchLimits::default(),
                None,
                T,
                &mut progress,
            );
            if result.is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "{point}");
        let summary = fixture.store.claude_launch_summary().unwrap();
        assert_eq!(
            (
                summary.groups_pending,
                summary.groups_valid,
                summary.candidates_waiting
            ),
            (1, 0, 0),
            "{point}: {summary:?}"
        );
        assert!(
            backlog.threads.contains(&PARENT_A.to_owned()),
            "{point}: still queued"
        );
        for _ in 0..20 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &LaunchLimits::default(),
                None,
                T,
            )
            .unwrap();
        }
        let summary = fixture.store.claude_launch_summary().unwrap();
        assert_eq!(
            (summary.relations_accepted, summary.staged),
            (1, 0),
            "{point}: {summary:?}"
        );
    }
}

/// A store failure at the second link of a pass: the first link's change
/// is already counted, the failing thread keeps its place, and a later pass
/// links it.
#[test]
fn a_failure_after_a_link_keeps_its_count_and_the_queue() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A), (PARENT_B, CHILD_B)]);
    let mut backlog = LaunchBacklog::starting();
    inject("link", 1);
    let mut progress = LaunchProgress::default();
    let mut result = Ok(());
    for _ in 0..10 {
        result = continue_claude_launches_into(
            &mut fixture.store,
            &fixture.home,
            &mut backlog,
            &LaunchLimits::default(),
            None,
            T,
            &mut progress,
        );
        if result.is_err() {
            break;
        }
    }
    assert!(result.is_err());
    assert_eq!(
        progress.changed, 2,
        "the committed child fact and link are counted: {progress:?}"
    );
    assert_eq!(relations(&fixture.store), 1);
    assert!(backlog.threads.contains(&PARENT_B.to_owned()));
    for _ in 0..10 {
        continue_claude_launches(
            &mut fixture.store,
            &fixture.home,
            &mut backlog,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
    }
    assert_eq!(relations(&fixture.store), 2);
}

/// A hash collision can only reject: two distinct identifiers counted
/// under one hash look like a reused identifier, and the chain is dropped;
/// without it the chain is kept. Exact identifiers decide the chain itself.
#[test]
fn a_forced_hash_collision_only_rejects() {
    let allowance = Allowance::new(1 << 20);
    let generation = xt_store::claude_launch::SegmentGeneration {
        device: 1,
        inode: 1,
        length: 100,
        mtime_ns: 1,
        ctime_ns: 1,
    };
    let segment = Segment {
        path: PathBuf::from("/nonexistent"),
        rollout: PARENT_A.into(),
        header: group::Header {
            paginated: false,
            ordinal: None,
            base: None,
            inherited_below: None,
            forked: false,
            role: group::Role::Own,
            copied: false,
        },
        generation,
    };
    let facts = |collide: bool| -> Box<MemberFacts> {
        let mut counts = HashMap::new();
        counts.insert(
            1_u64,
            Counts {
                calls: 1,
                outputs: 1,
            },
        );
        // A second identifier hashing to 2 alongside the chain's own.
        counts.insert(
            2_u64,
            Counts {
                calls: if collide { 2 } else { 1 },
                outputs: 1,
            },
        );
        Box::new(MemberFacts {
            rollout: PARENT_A.into(),
            generation,
            complete: 100,
            valid: true,
            counts,
            chains: vec![scan::Found {
                launch_call_id: "call_launch".into(),
                launch_op: 0,
                child: CHILD_A.into(),
                host: xt_store::Host::Claude,
                acknowledgment_call_id: "call_launch".into(),
                handle: Some("7".into()),
                launch_offset: 10,
                acknowledgment_offset: 50,
                launch_ordinal: None,
                acknowledgment_ordinal: None,
                binding_call_offset: None,
                binding_output_offset: None,
                launch_check_fingerprint: "0".repeat(64),
                ids: vec![1, 2],
            }],
            unfinished: Vec::new(),
            broken: 0,
            entries: allowance.reserve(0).unwrap(),
            bytes: allowance.reserve(0).unwrap(),
        })
    };
    let mut copies = allowance.reserve(0).unwrap();
    let (status, kept, broken) = finalize(
        std::slice::from_ref(&segment),
        &[facts(false)],
        PARENT_A,
        &mut copies,
    )
    .unwrap();
    assert_eq!((status, kept.len(), broken), (GroupStatus::Valid, 1, 0));
    let (status, kept, broken) = finalize(
        std::slice::from_ref(&segment),
        &[facts(true)],
        PARENT_A,
        &mut copies,
    )
    .unwrap();
    assert_eq!((status, kept.len(), broken), (GroupStatus::Valid, 0, 1));
    // Counted across members: the same identifier once in each is twice.
    let mut other = facts(false);
    other.rollout = PARENT_B.into();
    other.chains.clear();
    other.counts.remove(&2);
    let two = [
        segment.clone(),
        Segment {
            rollout: PARENT_B.into(),
            ..segment.clone()
        },
    ];
    let (_, kept, _) = finalize(&two, &[facts(false), other], PARENT_A, &mut copies).unwrap();
    assert!(kept.is_empty(), "identifier 1 occurs in both members");
}

const MIDDLE: &str = "01b00000-0000-7000-8000-0000000000d1";
const LAST: &str = "01b00000-0000-7000-8000-0000000000d2";

/// One parent of three files: a large original, a middle continuation that
/// is the base of the last, and the last holding one direct-exit launch of
/// `CHILD_A`, whose transcript is indexed too. Returns the three paths.
fn three_files() -> (Fixture, [PathBuf; 3]) {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().canonicalize().unwrap().join("home");
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
    let claude = bin.join("claude").display().to_string();
    let project = home.join(".claude/projects/-w");
    fs::create_dir_all(&project).unwrap();
    let day = home.join(".codex/sessions/2026/09/07");
    fs::create_dir_all(&day).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let write = |path: &Path, rows: &[Value]| -> usize {
        let body: String = rows.iter().map(|row| format!("{row}\n")).collect();
        fs::write(path, &body).unwrap();
        body.len()
    };
    let header = |ordinal: i64, base: Value| {
        json!({"timestamp": iso(-10_000), "type": "session_meta", "ordinal": ordinal,
            "payload": {"id": PARENT_A, "cwd": "/w", "source": "cli",
                "history_mode": "paginated", "history_base": base}})
    };
    let note = |ordinal: i64, text: &str| {
        json!({"timestamp": iso(-9_000), "type": "event_msg", "ordinal": ordinal,
            "payload": {"type": "note", "text": text}})
    };
    let paths = [
        day.join(format!("rollout-2026-09-07T00-00-00-{PARENT_A}.jsonl")),
        day.join(format!(
            "rollout-2026-09-07T00-30-00-{PARENT_A}_{MIDDLE}.jsonl"
        )),
        day.join(format!(
            "rollout-2026-09-07T01-00-00-{PARENT_A}_{LAST}.jsonl"
        )),
    ];
    let mut original = vec![header(0, Value::Null)];
    for ordinal in 1..=200 {
        original.push(note(ordinal, &"filler ".repeat(140)));
    }
    let first_len = write(&paths[0], &original);
    let mut middle = vec![header(
        201,
        json!({"thread_id": PARENT_A, "end_byte_offset": first_len, "end_ordinal_exclusive": 201}),
    )];
    for ordinal in 202..=211 {
        middle.push(note(ordinal, "middle"));
    }
    let middle_len = write(&paths[1], &middle);
    let mut launch = row(
        0,
        json!({"type": "custom_tool_call", "call_id": "call_launch",
            "name": "exec", "input": launch_cell(&claude, CHILD_A)}),
    );
    launch["ordinal"] = json!(213);
    let mut done = row(
        5,
        json!({"type": "custom_tool_call_output", "call_id": "call_launch",
            "output": [{"type": "input_text", "text": "Script completed\nOutput:\n"},
                       {"type": "input_text", "text": json!({"exit_code": 0, "output": ""}).to_string()}]}),
    );
    done["ordinal"] = json!(214);
    write(
        &paths[2],
        &[
            header(
                212,
                json!({"thread_id": MIDDLE, "end_byte_offset": middle_len, "end_ordinal_exclusive": 212}),
            ),
            launch,
            done,
        ],
    );
    let lines = vec![
        json!({"type": "session", "host": "codex", "native_session_id": PARENT_A,
               "conversation_id": format!("codex-{PARENT_A}"), "source_surface": "codex_cli",
               "started_at": iso(-10_000), "cwd": "/w", "git_branch": null, "title": null,
               "path": paths[0].display().to_string(), "mtime": 1790668716.0})
        .to_string(),
        json!({"uuid": "5555aaaa-5555-4555-8555-000000000000", "type": "user",
               "cwd": "/w", "timestamp": iso(-2500),
               "message": {"role": "user", "content": [{"type": "text", "text": "request"}]}})
        .to_string(),
    ];
    import_reader_lines(
        &mut store,
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
    write(
        &project.join(format!("{CHILD_A}.jsonl")),
        &[
            json!({"type": "user", "uuid": "0d000000-0000-4000-8000-000000000000",
            "parentUuid": null, "timestamp": iso(2), "sessionId": CHILD_A, "cwd": "/w",
            "message": {"role": "user", "content": PROMPT}}),
        ],
    );
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Claude],
            producer: &ProducerSource::Checkout {
                pin: PathBuf::from(".plugin-pin"),
                plugin_root: None,
            },
            python: None,
            observed_at: T,
            cancel: None,
        },
        ScanMode::Resume,
    );
    assert!(report.complete(), "{report:?}");
    (
        Fixture {
            _temp: temp,
            home,
            store,
        },
        paths,
    )
}

/// Planning checked the middle file's cutoff; then, before the middle file
/// is read, one byte is added inside an earlier row of it. Its rows and
/// ordinals still read well, but the cutoff is no longer a line boundary:
/// the history planned is not the one that would be read, so nothing is
/// linked, and planning again refuses the history. Unchanged, it links.
#[test]
fn a_byte_added_before_a_checked_cutoff_after_planning_links_nothing() {
    for changed in [false, true] {
        let (mut fixture, paths) = three_files();
        let limits = LaunchLimits {
            max_pass_bytes: 16 * 1024,
            ..LaunchLimits::default()
        };
        let mut backlog = LaunchBacklog::starting();
        let mut planned = false;
        for _ in 0..1_000 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            if let Some(ThreadWork {
                phase: Phase::Reading { facts, .. },
                ..
            }) = backlog.work.get(PARENT_A)
                && facts.is_empty()
            {
                planned = true;
                break;
            }
        }
        assert!(planned, "planned, the original still being read");
        if changed {
            let body = fs::read_to_string(&paths[1]).unwrap();
            let row = body.lines().nth(1).unwrap().to_owned();
            let spaced = row.replacen(",\"payload\"", ", \"payload\"", 1);
            assert_eq!(spaced.len(), row.len() + 1);
            fs::write(&paths[1], body.replacen(&row, &spaced, 1)).unwrap();
        }
        for _ in 0..1_000 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            if !backlog.pending() {
                break;
            }
        }
        let summary = fixture.store.claude_launch_summary().unwrap();
        if changed {
            assert_eq!(
                (summary.relations_accepted, summary.groups_invalid),
                (0, 1),
                "{summary:?}"
            );
        } else {
            assert_eq!(
                (summary.relations_accepted, summary.groups_valid),
                (1, 1),
                "{summary:?}"
            );
        }
    }
}

fn strings(values: &[&String]) -> usize {
    values.iter().map(|value| value.capacity()).sum()
}

/// The actual capacity of what a staging attempt keeps: its member facts'
/// launches, the candidates copied from them, the member records and the
/// planned files. The call-count maps are bounded by entries apart.
fn staging_capacity(phase: &Phase) -> usize {
    let Phase::Staging {
        plan,
        facts,
        members,
        candidates,
        ..
    } = phase
    else {
        panic!("staging");
    };
    let segments = &plan.segments;
    let mut total = segments.capacity() * std::mem::size_of::<Segment>()
        + facts.capacity() * std::mem::size_of::<Box<MemberFacts>>()
        + members.capacity() * std::mem::size_of::<MemberRecord>()
        + candidates.capacity() * std::mem::size_of::<LaunchCandidate>();
    for segment in segments {
        total += segment.path.capacity() + segment.rollout.capacity();
        if let Some((base, _, _)) = &segment.header.base {
            total += base.capacity();
        }
    }
    for read in facts {
        total += std::mem::size_of::<MemberFacts>()
            + read.rollout.capacity()
            + read.chains.capacity() * std::mem::size_of::<scan::Found>();
        for found in &read.chains {
            total += strings(&[
                &found.launch_call_id,
                &found.child,
                &found.acknowledgment_call_id,
            ]) + found.handle.as_ref().map_or(0, String::capacity)
                + found.ids.capacity() * 8;
        }
    }
    for member in members {
        total += member.rollout_id.capacity();
    }
    for candidate in candidates {
        total += strings(&[
            &candidate.key.parent_native_session_id,
            &candidate.key.rollout_id,
            &candidate.key.launch_call_id,
            &candidate.child_native_session_id,
            &candidate.acknowledgment_call_id,
        ]) + candidate
            .process_session_id
            .as_ref()
            .map_or(0, String::capacity);
    }
    total
}

/// A staging attempt kept across a failure holds its member facts and the
/// candidates copied from them together: the thread's allowance holds both
/// at their actual capacity.
#[test]
fn staging_reserves_the_candidates_it_copies() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let mut backlog = LaunchBacklog::starting();
    inject("publish", 0);
    let mut failed = false;
    for _ in 0..10 {
        if continue_claude_launches(
            &mut fixture.store,
            &fixture.home,
            &mut backlog,
            &LaunchLimits::default(),
            None,
            T,
        )
        .is_err()
        {
            failed = true;
            break;
        }
    }
    assert!(failed);
    let work = backlog.work.get(PARENT_A).expect("kept");
    let actual = staging_capacity(&work.phase);
    assert!(
        actual <= work.memory.used(),
        "actual {actual} > reserved {}",
        work.memory.used()
    );
}

/// Under lowered limits, with two threads' work held at once and a cache
/// too small for every member: after every pass, what each thread's work
/// and the idle cache keep — measured from their buffers' actual
/// capacities, not from what was reserved — is within what their
/// allowances hold, and each allowance within its limit; the pass's own
/// allowance holds nothing between passes. Both launches link.
#[test]
fn what_the_work_keeps_is_within_its_lowered_limits_at_actual_capacity() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A), (PARENT_B, CHILD_B)]);
    let limits = LaunchLimits {
        max_pass_bytes: 3_000,
        thread_memory: 256 << 10,
        cache_memory: 8 << 10,
        pass_memory: 1 << 20,
        ..LaunchLimits::default()
    };
    let mut backlog = LaunchBacklog::starting();
    let (mut together, mut cached) = (false, false);
    let mut phases = std::collections::BTreeSet::new();
    for pass in 0..10_000 {
        continue_claude_launches(
            &mut fixture.store,
            &fixture.home,
            &mut backlog,
            &limits,
            None,
            T,
        )
        .unwrap();
        together |= backlog.work.len() == 2;
        for (thread, work) in &backlog.work {
            phases.insert(match work.phase {
                Phase::Planning { .. } => "planning",
                Phase::Reading { .. } => "reading",
                Phase::Staging { .. } => "staging",
                Phase::Resolving { .. } => "resolving",
            });
            let (actual, used) = (work.phase.actual(), work.memory.used());
            assert!(
                actual <= used && used <= limits.thread_memory,
                "pass {pass} {thread}: actual {actual}, reserved {used}"
            );
        }
        let cache = backlog.cache.as_ref().unwrap();
        let actual: usize = cache
            .facts
            .values()
            .map(|read| MemberFacts::held(&read.rollout, &read.chains, &read.unfinished))
            .sum();
        cached |= !cache.facts.is_empty();
        assert!(
            actual <= cache.memory.used() && cache.memory.used() <= limits.cache_memory,
            "pass {pass}: cache actual {actual}, reserved {}",
            cache.memory.used()
        );
        assert_eq!(backlog.pass_memory.as_ref().unwrap().used(), 0);
        if !backlog.pending() {
            break;
        }
    }
    assert!(together, "two threads held work at once");
    assert!(cached, "the cache held facts");
    for phase in ["planning", "reading", "resolving"] {
        assert!(
            phases.contains(phase),
            "{phase} seen at a pass's end: {phases:?}"
        );
    }
    assert_eq!(relations(&fixture.store), 2);
}

/// Allocations counted per thread, so a test sees what one call actually
/// allocated at its peak.
mod counting {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };

    pub struct Counting;

    thread_local! {
        static LIVE: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }

    fn add(bytes: usize) {
        let _ = LIVE.try_with(|live| {
            live.set(live.get() + bytes as isize);
            let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
        });
    }

    fn sub(bytes: usize) {
        let _ = LIVE.try_with(|live| live.set(live.get() - bytes as isize));
    }

    // SAFETY: every call is passed to the system allocator unchanged; only
    // thread-local counters without destructors are touched besides.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let block = unsafe { System.alloc(layout) };
            if !block.is_null() {
                add(layout.size());
            }
            block
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let block = unsafe { System.alloc_zeroed(layout) };
            if !block.is_null() {
                add(layout.size());
            }
            block
        }

        unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
            // SAFETY: the caller's contract, passed on.
            unsafe { System.dealloc(block, layout) };
            sub(layout.size());
        }

        unsafe fn realloc(&self, block: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let moved = unsafe { System.realloc(block, layout, size) };
            if !moved.is_null() {
                sub(layout.size());
                add(size);
            }
            moved
        }
    }

    /// The most `run` had allocated at once beyond what was live before it.
    pub fn peak(run: impl FnOnce()) -> usize {
        let base = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(base));
        run();
        (PEAK.with(Cell::get) - base).max(0) as usize
    }
}

#[global_allocator]
static ALLOCATOR: counting::Counting = counting::Counting;

/// Decoding one row, whole or for its structural fields, allocates at its
/// peak no more than the bound reserved for it, for ordinary rows and for
/// rows built to allocate the most per byte.
#[test]
fn decoding_a_row_allocates_within_its_reserved_bound() {
    let escaped = "line \"quoted\"\n\t\\ ".repeat(20_000);
    let parts: Vec<Value> = (0..20_000)
        .map(|n| {
            if n % 2 == 0 {
                json!({})
            } else {
                json!({"type": "input_text", "text": "x"})
            }
        })
        .collect();
    let lines: Vec<String> = vec![
        serde_json::to_string(&vec![0; 100_000]).unwrap(),
        serde_json::to_string(&vec![json!({"a": 0}); 20_000]).unwrap(),
        serde_json::to_string(
            &(0..20_000)
                .map(|n| (format!("k{n}"), json!(0)))
                .collect::<serde_json::Map<_, _>>(),
        )
        .unwrap(),
        format!("{}{}", "[".repeat(100), "]".repeat(100)),
        serde_json::to_string(&vec![json!([]); 50_000]).unwrap(),
        json!({"timestamp": iso(0), "type": "response_item", "ordinal": 3, "payload": {
            "type": "custom_tool_call_output", "call_id": "call_x", "output": parts}})
        .to_string(),
        json!({"timestamp": iso(0), "type": "response_item", "payload": {
            "type": "function_call_output", "call_id": "call_x", "output": escaped}})
        .to_string(),
        json!({"timestamp": iso(0), "type": "response_item", "payload": {
            "type": "custom_tool_call", "call_id": "call_x", "name": "exec",
            "input": format!("{}{}", launch_cell("/bin/claude", CHILD_A), escaped)}})
        .to_string(),
        json!({"timestamp": iso(0), "type": "response_item", "payload": {
            "type": "function_call", "call_id": "call_w", "name": "wait",
            "arguments": json!({"cell_id": "12", "yield_time_ms": 30000}).to_string()}})
        .to_string(),
        json!({"timestamp": iso(0), "type": "response_item", "payload": {
            "type": "custom_tool_call_output", "call_id": "call_x", "output": [
                {"type": "input_text", "text": "Script running with cell ID 12\n"},
                {"type": "input_text", "text": json!({"session_id": 56751, "output": escaped}).to_string()}]}})
        .to_string(),
    ];
    for line in &lines {
        let bytes = line.as_bytes();
        let whole = counting::peak(|| {
            let value = serde_json::from_slice::<Value>(bytes).unwrap();
            let item = rows::item(std::hint::black_box(&value));
            std::hint::black_box(&item);
        });
        let bound = source::decoded_bound(bytes);
        assert!(
            whole <= bound,
            "whole: {whole} > {bound} for {} bytes: {}",
            bytes.len(),
            &line[..80.min(line.len())]
        );
        let head = counting::peak(|| {
            std::hint::black_box(rows::row(bytes));
        });
        let bound = rows::row_bound(bytes);
        assert!(
            head <= bound,
            "head: {head} > {bound} for {} bytes: {}",
            bytes.len(),
            &line[..80.min(line.len())]
        );
    }
}

/// A long opening header of the wrong thread is being read when the file is
/// replaced by same-length valid content. The bytes already read fail, but
/// they describe a generation that is gone: nothing is published invalid
/// for the replacement, which is planned again with no further event and
/// linked. An unchanged malformed header is published invalid and not read
/// again.
#[test]
fn a_header_replaced_while_it_was_read_is_planned_again() {
    for replaced in [false, true] {
        let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
        let day = fixture.home.join(".codex/sessions/2026/09/07");
        let original = day.join(format!("rollout-2026-09-07T00-00-00-{PARENT_A}.jsonl"));
        let valid = fs::read_to_string(&original).unwrap();
        // Another thread's ID, same length, early in the long header.
        let wrong = valid.replacen(PARENT_A, PARENT_B, 1);
        assert_eq!(wrong.len(), valid.len());
        fs::write(&original, &wrong).unwrap();
        let limits = LaunchLimits {
            max_pass_bytes: 5_000,
            ..LaunchLimits::default()
        };
        let mut backlog = LaunchBacklog::starting();
        let mut planning = false;
        for _ in 0..20 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            if matches!(
                backlog.work.get(PARENT_A).map(|work| &work.phase),
                Some(Phase::Planning { .. })
            ) {
                planning = true;
                break;
            }
        }
        assert!(planning, "the header is being read");
        if replaced {
            fs::write(&original, &valid).unwrap();
        }
        let mut read = Vec::new();
        for _ in 0..2_000 {
            let pass = continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            read.push(pass.bytes_read);
            if !backlog.pending() {
                break;
            }
        }
        let summary = fixture.store.claude_launch_summary().unwrap();
        if replaced {
            assert_eq!(
                (summary.relations_accepted, summary.groups_valid),
                (1, 1),
                "{summary:?}"
            );
        } else {
            assert_eq!(
                (summary.relations_accepted, summary.groups_invalid),
                (0, 1),
                "{summary:?}"
            );
            // A restarted worker reads it once more, to learn again why it
            // is invalid, and then not again while it is unchanged.
            let mut backlog = LaunchBacklog::starting();
            for _ in 0..50 {
                if !backlog.pending() {
                    break;
                }
                continue_claude_launches(
                    &mut fixture.store,
                    &fixture.home,
                    &mut backlog,
                    &limits,
                    None,
                    T,
                )
                .unwrap();
            }
            backlog.add_threads([PARENT_A]);
            let again = continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            assert_eq!(
                (again.threads_unchanged, again.files_read),
                (1, 0),
                "not read again: {again:?}"
            );
            assert_eq!(
                fixture
                    .store
                    .claude_launch_summary()
                    .unwrap()
                    .groups_invalid,
                1
            );
        }
    }
}

/// A continuation with a long opening header of another thread is being read
/// when it is removed and the census taken again. The bytes the open file
/// still supplies fail, but the file is gone: nothing is published invalid
/// for the rest of the history, which is planned again as it now is (the
/// original alone, valid). An unchanged malformed continuation is published
/// invalid and not read again.
#[test]
fn a_malformed_member_removed_while_it_was_read_refuses_nothing() {
    for removed in [false, true] {
        let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
        let day = fixture.home.join(".codex/sessions/2026/09/07");
        let continuation = fs::read_dir(&day)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().contains(&format!("{PARENT_A}_")))
            .unwrap();
        let body = fs::read_to_string(&continuation).unwrap();
        let (first, rest) = body.split_once('\n').unwrap();
        let mut header: Value = serde_json::from_str(first).unwrap();
        header["payload"]["id"] = json!(PARENT_B);
        header["payload"]["instructions"] = json!("long instructions ".repeat(1_500));
        fs::write(&continuation, format!("{header}\n{rest}")).unwrap();
        let limits = LaunchLimits {
            max_pass_bytes: 5_000,
            ..LaunchLimits::default()
        };
        let mut backlog = LaunchBacklog::starting();
        let mut reading = false;
        for _ in 0..40 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            // The original's header read, the continuation's under way.
            if let Some(ThreadWork {
                phase: Phase::Planning { planner, .. },
                ..
            }) = backlog.work.get(PARENT_A)
                && planner.observed().len() == 2
            {
                reading = true;
                break;
            }
        }
        assert!(reading, "the continuation's header is being read");
        if removed {
            fs::remove_file(&continuation).unwrap();
            backlog.add_threads([PARENT_A]);
        }
        for _ in 0..2_000 {
            continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            if !backlog.pending() {
                break;
            }
        }
        let summary = fixture.store.claude_launch_summary().unwrap();
        if removed {
            assert_eq!(
                (
                    summary.groups_valid,
                    summary.groups_invalid,
                    summary.members
                ),
                (1, 0, 1),
                "{summary:?}"
            );
        } else {
            assert_eq!(
                (summary.groups_invalid, summary.relations_accepted),
                (1, 0),
                "{summary:?}"
            );
            // A restarted worker reads it once more, to learn again why it
            // is invalid, and then not again while it is unchanged.
            let mut backlog = LaunchBacklog::starting();
            for _ in 0..50 {
                if !backlog.pending() {
                    break;
                }
                continue_claude_launches(
                    &mut fixture.store,
                    &fixture.home,
                    &mut backlog,
                    &limits,
                    None,
                    T,
                )
                .unwrap();
            }
            backlog.add_threads([PARENT_A]);
            let again = continue_claude_launches(
                &mut fixture.store,
                &fixture.home,
                &mut backlog,
                &limits,
                None,
                T,
            )
            .unwrap();
            assert_eq!(
                (again.threads_unchanged, again.files_read),
                (1, 0),
                "not read again: {again:?}"
            );
            assert_eq!(
                fixture
                    .store
                    .claude_launch_summary()
                    .unwrap()
                    .groups_invalid,
                1
            );
        }
    }
}

/// Passes until the backlog has nothing left, at most `limit`.
fn drain_launches(
    fixture: &mut Fixture,
    backlog: &mut LaunchBacklog,
    limit: usize,
) -> LaunchProgress {
    let mut sum = LaunchProgress::default();
    for _ in 0..limit {
        let progress = continue_claude_launches(
            &mut fixture.store,
            &fixture.home,
            backlog,
            &LaunchLimits::default(),
            None,
            T,
        )
        .unwrap();
        sum.published_invalid += progress.published_invalid;
        sum.published_valid += progress.published_valid;
        sum.threads_busy += progress.threads_busy;
        sum.threads_unfinished += progress.threads_unfinished;
        if !backlog.pending() {
            break;
        }
    }
    sum
}

/// The first parent's continuation file.
fn continuation(fixture: &Fixture) -> PathBuf {
    fs::read_dir(fixture.home.join(".codex/sessions/2026/09/07"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(PARENT_A) && name.contains("_01b"))
        })
        .unwrap()
}

fn group_status(fixture: &Fixture) -> GroupStatus {
    fixture
        .store
        .claude_launch_group(PARENT_A)
        .unwrap()
        .unwrap()
        .status
}

/// A read error that may pass (too many open files, an IO error) is tried
/// again: the history is not published invalid and keeps its launches; once
/// reads work, it is read. Only "permission denied" skips a history.
#[test]
fn a_read_error_that_may_pass_is_retried_and_keeps_the_launches() {
    for code in [libc::EIO, libc::EMFILE, libc::ENFILE] {
        let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
        let mut backlog = LaunchBacklog::starting();
        drain_launches(&mut fixture, &mut backlog, 100);
        assert_eq!(relations(&fixture.store), 1);
        // The history grows: it is read again, and every read fails.
        let path = continuation(&fixture);
        let mut body = fs::read_to_string(&path).unwrap();
        body.push_str(&format!(
            "{}\n",
            json!({"timestamp": iso(9), "type": "event_msg", "ordinal": 5,
                "payload": {"type": "note", "text": "later"}})
        ));
        fs::write(&path, body).unwrap();
        backlog.add_threads([PARENT_A]);
        source::fail_reads(Some(code));
        let failing = drain_launches(&mut fixture, &mut backlog, 10);
        source::fail_reads(None);
        assert_eq!(failing.published_invalid, 0, "{code}");
        assert!(failing.threads_busy > 0, "{code}: retried");
        assert!(backlog.pending(), "{code}: still to be read");
        assert_ne!(group_status(&fixture), GroupStatus::Invalid, "{code}");
        assert!(
            !fixture
                .store
                .claude_launch_candidates_for_child(CHILD_A, None, 10)
                .unwrap()
                .is_empty(),
            "{code}: its launch is kept"
        );
        assert_eq!(backlog.search_gaps().unwrap().unsearched, 0, "{code}");
        drain_launches(&mut fixture, &mut backlog, 100);
        assert_eq!(group_status(&fixture), GroupStatus::Valid, "{code}");
        assert_eq!(relations(&fixture.store), 1, "{code}");
    }
}

/// A last line with no newline in a file whose modification time is a day
/// ahead (another machine's clock) holds its history only until this run
/// has seen that exact file unchanged for the quiet period; the history is
/// then read without that line, its launch links, and the sighting is
/// forgotten.
#[test]
fn an_unfinished_line_with_a_future_time_is_released_once_seen_unchanged() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let path = continuation(&fixture);
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str("{\"timestamp\": \"2026-09-07T00:00:00.000Z\", \"type\": \"respon");
    fs::write(&path, body).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(86_400))
        .unwrap();
    let mut backlog = LaunchBacklog::starting();
    let held = drain_launches(&mut fixture, &mut backlog, 20);
    assert!(held.threads_unfinished > 0);
    assert!(backlog.held.contains_key(PARENT_A));
    assert_eq!(backlog.sightings.len(), 1);
    assert_eq!(relations(&fixture.store), 0);
    assert_ne!(group_status(&fixture), GroupStatus::Valid);
    // As if the quiet period passed with the file unchanged.
    backlog.sightings.age(source::UNFINISHED_QUIET);
    drain_launches(&mut fixture, &mut backlog, 100);
    assert!(!backlog.pending());
    assert_eq!(group_status(&fixture), GroupStatus::Valid);
    assert_eq!(relations(&fixture.store), 1);
    assert_eq!(backlog.sightings.len(), 0, "forgotten once read");
    assert_eq!(
        backlog.search_gaps(),
        Some(SearchGaps {
            unsearched: 0,
            forks_not_searched: 0,
            unfinished_line_skipped: 1,
            ..SearchGaps::default()
        })
    );
}

/// An empty file among a history's files adds nothing: the rest of the
/// history is read, its launch links, and the empty file is one of the
/// published members with nothing read of it.
#[test]
fn an_empty_file_beside_a_history_adds_nothing_and_breaks_nothing() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let empty = fixture.home.join(format!(
        ".codex/sessions/2026/09/07/rollout-2026-09-07T00-45-00-{PARENT_A}_01c00000-0000-7000-8000-000000000000.jsonl"
    ));
    fs::write(&empty, b"").unwrap();
    let mut backlog = LaunchBacklog::starting();
    drain_launches(&mut fixture, &mut backlog, 100);
    assert!(!backlog.pending());
    assert_eq!(group_status(&fixture), GroupStatus::Valid);
    assert_eq!(relations(&fixture.store), 1);
    let members = fixture.store.claude_launch_members(PARENT_A).unwrap();
    assert_eq!(members.len(), 3);
    assert!(members.iter().any(|member| {
        member.rollout_id == "01c00000-0000-7000-8000-000000000000"
            && member.valid
            && (member.scanned_length, member.generation.length) == (0, 0)
    }));
    assert_eq!(backlog.search_gaps(), Some(SearchGaps::default()));
}

/// A Claude session no launch names.
const MAIN: &str = "0c000000-0000-4000-8000-0000000000e3";
/// A fresh `codex exec` thread no launch names.
const EXEC: &str = "01a60000-0000-7000-8000-0000000000e1";
/// Another Codex history, read for launches but naming none.
const OTHER: &str = "01a60000-0000-7000-8000-0000000000f1";
/// A Codex history whose `codex exec` launch has no first result yet.
const LAUNCHER: &str = "01a60000-0000-7000-8000-0000000000f2";

const EXEC_COMMAND: &str = "codex exec --json --ignore-user-config --sandbox read-only \
    --skip-git-repo-check 'Reply exactly SYNTHETIC-CONTROL.'";

fn exec_call(offset: i64, ordinal: i64) -> Value {
    let input = format!(
        "text(await tools.exec_command({{cmd:{},workdir:\"/w\",yield_time_ms:1000}}));\n",
        js(EXEC_COMMAND)
    );
    let mut call = row(
        offset,
        json!({"type": "custom_tool_call", "call_id": "call_exec", "name": "exec",
            "input": input}),
    );
    call["ordinal"] = json!(ordinal);
    call
}

fn exec_result(offset: i64, ordinal: i64, thread: &str) -> Value {
    let printed = format!(
        "Reading additional input from stdin...\n{{\"type\":\"thread.started\",\"thread_id\":\"{thread}\"}}\n"
    );
    let running = json!({"chunk_id": "6f9747", "wall_time_seconds": 1.0, "session_id": 69773,
        "original_token_count": 35, "output": printed});
    let mut output = row(
        offset,
        json!({"type": "custom_tool_call_output", "call_id": "call_exec", "output": [
            {"type": "input_text", "text": "Script completed\nWall time 1.0 seconds\nOutput:\n"},
            {"type": "input_text", "text": running.to_string()}]}),
    );
    output["ordinal"] = json!(ordinal);
    output
}

fn rows_text(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn codex_header(thread: &str, offset: i64, source: &str) -> Value {
    json!({"timestamp": iso(offset), "type": "session_meta", "ordinal": 0,
        "payload": {"id": thread, "session_id": thread, "timestamp": iso(offset), "cwd": "/w",
            "originator": "codex_exec", "source": source, "history_mode": "paginated"}})
}

/// The fixture's parent and child, a Claude session and a fresh `codex
/// exec` thread nothing launched, and one more Codex history.
struct World {
    fixture: Fixture,
    spawns: crate::native::session_creation::SpawnBacklog,
    other: PathBuf,
}

impl World {
    fn new() -> Self {
        let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
        let home = fixture.home.clone();
        let rows = [
            json!({"type": "user", "uuid": format!("{MAIN}-first"), "parentUuid": null,
                "timestamp": iso(3_000), "sessionId": MAIN, "cwd": "/w",
                "message": {"role": "user", "content": "Hello"}}),
            json!({"type": "assistant", "uuid": format!("{MAIN}-answer"),
                "parentUuid": format!("{MAIN}-first"), "timestamp": iso(5_000),
                "sessionId": MAIN, "cwd": "/w", "message": {"role": "assistant",
                    "id": "msg_main", "model": "fixture-model",
                    "content": [{"type": "text", "text": "Hi"}],
                    "usage": {"input_tokens": 1, "output_tokens": 1}}}),
        ];
        fs::write(
            home.join(format!(".claude/projects/-w/{MAIN}.jsonl")),
            rows_text(&rows),
        )
        .unwrap();
        let day = home.join(".codex/sessions/2026/10/01");
        fs::create_dir_all(&day).unwrap();
        let other = day.join(format!("rollout-2026-10-01T09-00-00-{OTHER}.jsonl"));
        let mut note = row(-50_000, json!({"type": "event_msg", "text": "working"}));
        note["ordinal"] = json!(1);
        fs::write(
            &other,
            rows_text(&[codex_header(OTHER, -60_000, "vscode"), note]),
        )
        .unwrap();
        let exec_day = home.join(".codex/sessions/2026/10/02");
        fs::create_dir_all(&exec_day).unwrap();
        let exec = exec_day.join(format!("rollout-2026-10-02T09-00-00-{EXEC}.jsonl"));
        let mut prompt = row(
            8_294,
            json!({"type": "message", "role": "user", "id": "msg_synthetic",
                "content": [{"type": "input_text", "text": "Reply exactly SYNTHETIC-CONTROL."}]}),
        );
        prompt["ordinal"] = json!(1);
        fs::write(
            &exec,
            rows_text(&[codex_header(EXEC, 6_766, "exec"), prompt]),
        )
        .unwrap();
        let lines = vec![
            json!({"type": "session", "host": "codex", "native_session_id": EXEC,
                   "conversation_id": format!("codex-{EXEC}"), "source_surface": "codex_cli",
                   "started_at": iso(6_766), "cwd": "/w", "git_branch": null, "title": null,
                   "path": exec.display().to_string(), "mtime": 1790668716.0})
            .to_string(),
            json!({"uuid": "5555eeee-5555-4555-8555-000000000000", "type": "user", "cwd": "/w",
                   "timestamp": iso(8_294),
                   "message": {"role": "user", "content": [{"type": "text", "text": "Reply"}]}})
            .to_string(),
        ];
        import_reader_lines(
            &mut fixture.store,
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
        let report = scan_native(
            &mut fixture.store,
            &ImportRequest {
                home: &home,
                hosts: &[Host::Claude],
                producer: &ProducerSource::Checkout {
                    pin: PathBuf::from(".plugin-pin"),
                    plugin_root: None,
                },
                python: None,
                observed_at: T,
                cancel: None,
            },
            ScanMode::Resume,
        );
        assert!(report.complete(), "{report:?}");
        Self {
            fixture,
            spawns: crate::native::session_creation::SpawnBacklog::starting(),
            other,
        }
    }

    /// The watcher's passes, `count` of them or until nothing is left.
    fn passes(&mut self, count: usize) {
        for _ in 0..count {
            crate::native::session_creation::continue_codex_spawns(
                &mut self.fixture.store,
                &self.fixture.home,
                &mut self.spawns,
                crate::native::session_creation::spawn_limits(),
                None,
                T,
            )
            .unwrap();
            if !self.spawns.pending() {
                break;
            }
        }
    }

    /// Whether the Claude session and the fresh `exec` thread are checked.
    fn checked(&self) -> (bool, bool) {
        (
            self.fixture.store.completed_child_check(MAIN).unwrap(),
            self.fixture
                .store
                .completed_child_check(&format!("codex-{EXEC}"))
                .unwrap(),
        )
    }

    fn gaps(&self) -> SearchGaps {
        self.spawns.launches.search_gaps().expect("census current")
    }

    /// As if the quiet period passed.
    fn wait(&mut self) {
        self.spawns.launches.sightings.age(source::UNFINISHED_QUIET);
    }

    fn other_status(&self) -> Option<GroupStatus> {
        self.fixture
            .store
            .claude_launch_group(OTHER)
            .unwrap()
            .map(|group| group.status)
    }
}

/// Whether this user is held to file permissions (root reads anything).
fn permissions_apply(path: &Path) -> bool {
    fs::File::open(path).is_err()
}

/// A: a history whose reads keep failing with an IO error holds the
/// display checks only for the quiet period, then is skipped, counted and
/// still read in the background; once it reads, it is read normally.
#[test]
fn a_history_whose_reads_keep_failing_is_skipped_after_the_quiet_period() {
    let mut world = World::new();
    source::fail_reads_of(&world.other, 0, libc::EIO);
    world.passes(30);
    assert_eq!(world.checked(), (false, false), "held while its clock runs");
    world.wait();
    world.passes(30);
    assert_eq!(world.checked(), (true, true));
    assert_eq!(world.gaps().unsearched, 1);
    assert_ne!(
        world.other_status(),
        Some(GroupStatus::Invalid),
        "nothing published"
    );
    source::fail_reads(None);
    world.passes(100);
    assert_eq!(world.other_status(), Some(GroupStatus::Valid));
    assert_eq!(world.gaps(), SearchGaps::default());
}

/// A: a failing history whose file keeps changing starts its clock again
/// each time, and keeps holding.
#[test]
fn a_failing_history_that_keeps_changing_keeps_holding() {
    let mut world = World::new();
    source::fail_reads_of(&world.other, 0, libc::EIO);
    world.passes(10);
    world.wait();
    // Changed after the wait, in place: same file, new length.
    let mut body = fs::read_to_string(&world.other).unwrap();
    let mut more = row(-40_000, json!({"type": "event_msg", "text": "more"}));
    more["ordinal"] = json!(2);
    body.push_str(&rows_text(&[more]));
    fs::write(&world.other, body).unwrap();
    world.passes(10);
    source::fail_reads(None);
    assert_eq!(world.checked(), (false, false));
}

/// C: a history file the census lists but cannot stat (its folder may be
/// listed but not entered) holds only for the quiet period, then is
/// skipped and counted; entered again, it is read.
#[test]
fn a_history_that_cannot_be_stated_is_skipped_after_the_quiet_period() {
    let mut world = World::new();
    let day = world.other.parent().unwrap().to_owned();
    fs::set_permissions(&day, fs::Permissions::from_mode(0o444)).unwrap();
    if !permissions_apply(&world.other) {
        fs::set_permissions(&day, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user reads files whatever their permissions");
        return;
    }
    world.passes(30);
    let held = world.checked();
    world.wait();
    world.passes(30);
    let released = world.checked();
    let gaps = world.gaps();
    fs::set_permissions(&day, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(held, (false, false));
    assert_eq!(released, (true, true));
    assert_eq!((gaps.unsearched, gaps.folders_not_read), (1, 0));
    world.passes(100);
    assert_eq!(world.other_status(), Some(GroupStatus::Valid));
    assert_eq!(world.gaps(), SearchGaps::default());
}

/// B: a folder the census cannot list holds only for the quiet period;
/// the census is then used without it, counted, and not walked again on
/// every pass. Readable again, the next census reads it.
#[test]
fn a_folder_that_cannot_be_listed_is_skipped_after_the_quiet_period() {
    let mut world = World::new();
    let day = world.other.parent().unwrap().to_owned();
    fs::set_permissions(&day, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&day).is_ok() {
        fs::set_permissions(&day, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user lists folders whatever their permissions");
        return;
    }
    world.passes(30);
    let held = world.checked();
    world.wait();
    world.passes(30);
    let released = world.checked();
    let gaps = world.gaps();
    let epoch = world.spawns.launches.census_epoch();
    world.passes(5);
    let walks = world.spawns.launches.census_epoch() - epoch;
    fs::set_permissions(&day, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(held, (false, false));
    assert_eq!(released, (true, true));
    assert_eq!((gaps.folders_not_read, gaps.unsearched), (1, 0));
    assert_eq!(walks, 0, "an accepted census is not walked again each pass");
    world.spawns.launches.renew_census();
    world.passes(100);
    assert_eq!(world.other_status(), Some(GroupStatus::Valid));
    assert_eq!(world.gaps(), SearchGaps::default());
}

/// E: a census that stops at its entry limit holds only for the quiet
/// period, then its partial list is used and counted.
#[test]
fn a_census_over_its_entry_limit_is_used_after_the_quiet_period() {
    let mut world = World::new();
    group::CENSUS_LIMIT.with(|limit| limit.set(Some(3)));
    world.passes(30);
    let held = world.checked();
    world.wait();
    world.passes(30);
    let released = world.checked();
    let gaps = world.gaps();
    group::CENSUS_LIMIT.with(|limit| limit.set(None));
    assert_eq!(held, (false, false));
    assert_eq!(released, (true, true));
    assert!(gaps.too_many_files);
    world.spawns.launches.renew_census();
    world.passes(100);
    assert!(!world.gaps().too_many_files);
}

/// D: a `codex exec` launch with no first result holds the fresh `exec`
/// threads opened after it only while its history is being written; the
/// Claude session is never held by it. Released, it is counted and kept: a
/// result saved later still makes its thread a child.
#[test]
fn an_exec_launch_with_no_result_holds_only_while_its_history_is_written() {
    let mut world = World::new();
    let day = world.fixture.home.join(".codex/sessions/2026/10/01");
    let launcher = day.join(format!("rollout-2026-10-01T10-00-00-{LAUNCHER}.jsonl"));
    let head = [codex_header(LAUNCHER, -10_000, "vscode"), exec_call(0, 1)];
    fs::write(&launcher, rows_text(&head)).unwrap();
    world.passes(30);
    assert_eq!(
        world.checked(),
        (true, false),
        "only the exec thread is held"
    );
    world.wait();
    world.passes(30);
    assert_eq!(world.checked(), (true, true));
    assert_eq!(world.gaps().exec_launches_without_result, 1);
    // Its result is saved: the thread it opened is its child after all.
    let mut body = rows_text(&head);
    body.push_str(&rows_text(&[exec_result(7_319, 2, EXEC)]));
    fs::write(&launcher, body).unwrap();
    world.spawns.launches.add_threads([LAUNCHER]);
    world.spawns.launches.add_children([EXEC]);
    world.passes(100);
    assert!(
        !world
            .fixture
            .store
            .child_facts(&format!("codex-{EXEC}"))
            .unwrap()
            .is_empty(),
        "a known sub-session wins"
    );
    assert_eq!(world.gaps().exec_launches_without_result, 0);
}

/// A read that progresses over many passes with no error is never skipped,
/// however long it takes; an error past the first pass's budget starts the
/// clock, and pauses before it do not start it again.
#[test]
fn only_a_failing_read_is_skipped_not_a_long_one() {
    let limits = LaunchLimits {
        max_pass_bytes: 3_000,
        ..LaunchLimits::default()
    };
    let pass = |fixture: &mut Fixture, backlog: &mut LaunchBacklog| {
        continue_claude_launches(&mut fixture.store, &fixture.home, backlog, &limits, None, T)
            .unwrap()
    };
    // A long read, no error: paused, never skipped.
    let mut long = fixture(&[(PARENT_A, CHILD_A)]);
    let mut backlog = LaunchBacklog::starting();
    for _ in 0..4 {
        pass(&mut long, &mut backlog);
    }
    assert!(backlog.work.contains_key(PARENT_A), "read in progress");
    backlog.sightings.age(source::UNFINISHED_QUIET);
    assert!(!backlog.discovered_since(0));
    assert_eq!(backlog.search_gaps().unwrap().unsearched, 0);
    // An error at byte 10 000 of its first file, several passes in.
    let mut failing_fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let first = fs::read_dir(failing_fixture.home.join(".codex/sessions/2026/09/07"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&format!("{PARENT_A}.jsonl")))
        })
        .unwrap();
    source::fail_reads_of(&first, 10_000, libc::EIO);
    let mut backlog = LaunchBacklog::starting();
    let mut failing = 0;
    for _ in 0..40 {
        failing += pass(&mut failing_fixture, &mut backlog).threads_busy;
    }
    assert!(failing > 1, "met the error more than once, {failing}");
    assert!(!backlog.discovered_since(0), "held before the wait");
    backlog.sightings.age(source::UNFINISHED_QUIET);
    let skipped = backlog.discovered_since(0);
    let gaps = backlog.search_gaps().unwrap();
    source::fail_reads(None);
    assert!(skipped, "skipped after the wait");
    assert_eq!(gaps.unsearched, 1);
}

/// The first parent's original file.
fn original(fixture: &Fixture) -> PathBuf {
    fs::read_dir(fixture.home.join(".codex/sessions/2026/09/07"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&format!("{PARENT_A}.jsonl")))
        })
        .unwrap()
}

/// A read error while the base cutoff is checked is only that: the history
/// is read again, not refused, and once reads work it is valid and its
/// launch links.
#[test]
fn a_read_error_while_a_cutoff_is_checked_is_retried() {
    let mut fixture = fixture(&[(PARENT_A, CHILD_A)]);
    let base = original(&fixture);
    let cutoff = fs::metadata(&base).unwrap().len();
    // Only the read of the byte before the cutoff fails.
    source::fail_reads_of(&base, cutoff - 1, libc::EIO);
    let mut backlog = LaunchBacklog::starting();
    let failing = drain_launches(&mut fixture, &mut backlog, 10);
    source::fail_reads(None);
    assert_eq!(failing.published_invalid, 0);
    assert!(failing.threads_busy > 0, "retried");
    assert_eq!(relations(&fixture.store), 0);
    drain_launches(&mut fixture, &mut backlog, 100);
    assert_eq!(group_status(&fixture), GroupStatus::Valid);
    assert_eq!(relations(&fixture.store), 1);
}

/// A skipped history is counted once, as not searched, even when it holds
/// a `codex exec` launch with no first result.
#[test]
fn a_skipped_history_is_not_also_counted_as_a_launch_without_result() {
    let mut world = World::new();
    let day = world.fixture.home.join(".codex/sessions/2026/10/01");
    let launcher = day.join(format!("rollout-2026-10-01T10-00-00-{LAUNCHER}.jsonl"));
    let mut note = row(50, json!({"type": "event_msg", "text": "later"}));
    note["ordinal"] = json!(2);
    let head = [codex_header(LAUNCHER, -10_000, "vscode"), exec_call(0, 1)];
    fs::write(&launcher, rows_text(&head)).unwrap();
    world.passes(30);
    world.wait();
    world.passes(30);
    assert_eq!(world.gaps().exec_launches_without_result, 1);
    // It grows, and every read of it now fails.
    fs::write(
        &launcher,
        rows_text(&[head[0].clone(), head[1].clone(), note]),
    )
    .unwrap();
    source::fail_reads_of(&launcher, 0, libc::EIO);
    world.spawns.launches.add_threads([LAUNCHER]);
    world.passes(30);
    world.wait();
    world.passes(30);
    let gaps = world.gaps();
    source::fail_reads(None);
    assert_eq!((gaps.unsearched, gaps.exec_launches_without_result), (1, 0));
}
