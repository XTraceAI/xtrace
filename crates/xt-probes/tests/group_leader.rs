//! Stopping a process group by its recorded leader, as the app does at its
//! next start for a grading run whose Claude Code outlived it: only the
//! exact recorded process's group is signalled, `SIGTERM` first and
//! `SIGKILL` after the grace for a group that ignores it.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use xt_probes::{
    leader::{GroupLeader, GroupStop, ProcessStart},
    process::spawn_grouped,
};

/// A shell leading its own group, with a sleeping member, that runs `trap`
/// first. Reaped on its own thread, as the system reaps an orphan, so a
/// stopped leader does not linger as this test's zombie.
fn group(trap: &str) -> (u32, std::thread::JoinHandle<()>) {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!("{trap}\nsleep 60 &\nwait"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child: Child = spawn_grouped(&mut command).unwrap();
    let pid = child.id();
    let reaper = std::thread::spawn(move || {
        let _ = child.wait();
    });
    // The member is started by the shell; wait until the group has two.
    assert!(until(Duration::from_secs(5), || members(pid) >= 2));
    (pid, reaper)
}

/// How many processes are in the group `pgid`.
fn members(pgid: u32) -> usize {
    let out = Command::new("/usr/bin/pgrep")
        .args(["-g", &pgid.to_string()])
        .stderr(Stdio::null())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count()
}

fn until(limit: Duration, check: impl Fn() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    check()
}

/// Kills what a failing case left behind.
fn cleanup(pid: u32) {
    // SAFETY: a plain signal to a group this test started.
    unsafe {
        libc::kill(-i32::try_from(pid).unwrap(), libc::SIGKILL);
    }
}

#[test]
fn a_group_that_honours_sigterm_is_terminated() {
    let (pid, reaper) = group("");
    let leader = GroupLeader::observe(pid).expect("a group leader of this user");
    let recorded = GroupLeader::parse(leader.record().as_bytes()).unwrap();
    assert_eq!(recorded, leader);
    let started = Instant::now();
    assert_eq!(
        recorded.stop_group(Duration::from_secs(3)),
        GroupStop::Terminated
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(members(pid), 0);
    reaper.join().unwrap();
}

#[test]
fn a_group_that_ignores_sigterm_is_killed_after_the_grace() {
    let (pid, reaper) = group("trap '' TERM");
    let leader = GroupLeader::observe(pid).unwrap();
    let started = Instant::now();
    assert_eq!(
        leader.stop_group(Duration::from_millis(400)),
        GroupStop::Killed
    );
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert!(until(Duration::from_secs(5), || members(pid) == 0));
    reaper.join().unwrap();
}

#[test]
fn a_record_whose_start_time_differs_signals_nothing() {
    let (pid, reaper) = group("trap '' TERM");
    let leader = GroupLeader::observe(pid).unwrap();
    for start in [
        ProcessStart {
            seconds: leader.start.seconds + 1,
            ..leader.start
        },
        ProcessStart {
            fraction: leader.start.fraction + 1,
            ..leader.start
        },
    ] {
        let other = GroupLeader { pid, start };
        assert!(!other.is_running());
        assert_eq!(
            other.stop_group(Duration::from_millis(100)),
            GroupStop::NotRunning
        );
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(members(pid), 2, "nothing was signalled");
    cleanup(pid);
    reaper.join().unwrap();
}

#[test]
fn a_process_that_does_not_lead_its_group_is_never_observed() {
    // An ordinary child: in this test's own group, so not a leader.
    let mut child = Command::new("/bin/sleep")
        .arg("60")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    assert_eq!(GroupLeader::observe(pid), None);
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn a_leader_that_is_gone_signals_nothing() {
    let (pid, reaper) = group("");
    let leader = GroupLeader::observe(pid).unwrap();
    cleanup(pid);
    reaper.join().unwrap();
    assert!(until(Duration::from_secs(5), || members(pid) == 0));
    assert_eq!(
        leader.stop_group(Duration::from_millis(100)),
        GroupStop::NotRunning
    );
}
