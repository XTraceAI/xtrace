//! One bounded child process, contained in its own process group.
//!
//! This is the one place a probe or a judge starts, awaits and, when a bound
//! is reached, ends an external command. The GitHub CLI client
//! ([`crate::gh`]) and the grading runner (`xt-judge`) both use it.
//!
//! * The child is the leader of its own process group ([`spawn_grouped`]), so
//!   a deadline, a cancellation or an oversized stdout terminates everything
//!   it started, and reaps it. A descendant that inherited the pipes cannot
//!   keep the run alive. With [`Bounds::exit_grace`] set, the group is also
//!   ended when the leader finishes normally, so nothing it started outlives
//!   the run.
//! * Both output streams are separately bounded and drained concurrently, so
//!   a child that floods one of them can neither exhaust memory here nor
//!   block on a full pipe. Bytes past a bound are read and dropped.
//! * A caller may watch stdout line by line while the child runs
//!   ([`collect_with`]): each complete line, up to [`MAX_OBSERVED_LINE_BYTES`],
//!   is handed to its observer from the stdout drain. A longer line is
//!   skipped up to its newline and never observed. This changes nothing
//!   about what is retained.
//! * The child's exit is observed without being consumed, so the leader stays
//!   unreaped until the run is decided: its pid is what reserves the ID of
//!   the group a termination signals, and it is given up exactly once,
//!   afterwards.
//!
//! Only Unix hosts are supported: the guarantee is built on process groups.
//! Everything except [`CancelFlag`] exists only there.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Cancels a run from another thread. A cancel before the spawn refuses to
/// start the child; a cancel while it runs kills its process group and reaps
/// it. Cancellation is permanent for the flag.
///
/// This is deliberately smaller than the ingest reader's `CancelToken`: a run
/// awaits its own single child in the calling thread, so a flag the wait loop
/// polls is enough and no registry of live children is needed.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Longest stdout line handed to a [`LineObserver`], without its newline. A
/// longer line is read and dropped up to its newline, and not observed.
pub const MAX_OBSERVED_LINE_BYTES: usize = 256 * 1024;

/// Called from the stdout drain with each complete line, without its
/// newline, in order; a last line with no newline is handed over when the
/// stream ends. It runs on the drain's own thread, so it must return quickly:
/// while it runs, the child's stdout is not read.
pub type LineObserver = Box<dyn FnMut(&[u8]) + Send>;

/// Splits a byte stream into lines for a [`LineObserver`], holding at most
/// `cap` bytes of the line being read.
#[cfg_attr(not(unix), allow(dead_code))]
struct Lines {
    observer: LineObserver,
    cap: usize,
    line: Vec<u8>,
    /// The line being read passed `cap`: it is dropped up to its newline.
    skipping: bool,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl Lines {
    fn new(observer: LineObserver, cap: usize) -> Self {
        Self {
            observer,
            cap,
            line: Vec::new(),
            skipping: false,
        }
    }

    fn feed(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let end = bytes.iter().position(|&byte| byte == b'\n');
            let piece = &bytes[..end.unwrap_or(bytes.len())];
            if !self.skipping {
                if self.line.len() + piece.len() > self.cap {
                    self.skipping = true;
                    self.line = Vec::new();
                } else {
                    self.line.extend_from_slice(piece);
                }
            }
            let Some(end) = end else {
                return;
            };
            self.end_line();
            bytes = &bytes[end + 1..];
        }
    }

    fn end_line(&mut self) {
        if !self.skipping {
            (self.observer)(&self.line);
        }
        self.line.clear();
        self.skipping = false;
    }

    /// The stream ended: a last line without a newline is still a line.
    fn finish(&mut self) {
        if self.skipping || !self.line.is_empty() {
            self.end_line();
        }
    }
}

#[cfg(unix)]
pub use unix::{
    Bounds, Captured, DRAIN_GRACE, Ended, Finished, collect, collect_with, exit_observed,
    spawn_grouped,
};

