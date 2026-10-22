//! What a normal finish does to a group member the leader left behind, for
//! both configurations of [`xt_probes::process::collect`]: the GitHub CLI's
//! (`exit_grace: None`, nothing signalled) and the grading runner's
//! (`exit_grace: Some`, the group is killed before the final reap).
#![cfg(unix)]
//!
//! The leader is `/bin/sh`; the member it leaves behind redirects its output
//! to `/dev/null`, so the pipes close as soon as the leader exits and the
//! run finishes normally while the member is still sleeping.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use xt_probes::process::{Bounds, collect, spawn_grouped};

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("group-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

/// Whether any process is in the group `pgid`.
fn group_alive(pgid: u32) -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-g", &pgid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn until(limit: Duration, check: impl Fn() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    check()
}

/// Starts the leader: it leaves a member that writes `late.txt` after
/// `delay` seconds, prints `ok` and exits 0.
fn leave_a_member(root: &Path, delay: u32, exit_grace: Option<Duration>) -> u32 {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "(sleep {delay}; echo late > \"$DIR/late.txt\") >/dev/null 2>&1 </dev/null &\necho ok"
        ))
        .env("DIR", root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = spawn_grouped(&mut command).unwrap();
    let pgid = child.id();
    let bounds = Bounds {
        deadline: Duration::from_secs(30),
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
        stdout_overflow_ends: false,
        exit_grace,
        term_grace: None,
    };
    let finished = collect(child, bounds, None).unwrap();
    assert!(finished.status.success());
    assert_eq!(finished.stdout.bytes, b"ok\n");
    pgid
}

#[test]
fn with_an_exit_grace_a_normal_finish_kills_the_group_first() {
    let (_dir, root) = scratch();
    let started = Instant::now();
    let pgid = leave_a_member(&root, 2, Some(Duration::from_secs(10)));
    assert!(started.elapsed() < Duration::from_secs(2), "not held open");
    assert!(
        until(Duration::from_secs(5), || !group_alive(pgid)),
        "no member of the run's group is alive"
    );
    std::thread::sleep(Duration::from_secs(3));
    assert!(!root.join("late.txt").exists(), "the member never wrote");
}

#[test]
fn without_an_exit_grace_a_normal_finish_signals_nothing() {
    let (_dir, root) = scratch();
    let pgid = leave_a_member(&root, 1, None);
    // GitHub CLI behaviour, unchanged: the member is not signalled.
    assert!(group_alive(pgid), "the member is left running");
    assert!(
        until(Duration::from_secs(10), || root.join("late.txt").exists()),
        "the member finished on its own"
    );
}

/// Starts a leader that records `SIGTERM` in `term.txt` and then either exits
/// (`honour`) or keeps running until killed, and cancels it once it is ready.
fn cancel_with_term_grace(root: &Path, honour: bool, grace: Duration) -> (Duration, u32) {
    let on_term = if honour {
        "echo term > \"$DIR/term.txt\"; exit 0"
    } else {
        "echo term > \"$DIR/term.txt\""
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "trap '{on_term}' TERM; echo ready > \"$DIR/ready.txt\"; \
             while :; do sleep 0.05; done"
        ))
        .env("DIR", root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = spawn_grouped(&mut command).unwrap();
    let pgid = child.id();
    let cancel = xt_probes::process::CancelFlag::new();
    let trigger = cancel.clone();
    let ready = root.join("ready.txt");
    std::thread::spawn(move || {
        until(Duration::from_secs(10), || ready.exists());
        trigger.cancel();
    });
    let bounds = Bounds {
        deadline: Duration::from_secs(30),
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
        stdout_overflow_ends: false,
        exit_grace: Some(Duration::from_secs(1)),
        term_grace: Some(grace),
    };
    let started = Instant::now();
    let ended = collect(child, bounds, Some(&cancel)).unwrap_err();
    assert_eq!(ended, xt_probes::process::Ended::Cancelled);
    (started.elapsed(), pgid)
}

#[test]
fn a_term_grace_sends_sigterm_first_and_a_leader_that_honours_it_exits_early() {
    let (_dir, root) = scratch();
    let (took, pgid) = cancel_with_term_grace(&root, true, Duration::from_secs(5));
    assert_eq!(
        std::fs::read_to_string(root.join("term.txt")).unwrap(),
        "term\n"
    );
    assert!(took < Duration::from_secs(4), "{took:?}");
    assert!(until(Duration::from_secs(5), || !group_alive(pgid)));
}

#[test]
fn a_term_grace_ends_with_sigkill_when_sigterm_is_ignored() {
    let (_dir, root) = scratch();
    let (took, pgid) = cancel_with_term_grace(&root, false, Duration::from_millis(600));
    assert_eq!(
        std::fs::read_to_string(root.join("term.txt")).unwrap(),
        "term\n"
    );
    assert!(took >= Duration::from_millis(600), "{took:?}");
    assert!(until(Duration::from_secs(5), || !group_alive(pgid)));
}
