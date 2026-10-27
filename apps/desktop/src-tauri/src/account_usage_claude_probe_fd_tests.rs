use super::{COLS, ROWS, Screen, pty};
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn inherited_copy(fd: i32, minimum: i32) -> OwnedFd {
    // SAFETY: duplicates a live owned test descriptor; the returned fd is new.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD, minimum) };
    assert!(duplicate >= minimum);
    // SAFETY: F_DUPFD returned a new descriptor owned by this test.
    unsafe { OwnedFd::from_raw_fd(duplicate) }
}

fn extra_pty() -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: valid output pointers; no termios, name or window size requested.
    assert_eq!(
        unsafe {
            libc::openpty(
                &raw mut master,
                &raw mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: openpty returned two descriptors this test owns.
    let ends = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&ends.0, &ends.1] {
        // SAFETY: changes only the close-on-exec flag of an owned test fd.
        assert_eq!(unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIOCLEX) }, 0);
    }
    ends
}

fn run_terminal(script: &str, input: Option<&[u8]>) -> String {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let mut session = pty::Session::spawn(
        Path::new("/bin/sh"),
        dir.path(),
        ROWS,
        COLS,
        &["-c", script],
    )
    .unwrap();
    eprintln!(
        "PTY test spawn: {:.3}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    if let Some(input) = input {
        session.write(input).unwrap();
    }
    let mut screen = Screen::new(ROWS as usize, COLS as usize);
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        match session.read(Duration::from_millis(50)) {
            pty::Read::Data(bytes) => screen.feed(&bytes),
            pty::Read::Closed => break,
            pty::Read::Idle if session.exited() => break,
            pty::Read::Idle => {}
        }
    }
    session.quit(Instant::now() + Duration::from_secs(2));
    screen.text()
}

fn assert_closed(fds: &[i32]) {
    // High test descriptors cannot collide with the shell's redirection scratch fd10.
    let list = fds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let text = run_terminal(
        &format!(
            "for fd in {list}; do (: <&$fd) 2>/dev/null && echo leaked-$fd; done; echo checked"
        ),
        None,
    );
    assert!(text.contains("checked"), "{text}");
    assert!(!text.contains("leaked-"), "{text}");
}

#[test]
fn inherited_file_and_raw_pty_ends_are_closed_after_exec() {
    let null = File::open("/dev/null").unwrap();
    let (master, slave) = extra_pty();
    let inherited = [
        inherited_copy(null.as_raw_fd(), 256),
        inherited_copy(master.as_raw_fd(), 256),
        inherited_copy(slave.as_raw_fd(), 256),
    ];
    assert_closed(&inherited.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>());
}

#[test]
fn high_fd_is_closed_after_lowering_only_the_child_soft_limit() {
    const CASE: &str = "XTRACE_TEST_PTY_HIGH_FD";
    if let Some(value) = std::env::var_os(CASE) {
        let fd: i32 = value.to_str().unwrap().parse().unwrap();
        let mut limits: libc::rlimit = unsafe { std::mem::zeroed() };
        // SAFETY: a valid output pointer for this isolated re-executed test child.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limits) },
            0
        );
        assert_eq!(limits.rlim_cur, 64);
        // SAFETY: observes the deliberately inherited synthetic /dev/null fd.
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, 0);
        assert_closed(&[fd]);
        return;
    }
    let mut before: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: valid output pointer, no process limit is changed here.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut before) },
        0
    );
    let null = File::open("/dev/null").unwrap();
    let high = inherited_copy(null.as_raw_fd(), 4096);
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("child.log");
    let output = File::create(&log).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "account_usage_claude_probe::fd_tests::high_fd_is_closed_after_lowering_only_the_child_soft_limit", "--nocapture"])
        .env(CASE, high.as_raw_fd().to_string()).stdout(Stdio::from(output.try_clone().unwrap())).stderr(Stdio::from(output));
    let child_limit = libc::rlimit {
        rlim_cur: 64,
        rlim_max: before.rlim_max,
    };
    // SAFETY: changes only the forked child's limits, before that child execs.
    unsafe {
        command.pre_exec(move || {
            if libc::setrlimit(libc::RLIMIT_NOFILE, &raw const child_limit) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned high-fd test child timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(log).unwrap()
    );
    let mut after: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: valid output pointer; verify the multithreaded parent was untouched.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut after) },
        0
    );
    assert_eq!(
        (after.rlim_cur, after.rlim_max),
        (before.rlim_cur, before.rlim_max)
    );
}

#[test]
fn intended_standard_streams_and_controlling_terminal_still_work() {
    let text = run_terminal(
        "test -t 0 && test -t 1 && test -t 2 || exit 7; (: </dev/tty) || exit 8; read value; printf 'stdout-%s\\n' \"$value\"; printf 'stderr-%s\\n' \"$value\" >&2; echo tty-ok",
        Some(b"synthetic-input\n"),
    );
    for marker in ["stdout-synthetic-input", "stderr-synthetic-input", "tty-ok"] {
        assert!(text.contains(marker), "{text}");
    }
}

#[test]
fn missing_executable_still_returns_not_found() {
    let directory = tempfile::tempdir().unwrap();
    let error = pty::Session::spawn(
        &directory.path().join("missing"),
        directory.path(),
        ROWS,
        COLS,
        &[],
    )
    .err()
    .expect("missing executable became a false successful spawn");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn descriptor_bound_covers_existing_high_fds_and_new_allocations() {
    assert_eq!(pty::descriptor_bound(64, 6400).unwrap(), 6400);
    assert_eq!(pty::descriptor_bound(184_320, 25).unwrap(), 184_320);
    for (ceiling, allocated) in [(0, 25), (-1, 25), (64, 0), (64, u32::MAX)] {
        assert!(pty::descriptor_bound(ceiling, allocated).is_err());
    }
}

#[test]
fn failure_after_marking_still_reaches_rusts_exec_error_pipe() {
    let bound = pty::inherited_fd_bound().unwrap();
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg("exit 0");
    // SAFETY: the callback performs only the child's scalar/fcntl sweep and
    // returns an OS error without closing Rust's error-report descriptor.
    unsafe {
        command.pre_exec(move || {
            pty::mark_inherited_fds(bound)?;
            Err(std::io::Error::from_raw_os_error(libc::EACCES))
        });
    }
    let error = command
        .spawn()
        .expect_err("pre_exec error became a false successful spawn");
    assert_eq!(error.raw_os_error(), Some(libc::EACCES));
}

#[test]
fn parallel_pty_and_ordinary_spawns_keep_the_helpers_descriptors_isolated() {
    let null = File::open("/dev/null").unwrap();
    let inherited = inherited_copy(null.as_raw_fd(), 256);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let fd = inherited.as_raw_fd();
            scope.spawn(move || {
                for _ in 0..4 {
                    assert_closed(&[fd]);
                }
            });
            scope.spawn(|| {
                for _ in 0..4 {
                    assert!(
                        Command::new("/bin/sh")
                            .args(["-c", "exit 0"])
                            .status()
                            .unwrap()
                            .success()
                    );
                }
            });
        }
    });
}