#[cfg(unix)]
mod unix {
    use super::{CancelFlag, LineObserver, Lines, MAX_OBSERVED_LINE_BYTES};
    use std::{
        io::Read,
        process::{Child, Command, ExitStatus},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    /// How often the run is polled while the child runs.
    const POLL: Duration = Duration::from_millis(10);
    /// How long a terminated group is given to close the pipes it holds
    /// before its drains are abandoned. The child itself is always reaped.
    pub const DRAIN_GRACE: Duration = Duration::from_secs(5);
    /// Read size; unrelated to the retained bounds.
    const CHUNK: usize = 8 * 1024;

    /// The bounds of one run.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Bounds {
        /// From spawn to exited and drained.
        pub deadline: Duration,
        /// Stdout bytes kept; more are read and dropped.
        pub max_stdout_bytes: usize,
        /// Stderr bytes kept; more are read and dropped.
        pub max_stderr_bytes: usize,
        /// Whether stdout past its bound ends the run as
        /// [`Ended::OutputTooLarge`] (a truncated response is unusable), or is
        /// only marked [`Captured::overflowed`].
        pub stdout_overflow_ends: bool,
        /// After the leader exited, how long a descendant may keep the pipes
        /// open before the group is ended and the run finishes with the
        /// leader's exit status. `None` waits for the drains until the
        /// deadline.
        ///
        /// `Some` also means the leader's exit decides the run: the group is
        /// killed before the final reap on **every** path, a normal finish
        /// included, so no member left behind (one that closed or redirected
        /// its pipes, say) can still act after the caller has judged the run.
        /// With `None` a normal finish reaps the leader and signals nothing.
        pub exit_grace: Option<Duration>,
        /// How the group is ended. `None` sends `SIGKILL` at once. `Some`
        /// first sends `SIGTERM` to the whole group, gives the leader up to
        /// this long to exit on its own, then sends `SIGKILL` to the group
        /// anyway, so a member that ignores `SIGTERM` still goes. The leader
        /// stays unreaped throughout, so the group ID stays this run's.
        pub term_grace: Option<Duration>,
    }

