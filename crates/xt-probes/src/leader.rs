//! A process group's leader, recognised again later by its process ID *and*
//! its start time, so a group a previous run of the app left behind can be
//! stopped without ever signalling an unrelated process that reuses the ID.
//!
//! * [`GroupLeader::observe`] reads a process as it is now: its ID, its start
//!   time (macOS `proc_pidinfo`, Linux `/proc/<pid>/stat`), and only when it
//!   belongs to this user and still leads its own process group.
//! * [`GroupLeader::record`] and [`GroupLeader::parse`] write and read that
//!   identity as one short line, kept by the caller beside the run.
//! * [`GroupLeader::stop_group`] signals the group only when the process with
//!   that ID is still exactly the recorded one (same start time, still the
//!   group's leader, still this user's): `SIGTERM` to the whole group, up to
//!   a grace for the group to empty, then `SIGKILL` to whatever is left.
//!   Anything else (gone, reused by another process, no longer a leader)
//!   signals nothing.
//!
//! Only macOS and Linux can observe a process; elsewhere nothing is observed
//! and so nothing is ever signalled.

use std::time::{Duration, Instant};

/// The first word of a record, so a file of any other kind never parses.
const RECORD_TAG: &str = "xtrace-group-leader";
/// The record format's version.
const RECORD_VERSION: &str = "1";
/// Largest record [`GroupLeader::parse`] reads.
pub const MAX_RECORD_BYTES: usize = 128;
/// How often a stopping group is looked at.
const POLL: Duration = Duration::from_millis(20);

/// When a process started, as the operating system reports it: seconds and
/// microseconds since the epoch on macOS, clock ticks since boot on Linux.
/// Only ever compared with another reading on the same machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessStart {
    pub seconds: u64,
    pub fraction: u64,
}

/// A process that leads its own process group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupLeader {
    pub pid: u32,
    pub start: ProcessStart,
}

/// What [`GroupLeader::stop_group`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupStop {
    /// The recorded process is not running any more (gone, or its ID now
    /// names another process, or it no longer leads a group): nothing was
    /// signalled.
    NotRunning,
    /// The group was sent `SIGTERM` and emptied within the grace.
    Terminated,
    /// The group was still there after the grace and was sent `SIGKILL`.
    Killed,
}

impl GroupLeader {
    /// The process `pid` as it is now, when it belongs to this user and leads
    /// its own process group. `None` otherwise, or when it cannot be read.
    pub fn observe(pid: u32) -> Option<Self> {
        if pid <= 1 {
            return None;
        }
        let (start, pgid, uid) = os::read(pid)?;
        (pgid == pid && uid == crate::regular_file::effective_user_id())
            .then_some(Self { pid, start })
    }

    /// The identity as one line, for the caller to keep.
    pub fn record(&self) -> String {
        format!(
            "{RECORD_TAG} {RECORD_VERSION} {} {} {}\n",
            self.pid, self.start.seconds, self.start.fraction
        )
    }

