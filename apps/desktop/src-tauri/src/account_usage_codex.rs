//! The installed Codex CLI's read-only app-server account limits call.
use crate::account_usage::{available, failed, parse_codex, unavailable};
use crate::account_usage_dto::{AccountProviderUsage, AccountUsageIssue};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub(super) fn read() -> AccountProviderUsage {
    let Some(executable) = resolve_codex() else {
        return unavailable(AccountUsageIssue::SourceUnavailable);
    };
    let mut command = Command::new(executable);
    command.args(["app-server", "--stdio"]);
    let value = match exchange(&mut command, Duration::from_secs(8)) {
        Ok(value) => value,
        Err(AccountUsageIssue::Timeout) => return failed(AccountUsageIssue::Timeout),
        Err(AccountUsageIssue::InvalidResponse) => {
            return failed(AccountUsageIssue::InvalidResponse);
        }
        Err(issue) => return unavailable(issue),
    };
    match parse_codex(&value) {
        Ok(windows) => available(windows),
        Err(AccountUsageIssue::SourceUnavailable) => {
            unavailable(AccountUsageIssue::SourceUnavailable)
        }
        Err(issue) => failed(issue),
    }
}

const INITIALIZE: &str = "{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"xtrace\",\"version\":\"0.1.0\"}}}\n";
const RATE_LIMITS: &str = concat!(
    "{\"method\":\"initialized\",\"params\":{}}\n",
    "{\"id\":2,\"method\":\"account/rateLimits/read\",\"params\":{\"excludeResetCreditDetails\":true}}\n",
);
const MAX_OUTPUT: usize = 131_072;

enum Message {
    Json(Value),
    OutputTooLarge,
    End,
}

/// A private, read-only app-server exchange. Stdin remains open until id 2:
/// the installed CLI may drop queued work if it sees EOF before replying.
fn exchange(command: &mut Command, timeout: Duration) -> Result<Value, AccountUsageIssue> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
    let Some(stdout) = child.stdout.take() else {
        kill_group_and_reap(&mut child);
        return Err(AccountUsageIssue::SourceUnavailable);
    };
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || read_messages(stdout, tx));
    let deadline = Instant::now() + timeout;
    let outcome = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or(AccountUsageIssue::SourceUnavailable)?;
        stdin
            .write_all(INITIALIZE.as_bytes())
            .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
        let initialized = wait_for_id(&rx, 1, deadline)?;
        if initialized.get("error").is_some() {
            return Err(AccountUsageIssue::SourceUnavailable);
        }
        stdin
            .write_all(RATE_LIMITS.as_bytes())
            .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
        let reply = wait_for_id(&rx, 2, deadline)?;
        if reply.get("error").is_some() {
            return Err(AccountUsageIssue::SourceUnavailable);
        }
        Ok(reply)
    })();
    // The protocol exchange is complete or failed. Never leave a CLI process,
    // its descendants, or a reader thread attached to XTrace.
    kill_group_and_reap(&mut child);
    let _ = reader.join();
    outcome
}

fn wait_for_id(
    rx: &Receiver<Message>,
    id: i64,
    deadline: Instant,
) -> Result<Value, AccountUsageIssue> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or(AccountUsageIssue::Timeout)?;
        let message = rx.recv_timeout(left).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => AccountUsageIssue::Timeout,
            mpsc::RecvTimeoutError::Disconnected => AccountUsageIssue::SourceUnavailable,
        })?;
        match message {
            Message::Json(value) if value.get("id").and_then(Value::as_i64) == Some(id) => {
                return Ok(value);
            }
            Message::Json(_) => {}
            Message::OutputTooLarge => return Err(AccountUsageIssue::InvalidResponse),
            Message::End => return Err(AccountUsageIssue::SourceUnavailable),
        }
    }
}

fn read_messages(mut stdout: impl Read, tx: mpsc::Sender<Message>) {
    let mut total = 0usize;
    let mut line = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        total = total.saturating_add(count);
        if total > MAX_OUTPUT {
            let _ = tx.send(Message::OutputTooLarge);
            return;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    let _ = tx.send(Message::Json(value));
                }
                line.clear();
            } else {
                line.push(*byte);
            }
        }
    }
    let _ = tx.send(Message::End);
}