    /// One drained stream.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Captured {
        /// The first bytes, up to the bound.
        pub bytes: Vec<u8>,
        /// More was written than the bound kept.
        pub overflowed: bool,
    }

    /// A run whose child exited on its own.
    #[derive(Debug)]
    pub struct Finished {
        /// The leader's exit status, from its one reap.
        pub status: ExitStatus,
        pub stdout: Captured,
        pub stderr: Captured,
    }

    /// Why a run was ended. In every case the child's whole group was killed
    /// and the leader reaped.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Ended {
        Timeout,
        Cancelled,
        /// Stdout passed its bound and [`Bounds::stdout_overflow_ends`].
        OutputTooLarge,
        /// The child's pipes or exit could not be observed.
        Unobservable,
    }

    /// Start the child as the leader of its own process group, so a deadline
    /// or cancel can terminate everything it spawned.
    pub fn spawn_grouped(command: &mut Command) -> std::io::Result<Child> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        command.spawn()
    }

    /// Whether the child has terminated, observed **without reaping it**.
    ///
    /// This is the whole reason a run does not poll with `try_wait`: that
    /// reaps the leader the moment it exits, which releases its pid, and the
    /// pid of a group leader is what reserves that group's ID. A launcher
    /// that exits while a descendant still holds the pipes would then leave
    /// this run holding an ID it no longer owns. `WNOWAIT` leaves the leader
    /// waitable — a zombie the kernel keeps, and a member of its own group —
    /// so the ID stays reserved until [`terminate`] or the final reap gives
    /// it up, exactly once.
    pub fn exit_observed(pid: u32) -> std::io::Result<bool> {
        let id = libc::id_t::try_from(pid)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        // Zeroed first: with `WNOHANG` a report of no state change is a zero
        // `si_pid`, and not every implementation writes that zero itself.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid, initialized siginfo_t for this call, and
        // the options are the POSIX non-blocking, non-consuming pair.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                id,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            return match error.raw_os_error() {
                // An interrupted observation is simply not an observation.
                Some(libc::EINTR) => Ok(false),
                _ => Err(error),
            };
        }
        Ok(reported_pid(&info) != 0)
    }

    /// `si_pid` is a plain field on the BSDs and an accessor on Linux.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn reported_pid(info: &libc::siginfo_t) -> libc::pid_t {
        // SAFETY: the union is read as the child-status member the `WEXITED`
        // report fills, and as the zeroes it was initialized with otherwise.
        unsafe { info.si_pid() }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn reported_pid(info: &libc::siginfo_t) -> libc::pid_t {
        info.si_pid
    }

    /// Kill every process in the group the child leads, then reap the leader
    /// exactly once, returning its status.
    ///
    /// This is only ever reached with the leader still unreaped: running, or
    /// exited and kept waitable by [`exit_observed`]. Its pid is therefore
    /// still allocated and still names this group, so `-pid` cannot reach any
    /// other group, and the group is never empty at this point — even a
    /// zombie leader is a member of it. The signal is sent deliberately when
    /// the leader has already exited, because that is exactly the case where
    /// a descendant is still holding the pipes and terminating the leader
    /// alone would leave it running. The reap comes after the signal, and is
    /// the only one: it is what finally releases the pid and the group ID.
    ///
    /// With `term_grace`, the group is first sent `SIGTERM` and the leader is
    /// given that long to exit (observed without reaping it); `SIGKILL`
    /// follows either way, so nothing that ignored the request survives.
    fn terminate(pid: u32, child: &mut Child, term_grace: Option<Duration>) -> Option<ExitStatus> {
        if let Ok(group) = i32::try_from(pid) {
            if let Some(grace) = term_grace {
                // SAFETY: a plain signal to the unreaped child's own group.
                unsafe {
                    libc::kill(-group, libc::SIGTERM);
                }
                let deadline = Instant::now() + grace;
                while Instant::now() < deadline && !matches!(exit_observed(pid), Ok(true) | Err(_))
                {
                    std::thread::sleep(POLL);
                }
            }
            // SAFETY: a plain signal to the unreaped child's own group.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        child.wait().ok()
    }

    /// One bounded, concurrently drained stream. The thread reads to the end
    /// whatever the bound, so the child can never block on a full pipe;
    /// bytes past the bound are dropped instead of retained.
    struct Drain {
        handle: Option<JoinHandle<Vec<u8>>>,
        overflowed: Arc<AtomicBool>,
    }

    impl Drain {
        fn spawn(
            mut stream: impl Read + Send + 'static,
            limit: usize,
            observer: Option<LineObserver>,
        ) -> Self {
            let overflowed = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&overflowed);
            let handle = std::thread::spawn(move || {
                let mut lines =
                    observer.map(|observer| Lines::new(observer, MAX_OBSERVED_LINE_BYTES));
                let mut kept: Vec<u8> = Vec::new();
                let mut buffer = [0u8; CHUNK];
                let mut total: usize = 0;
                loop {
                    match stream.read(&mut buffer) {
                        // A read error ends this stream exactly as its end does.
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if let Some(lines) = lines.as_mut() {
                                lines.feed(&buffer[..read]);
                            }
                            total = total.saturating_add(read);
                            if total > limit {
                                flag.store(true, Ordering::SeqCst);
                            }
                            let room = limit.saturating_sub(kept.len());
                            kept.extend_from_slice(&buffer[..read.min(room)]);
                        }
                    }
                }
                if let Some(lines) = lines.as_mut() {
                    lines.finish();
                }
                kept
            });
            Self {
                handle: Some(handle),
                overflowed,
            }
        }

        fn overflowed(&self) -> bool {
            self.overflowed.load(Ordering::SeqCst)
        }

        fn finished(&self) -> bool {
            self.handle
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished)
        }

        fn take(&mut self) -> Captured {
            Captured {
                overflowed: self.overflowed(),
                bytes: self
                    .handle
                    .take()
                    .and_then(|handle| handle.join().ok())
                    .unwrap_or_default(),
            }
        }
    }

    /// Await the child and its drains within the bounds. Returns how the
    /// child exited and what it wrote; a bound that ended the run returns its
    /// reason instead, with the group terminated and the child reaped.
    pub fn collect(
        child: Child,
        bounds: Bounds,
        cancel: Option<&CancelFlag>,
    ) -> Result<Finished, Ended> {
        collect_with(child, bounds, cancel, None)
    }

    /// [`collect`], with each stdout line also handed to `on_stdout_line` as
    /// the child writes it ([`LineObserver`]). The bounds and what is
    /// retained are exactly [`collect`]'s; a line longer than
    /// [`MAX_OBSERVED_LINE_BYTES`] is not observed.
    pub fn collect_with(
        mut child: Child,
        bounds: Bounds,
        cancel: Option<&CancelFlag>,
        on_stdout_line: Option<LineObserver>,
    ) -> Result<Finished, Ended> {
        // The leader's pid is also its process group's ID. Nothing below
        // reaps it — the poll observes its exit without consuming it — so
        // this ID stays reserved until the single reap at the end.
        let pid = child.id();
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
            terminate(pid, &mut child, bounds.term_grace);
            return Err(Ended::Unobservable);
        };
        let mut stdout = Drain::spawn(stdout, bounds.max_stdout_bytes, on_stdout_line);
        let mut stderr = Drain::spawn(stderr, bounds.max_stderr_bytes, None);
        let deadline = Instant::now() + bounds.deadline;
        let mut exited_at: Option<Instant> = None;
        let mut ended: Option<Ended> = None;
        let mut outlived = false;
        loop {
            if exited_at.is_none() {
                match exit_observed(pid) {
                    Ok(true) => exited_at = Some(Instant::now()),
                    Ok(false) => {}
                    Err(_) => {
                        ended = Some(Ended::Unobservable);
                        break;
                    }
                }
            }
            // A response past the bound is truncated, so it is unusable even
            // if the rest of the run completed.
            if bounds.stdout_overflow_ends && stdout.overflowed() {
                ended = Some(Ended::OutputTooLarge);
                break;
            }
            // Finished means exited *and* drained: a child that exited while
            // a descendant still holds the pipes is not done, and its exit
            // has not been consumed, so the group can still be terminated.
            if exited_at.is_some() && stdout.finished() && stderr.finished() {
                break;
            }
            if cancel.is_some_and(CancelFlag::is_cancelled) {
                ended = Some(Ended::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                ended = Some(Ended::Timeout);
                break;
            }
            if let (Some(at), Some(grace)) = (exited_at, bounds.exit_grace)
                && at.elapsed() >= grace
            {
                outlived = true;
                break;
            }
            std::thread::sleep(POLL);
        }
        if let Some(reason) = ended {
            // The whole group goes, leader exited or not, and only then is
            // the leader reaped; a killed group's readers reach the end of
            // their pipes, and are abandoned only if they do not.
            terminate(pid, &mut child, bounds.term_grace);
            abandon_after(DRAIN_GRACE, [stdout, stderr]);
            return Err(reason);
        }
        let status = if outlived {
            // The leader exited on its own but a descendant kept the pipes:
            // end the group, and finish with the leader's own exit status.
            let status = terminate(pid, &mut child, bounds.term_grace);
            wait_for(DRAIN_GRACE, &[&stdout, &stderr]);
            status
        } else if bounds.exit_grace.is_some() {
            // The leader exited and every pipe is closed, but a member that
            // let go of its pipes may still be running. The leader is still
            // unreaped, so the group ID is still this run's: end the group,
            // then reap, and finish with the leader's own exit status.
            terminate(pid, &mut child, bounds.term_grace)
        } else {
            // Nothing is signalled: the child exited and every writer has
            // closed its pipe. This is the one reap of the whole run.
            child.wait().ok()
        };
        let Some(status) = status else {
            abandon_after(DRAIN_GRACE, [stdout, stderr]);
            return Err(Ended::Unobservable);
        };
        let stdout = if stdout.finished() {
            stdout.take()
        } else {
            Captured::default()
        };
        let stderr = if stderr.finished() {
            stderr.take()
        } else {
            Captured::default()
        };
        Ok(Finished {
            status,
            stdout,
            stderr,
        })
    }

    fn wait_for(grace: Duration, drains: &[&Drain]) {
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline && !drains.iter().all(|drain| drain.finished()) {
            std::thread::sleep(POLL);
        }
    }

    /// Wait out the drains of a terminated group, then let them go. Only a
    /// descendant that left the group on its own can still hold a pipe here;
    /// its reader keeps a bounded buffer and ends when that pipe closes.
    fn abandon_after(grace: Duration, drains: [Drain; 2]) {
        wait_for(grace, &[&drains[0], &drains[1]]);
        for mut drain in drains {
            if drain.finished() {
                drop(drain.take());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lines;
    use std::sync::{Arc, Mutex};

    fn split(chunks: &[&[u8]], cap: usize) -> Vec<Vec<u8>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let mut lines = Lines::new(
            Box::new(move |line: &[u8]| sink.lock().unwrap().push(line.to_vec())),
            cap,
        );
        for chunk in chunks {
            lines.feed(chunk);
        }
        lines.finish();
        drop(lines);
        Arc::try_unwrap(seen).unwrap().into_inner().unwrap()
    }

    #[test]
    fn lines_split_across_reads_are_joined() {
        assert_eq!(
            split(&[b"ab", b"c\nde", b"f\n\ng"], 64),
            [
                b"abc".to_vec(),
                b"def".to_vec(),
                b"".to_vec(),
                b"g".to_vec()
            ]
        );
        assert_eq!(split(&[b"one\n"], 64), [b"one".to_vec()]);
        assert!(split(&[], 64).is_empty());
    }

    #[test]
    fn a_line_over_the_cap_is_skipped_up_to_its_newline() {
        // Exactly the cap is kept; one more byte drops the whole line, even
        // when it arrives in pieces, and the next line is read normally.
        assert_eq!(
            split(&[b"1234\n12", b"345", b"67\nok\n"], 4),
            [b"1234".to_vec(), b"ok".to_vec()]
        );
        // A last line over the cap with no newline is not observed either.
        assert_eq!(split(&[b"ok\n123456"], 4), [b"ok".to_vec()]);
    }
}