    /// A line [`Self::record`] wrote; anything else is `None`.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
        let mut words = text.split(' ');
        if words.next()? != RECORD_TAG || words.next()? != RECORD_VERSION {
            return None;
        }
        let mut number = || -> Option<u64> {
            let word = words.next()?;
            // Plain decimal digits only: no sign, no space, no leading `+`.
            (!word.is_empty() && word.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| word.parse().ok())
                .flatten()
        };
        let pid = u32::try_from(number()?).ok()?;
        let start = ProcessStart {
            seconds: number()?,
            fraction: number()?,
        };
        if words.next().is_some() || pid <= 1 {
            return None;
        }
        Some(Self { pid, start })
    }

    /// Whether the process with this ID is still exactly this one.
    pub fn is_running(&self) -> bool {
        Self::observe(self.pid).as_ref() == Some(self)
    }

    /// Stop the group this process leads, only while it is still exactly
    /// this process: `SIGTERM` to the group, up to `grace` for the group to
    /// empty, then `SIGKILL` to whatever is left of it.
    ///
    /// The group ID cannot be given to a new process while the group has
    /// members, so the group is followed by looking, every few milliseconds,
    /// whether it still has any. Before the final `SIGKILL` the ID is read
    /// once more: when it names a process again that is not the recorded
    /// leader, nothing more is sent.
    #[cfg(unix)]
    pub fn stop_group(&self, grace: Duration) -> GroupStop {
        if !self.is_running() {
            return GroupStop::NotRunning;
        }
        let Ok(group) = i32::try_from(self.pid) else {
            return GroupStop::NotRunning;
        };
        // SAFETY: a plain signal to the group the recorded, still running
        // leader leads.
        unsafe {
            libc::kill(-group, libc::SIGTERM);
        }
        let deadline = Instant::now() + grace;
        loop {
            if !group_has_members(group) {
                return GroupStop::Terminated;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
        // The ID names a process that is not the recorded leader: it was
        // given to someone else, so the group is no longer this one's.
        if let Some((start, _, _)) = os::read(self.pid)
            && start != self.start
        {
            return GroupStop::Terminated;
        }
        // SAFETY: the group still has members and its ID was never released
        // since the leader was confirmed above.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        GroupStop::Killed
    }

    #[cfg(not(unix))]
    pub fn stop_group(&self, _: Duration) -> GroupStop {
        GroupStop::NotRunning
    }
}

/// Whether any process is still in the group `group`.
#[cfg(unix)]
fn group_has_members(group: i32) -> bool {
    // SAFETY: signal 0 only checks that the group exists and may be signalled.
    let result = unsafe { libc::kill(-group, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(target_os = "macos")]
mod os {
    use super::ProcessStart;

    /// Start time, process group and owner of `pid`.
    pub fn read(pid: u32) -> Option<(ProcessStart, u32, u32)> {
        let id = libc::c_int::try_from(pid).ok()?;
        // SAFETY: zeroed is a valid value for this plain C struct.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: `info` is a writable buffer of exactly `size` bytes for the
        // `PROC_PIDTBSDINFO` flavour.
        let written = unsafe {
            libc::proc_pidinfo(id, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size)
        };
        if written != size || info.pbi_pid != pid {
            return None;
        }
        Some((
            ProcessStart {
                seconds: info.pbi_start_tvsec,
                fraction: info.pbi_start_tvusec,
            },
            info.pbi_pgid,
            info.pbi_uid,
        ))
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::ProcessStart;
    use std::os::unix::fs::MetadataExt;

    /// Start time, process group and owner of `pid`, from `/proc`.
    pub fn read(pid: u32) -> Option<(ProcessStart, u32, u32)> {
        let path = format!("/proc/{pid}/stat");
        let uid = std::fs::metadata(format!("/proc/{pid}")).ok()?.uid();
        let text = std::fs::read_to_string(path).ok()?;
        // The command name is in parentheses and may hold spaces or `)`.
        let rest = &text[text.rfind(')')? + 1..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // After the name: state is field 3, so field N is index N - 3.
        let pgid = fields.get(5 - 3)?.parse().ok()?;
        let ticks = fields.get(22 - 3)?.parse().ok()?;
        Some((
            ProcessStart {
                seconds: ticks,
                fraction: 0,
            },
            pgid,
            uid,
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod os {
    use super::ProcessStart;

    pub fn read(_: u32) -> Option<(ProcessStart, u32, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: GroupLeader = GroupLeader {
        pid: 4242,
        start: ProcessStart {
            seconds: 1_760_000_000,
            fraction: 123_456,
        },
    };

    #[test]
    fn a_record_reads_back_exactly() {
        let record = SAMPLE.record();
        assert_eq!(record, "xtrace-group-leader 1 4242 1760000000 123456\n");
        assert_eq!(GroupLeader::parse(record.as_bytes()), Some(SAMPLE));
    }

    #[test]
    fn anything_but_a_whole_record_is_refused() {
        for bad in [
            "",
            "xtrace-group-leader 1 4242 1760000000 123456",
            "xtrace-group-leader 2 4242 1760000000 123456\n",
            "other 1 4242 1760000000 123456\n",
            "xtrace-group-leader 1 4242 1760000000\n",
            "xtrace-group-leader 1 4242 1760000000 123456 9\n",
            "xtrace-group-leader 1 -4242 1760000000 123456\n",
            "xtrace-group-leader 1 +4242 1760000000 123456\n",
            "xtrace-group-leader 1 1 1760000000 123456\n",
            "xtrace-group-leader 1 0 1760000000 123456\n",
            "xtrace-group-leader 1 99999999999 1760000000 123456\n",
            "xtrace-group-leader  1 4242 1760000000 123456\n",
        ] {
            assert_eq!(GroupLeader::parse(bad.as_bytes()), None, "{bad:?}");
        }
        let long = format!("{}{}", SAMPLE.record(), " ".repeat(MAX_RECORD_BYTES));
        assert_eq!(GroupLeader::parse(long.as_bytes()), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn this_process_is_observed_only_when_it_leads_its_group() {
        let pid = std::process::id();
        // SAFETY: `getpgrp` takes no argument and cannot fail.
        let leads = unsafe { libc::getpgrp() } == i32::try_from(pid).unwrap();
        assert_eq!(GroupLeader::observe(pid).is_some(), leads);
        // Process 1 is never observed, and neither is an ID no process has.
        assert_eq!(GroupLeader::observe(1), None);
        assert_eq!(GroupLeader::observe(0), None);
    }
}