fn kill_group_and_reap(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = kill(-pid, 9);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The one Codex resolver ([`xt_probes::executable::resolve_codex`]), over
/// this process's `XTRACE_CODEX`, `PATH` and `HOME`.
fn resolve_codex() -> Option<PathBuf> {
    let explicit = std::env::var_os(xt_judge::diagnosis::agents::CODEX_ENV);
    let home = std::env::var_os("HOME");
    xt_probes::executable::resolve_codex(
        explicit.as_deref(),
        std::env::var_os("PATH").as_deref(),
        home.as_deref().map(Path::new),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_APP_SERVER: &str = r#"
import json, select, sys, time
assert json.loads(sys.stdin.readline())['method'] == 'initialize'
print(json.dumps({'id': 1, 'result': {}}), flush=True)
assert json.loads(sys.stdin.readline())['method'] == 'initialized'
assert json.loads(sys.stdin.readline())['method'] == 'account/rateLimits/read'
time.sleep(0.08)
# The prior adapter closed stdin immediately after sending all three lines.
# This fake server then discards the queued account read, like the live CLI.
if select.select([sys.stdin], [], [], 0)[0] and sys.stdin.read(1) == '':
    sys.exit(0)
print(json.dumps({'id': 2, 'result': {'rateLimits': {'primary': {'usedPercent': 29, 'windowDurationMins': 10080, 'resetsAt': 1791050717}, 'secondary': None}}}), flush=True)
"#;

    #[test]
    fn waits_for_initialize_then_async_rate_limits_reply_without_closing_stdin() {
        let mut command = Command::new("python3");
        command.args(["-u", "-c", FAKE_APP_SERVER]);
        let reply = exchange(&mut command, Duration::from_secs(2)).unwrap();
        let windows = parse_codex(&reply).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, 29.0);
        assert_eq!(windows[0].window, "Week");
    }

    #[cfg(unix)]
    #[test]
    fn override_reads_fake_account_limits_and_invalid_overrides_stop() {
        use crate::account_usage_dto::AccountUsageState;
        use std::{
            ffi::OsStr,
            os::unix::{fs::PermissionsExt, process::CommandExt},
        };

        const CHILD_RESULT: &str = "XTRACE_ACCOUNT_USAGE_TEST_RESULT";
        if let Some(expected) = std::env::var_os(CHILD_RESULT) {
            let resolved = resolve_codex();
            if expected == "available" {
                let fake = std::env::var_os(xt_judge::diagnosis::agents::CODEX_ENV).unwrap();
                // Check before read() so even an unfixed resolver cannot run a real CLI.
                assert_eq!(resolved.as_deref(), Some(Path::new(&fake)));
                let usage = read();
                assert_eq!(usage.state, AccountUsageState::Available);
                assert_eq!(usage.windows.len(), 1);
                assert_eq!(usage.windows[0].used_percent, 29.0);
                assert_eq!(usage.windows[0].window, "Week");
            } else {
                assert_eq!(resolved, None, "invalid override must not fall back");
                let usage = read();
                assert_eq!(usage.state, AccountUsageState::Unavailable);
                assert_eq!(usage.issue, Some(AccountUsageIssue::SourceUnavailable));
            }
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let codex_home = root.path().join("codex-home");
        let path = root.path().join("bin");
        for directory in [&home, &codex_home, &path] {
            std::fs::create_dir(directory).unwrap();
        }
        let python = xt_probes::executable::resolve_within(
            "python3",
            None,
            std::env::var_os("PATH").as_deref(),
            &[Path::new("/usr/bin")],
        )
        .unwrap();
        let fake = root.path().join("fake-codex");
        std::fs::write(
            &fake,
            format!("#!{} -u\n{FAKE_APP_SERVER}", python.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fallback = path.join("codex");
        std::fs::write(&fallback, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755)).unwrap();
        let non_executable = root.path().join("non-executable");
        std::fs::write(&non_executable, "not executable").unwrap();
        std::fs::set_permissions(&non_executable, std::fs::Permissions::from_mode(0o644)).unwrap();
        let missing = root.path().join("missing");
        for (expected, explicit) in [
            ("available", fake.as_os_str()),
            ("unavailable", OsStr::new("")),
            ("unavailable", OsStr::new("relative/codex")),
            ("unavailable", missing.as_os_str()),
            ("unavailable", non_executable.as_os_str()),
        ] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "account_usage_codex::tests::override_reads_fake_account_limits_and_invalid_overrides_stop",
                    "--nocapture",
                ])
                .env_clear()
                .env(CHILD_RESULT, expected)
                .env(xt_judge::diagnosis::agents::CODEX_ENV, explicit)
                .env("HOME", &home)
                .env("CODEX_HOME", &codex_home)
                .env("PATH", &path)
                .process_group(0);
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(12);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "override case {explicit:?} failed");
                    break;
                }
                if Instant::now() >= deadline {
                    kill_group_and_reap(&mut child);
                    panic!("override case {explicit:?} timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_and_reaps_fake_app_server() {
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let fake = r#"
import json, os, sys, time
with open(sys.argv[1], 'w') as out:
    out.write(str(os.getpid()))
assert json.loads(sys.stdin.readline())['method'] == 'initialize'
print(json.dumps({'id': 1, 'result': {}}), flush=True)
sys.stdin.readline()
sys.stdin.readline()
time.sleep(30)
"#;
        let mut command = Command::new("python3");
        command.args(["-u", "-c", fake]).arg(pid_file.path());
        let started = Instant::now();
        assert!(matches!(
            exchange(&mut command, Duration::from_millis(250)),
            Err(AccountUsageIssue::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid: i32 = std::fs::read_to_string(pid_file.path())
            .unwrap()
            .parse()
            .unwrap();
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        assert_eq!(
            unsafe { kill(pid, 0) },
            -1,
            "timed-out app-server process remained alive"
        );
    }
}
