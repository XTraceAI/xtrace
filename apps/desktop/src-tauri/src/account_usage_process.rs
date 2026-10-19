//! Small process readers with fixed bounds. stderr is discarded because it may
//! include command or provider data; neither argv nor logs contain a token.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) enum ProcessFailure {
    Spawn,
    Timeout,
    OutputTooLarge,
    Exit,
}

pub(super) fn run_to_exit(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    max_output: usize,
    group: bool,
) -> Result<Vec<u8>, ProcessFailure> {
    // Put outer provider helpers in their own group. If a bounded helper
    // times out, its curl child must end with it rather than outlive XTrace.
    #[cfg(unix)]
    if group {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ProcessFailure::Spawn)?;
    let Some(stdout) = child.stdout.take() else {
        kill_tree(&mut child, group);
        let _ = child.wait();
        return Err(ProcessFailure::Spawn);
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut stdout = stdout;
        let mut chunk = [0u8; 4096];
        let mut oversized = false;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    if output.len().saturating_add(count) > max_output {
                        oversized = true;
                        break;
                    }
                    output.extend_from_slice(&chunk[..count]);
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(if oversized {
            Err(ProcessFailure::OutputTooLarge)
        } else {
            Ok(output)
        });
    });
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(input).is_err()
    {
        kill_tree(&mut child, group);
        let _ = child.wait();
        let _ = rx.recv_timeout(Duration::from_secs(1));
        return Err(ProcessFailure::Exit);
    }
    let deadline = Instant::now() + timeout;
    let mut output = None;
    let failure = loop {
        if let Ok(result) = rx.try_recv() {
            output = Some(result);
        }
        if matches!(output, Some(Err(_))) {
            break None;
        }
        // Do not reap the leader before signalling its group. Even after
        // exit, its reserved PID keeps the group identity safe to use.
        #[cfg(unix)]
        let exited = if group {
            exited_unreaped(&child)
        } else {
            child.try_wait().map(|status| status.is_some())
        };
        #[cfg(not(unix))]
        let exited = child.try_wait().map(|status| status.is_some());
        match exited {
            Ok(true) => break None,
            Err(_) => break Some(ProcessFailure::Exit),
            Ok(false) => {}
        }
        if Instant::now() >= deadline {
            break Some(ProcessFailure::Timeout);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Every completion, including a successful leader exit, owns this
    // cleanup. Descendants must not continue a finished helper's work.
    kill_tree(&mut child, group);
    let status = child.wait();
    if output.is_none() {
        // A descendant that escaped the group may retain stdout; never
        // join its drain thread without a bound.
        output = rx.recv_timeout(Duration::from_secs(1)).ok();
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    let output = output.ok_or(ProcessFailure::Exit)??;
    let status = status.map_err(|_| ProcessFailure::Exit)?;
    if !status.success() && output.is_empty() {
        return Err(ProcessFailure::Exit);
    }
    Ok(output)
}

#[cfg(unix)]
fn exited_unreaped(child: &std::process::Child) -> std::io::Result<bool> {
    // SAFETY: siginfo is a valid zeroed out-parameter. waitid inspects only
    // our child and WNOWAIT keeps its PID reserved until group cleanup.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let exited = unsafe { info.si_pid() } != 0;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let exited = info.si_pid != 0;
    Ok(exited)
}

fn kill_tree(child: &mut std::process::Child, group: bool) {
    #[cfg(unix)]
    if group && let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: grouped children are never reaped before this call; the
        // unreaped leader reserves the group ID against reuse.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn early_leader_exit_cleans_descendants_with_inherited_and_private_pipes() {
        for private_pipes in [false, true] {
            for output in ["", "partial result"] {
                let root = tempfile::tempdir().unwrap();
                let pid_file = root.path().join("child.pid");
                let mut command = Command::new("python3");
                command
                    .args([
                        "-c",
                        r#"
import os, pathlib, subprocess, sys
child = subprocess.Popen(
    [sys.executable, '-c', 'import time; time.sleep(10)'],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE if sys.argv[2] == 'true' else None,
    stderr=subprocess.DEVNULL)
pathlib.Path(sys.argv[1]).write_text(str(child.pid))
print(sys.argv[3], end='', flush=True)
os._exit(1)
"#,
                    ])
                    .arg(&pid_file)
                    .arg(private_pipes.to_string())
                    .arg(output);
                let started = Instant::now();
                let result = run_to_exit(&mut command, b"", Duration::from_secs(2), 1024, true);
                if output.is_empty() {
                    assert!(matches!(result, Err(ProcessFailure::Exit)), "{result:?}");
                } else {
                    // Existing helper behavior accepts useful output even
                    // when the helper's exit status is unsuccessful.
                    assert_eq!(result.unwrap(), output.as_bytes());
                }
                assert!(started.elapsed() < Duration::from_secs(2));
                let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                // SAFETY: signal zero only checks the synthetic child's PID.
                while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert_ne!(
                    unsafe { libc::kill(pid, 0) },
                    0,
                    "descendant survived cleanup: {pid}"
                );
            }
        }
    }

    #[test]
    fn stalled_helper_is_timed_out() {
        let mut command = Command::new("/bin/sleep");
        command.arg("10");
        let started = Instant::now();
        assert!(matches!(
            run_to_exit(&mut command, b"", Duration::from_millis(100), 1024, true),
            Err(ProcessFailure::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bounded_reader_accepts_small_output_and_rejects_large_output() {
        let mut echo = Command::new("/bin/cat");
        let output = run_to_exit(&mut echo, b"small", Duration::from_secs(1), 16, true).unwrap();
        assert_eq!(output, b"small");

        let mut echo = Command::new("/bin/cat");
        let failure = run_to_exit(
            &mut echo,
            b"larger than cap",
            Duration::from_secs(1),
            4,
            true,
        );
        assert!(matches!(failure, Err(ProcessFailure::OutputTooLarge)));
    }
}
