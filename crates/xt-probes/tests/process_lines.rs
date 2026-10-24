//! Watching a child's stdout line by line while it runs
//! ([`xt_probes::process::collect_with`]).
#![cfg(unix)]

use std::{
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_probes::process::{Bounds, MAX_OBSERVED_LINE_BYTES, collect, collect_with, spawn_grouped};

fn bounds(max_stdout_bytes: usize) -> Bounds {
    Bounds {
        deadline: Duration::from_secs(30),
        max_stdout_bytes,
        max_stderr_bytes: 1024,
        stdout_overflow_ends: false,
        exit_grace: Some(Duration::from_secs(1)),
        term_grace: None,
    }
}

fn shell(script: &str) -> std::process::Child {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn_grouped(&mut command).unwrap()
}

type Seen = Arc<Mutex<Vec<(Vec<u8>, Instant)>>>;

fn observer(seen: &Seen) -> xt_probes::process::LineObserver {
    let seen = Arc::clone(seen);
    Box::new(move |line: &[u8]| seen.lock().unwrap().push((line.to_vec(), Instant::now())))
}

#[test]
fn lines_are_observed_as_they_are_written_and_retention_is_unchanged() {
    let seen: Seen = Arc::default();
    // The first line is written in two pieces; the second only after a pause.
    let child = shell("printf 'fir'; sleep 0.2; printf 'st\\n'; sleep 0.6; printf 'second\\nlast'");
    let finished = collect_with(child, bounds(8), None, Some(observer(&seen))).unwrap();
    let ended = Instant::now();
    assert!(finished.status.success());
    // Only the bound is retained, exactly as without an observer.
    assert_eq!(finished.stdout.bytes, b"first\nse");
    assert!(finished.stdout.overflowed);
    let seen = seen.lock().unwrap();
    let lines: Vec<&[u8]> = seen.iter().map(|(line, _)| line.as_slice()).collect();
    assert_eq!(lines, [&b"first"[..], b"second", b"last"]);
    // The first line was seen while the child was still running.
    assert!(ended.duration_since(seen[0].1) >= Duration::from_millis(400));
}

#[test]
fn a_line_over_the_observed_bound_is_skipped() {
    let seen: Seen = Arc::default();
    let long = MAX_OBSERVED_LINE_BYTES + 1;
    let script = format!(
        "printf 'before\\n'; head -c {long} /dev/zero | tr '\\0' 'x'; printf '\\nafter\\n'"
    );
    let finished = collect_with(
        shell(&script),
        bounds(1024 * 1024),
        None,
        Some(observer(&seen)),
    )
    .unwrap();
    assert!(finished.status.success());
    assert_eq!(
        finished.stdout.bytes.len(),
        long + "before\n\nafter\n".len()
    );
    let seen = seen.lock().unwrap();
    let lines: Vec<&[u8]> = seen.iter().map(|(line, _)| line.as_slice()).collect();
    assert_eq!(lines, [&b"before"[..], b"after"]);
}

#[test]
fn collect_without_an_observer_is_unchanged() {
    let finished = collect(shell("printf 'a\\nb\\n'"), bounds(1024), None).unwrap();
    assert_eq!(finished.stdout.bytes, b"a\nb\n");
    assert!(!finished.stdout.overflowed);
}
