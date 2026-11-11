//! Parent links for Claude sessions a Claude agent launched with `claude -p`
//! from its own `Bash` tool, matched by their exact prompt.
//!
//! An agent's real launch commands seldom name the new session, and their
//! output usually goes to a file or a background job, so the stricter
//! [`super`] child facts (which compare the printed answer) cannot see them.
//! A false link is worse than none. This match links a child only when all
//! of these hold:
//!
//! 1. A caller's saved Claude transcript — its own file, or one of its helper
//!    agents' files under `<session>/subagents/` — holds an assistant record
//!    of that session with a `Bash` tool call whose command holds a
//!    create-mode `claude -p` (or `--print`) launch with a literal prompt
//!    ([`words`], read by the shared option map). Resume, continue and fork
//!    are not launches. A launch naming its session with `--session-id` may
//!    make only that session.
//! 2. The child is exactly one indexed Claude user session whose first
//!    eligible input (a user record that is not meta, sidechain or a tool
//!    result) is exactly that prompt, read from its own transcript and
//!    compared as a digest.
//! 3. The child's first record is dated at or after the call, and no more
//!    than an hour after it.
//! 4. Of the launches found that could have made the child, it belongs to
//!    the latest, the closest send before it (a retry of the same prompt
//!    keeps its own child); and that launch keeps no other indexed session.
//!    Launches tied for latest, or one that named the child, leave it
//!    unlinked.
//! 5. The parent, the session owning the transcript, is exactly one indexed
//!    Claude user session other than the child.
//! 6. The launch reads only the input its `Bash` call was given
//!    ([`words::Command::own_input`]): no `<` redirection or here-document,
//!    and not after a `|`. A launch given other input may have sent its child
//!    more than the prompt, so it never makes a link; it still counts as a
//!    launch that could have made a child, so it can stop a link.
//! 7. The launch's hour was over when the match listed its possible
//!    children, so no other session of that hour can still appear (one
//!    imported after the list was taken is in the next match's list). A
//!    session saved in the hour but imported after its end is a known limit.
//!    A link is decided once: it is never withdrawn or moved, so
//!    what a later read misses (a transcript Claude has since deleted, say)
//!    never changes it. What a match does not write it decides again later.
//! 8. No gap in the evidence could change it. A launch gap is a span of
//!    time that may hold a launch the match does not know; it stops a link
//!    when it touches the time from the launch to the child:
//!    - a caller's transcript or helper folder that could not be read whole,
//!      or held more than this version keeps: from its first indexed record
//!      on;
//!    - a caller the index records with `Bash` calls whose transcript no
//!      longer exists, or is not the one file in a real project folder the
//!      index records; or, when the index records `Bash` calls of its helper
//!      agents, whose helper folder or any helper file the index read is no
//!      longer listed: from its first to its last indexed record;
//!    - a `Bash` command naming `claude` that this reader cannot read as
//!      what it runs (a wrapper such as `sudo` or `xargs`, an expansion as
//!      the program, a function, a shell script or input not read, an
//!      unknown option, a prompt that is not literal or not given): at that
//!      call's time; and a launch record naming `claude` with no time: its
//!      caller's indexed span.
//!
//!    A possible child that cannot be read for any reason (no one transcript
//!    in a real project folder, an error, an opening the index does not
//!    match) stops every launch that could have made it. Too many files or
//!    sessions to keep stop every new link. What could not be read is read
//!    again once a scan imports anything.
//! 9. The Codex launch work could no longer name the child: every Codex
//!    history the census finds has been read since the match began, and no
//!    published or unfinished Codex launch naming it is still open — the
//!    checks a display check waits for ([`LaunchBacklog::discovered_since`],
//!    [`LaunchBacklog::unfinished_for`]). A Codex history found only after a
//!    link was written still conflicts both: a known limit.
//! 10. The child is not already related, or named as a child, by any other
//!     evidence, such as a Codex launch naming its session: the store
//!     abstains ([`Store::record_claude_prompt_launch`]).
//!
//! The child fact and the relation are written together
//! ([`Store::record_claude_prompt_launch`]). Work lives in memory in a
//! [`PromptBacklog`] and is done in bounded passes after the other `Bash`
//! work: a startup sweep over every indexed caller, the sessions each scan
//! imports, then one match over every launch found, whose possible children
//! are the sessions whose first input came within two hours after a launch
//! (the child's hour, and a margin for an input saved after its first
//! record). A link waiting for its hour or for the Codex launch work is
//! looked at again once due, by a match that holds no display check unless
//! something was read or imported since the last one. A transcript is read
//! once and then only what is appended to it. Only digests of prompts are
//! kept in memory; identifiers and positions are stored.

mod words;

use super::super::options::{self, Action};
use super::super::source::{self, Allowance, Budget, Reserved};
use super::super::{LaunchBacklog, LaunchLimits, command, rows};
use super::{BIRTH_WINDOW_MS, OwnFirst, OwnOpening, OwnRead, transcript};
use serde_json::Value;
use sha2::Digest;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};
use xt_store::{
    Host, SessionSource, Store,
    batch::escape_like,
    child_fact::ChildFactDisposition,
    creation::{CLAUDE_PROMPT_LAUNCH_VERSION, ClaudePromptLaunchProof, CreationDisposition},
};

/// Callers one sweep page reads from the index.
const CALLER_PAGE: usize = 256;
/// Transcript files remembered; more forget them all (each is read again).
const MAX_FILES: usize = 100_000;
/// Helper-agent files one caller may have; more are not read.
const MAX_HELPERS: usize = 4_096;
/// Launches, or commands that could not be read, one file may hold; a file
/// with more is not used.
const MAX_FILE_LAUNCHES: usize = 10_000;
/// Indexed sessions a match looks at as possible children; more decide
/// nothing.
const MAX_CHILDREN: usize = 100_000;
/// How long after a launch a session's first input may come and it is
/// still listed as a possible child: the child's hour, and a margin for an
/// input saved after the child's first record.
const LISTED_AFTER_MS: i64 = 2 * BIRTH_WINDOW_MS;
/// How soon a link waiting for a Codex launch naming its child is looked at
/// again.
const CODEX_RECHECK_MS: i64 = 60_000;
/// Held links the last match remembers for a report.
const MAX_HELD_REPORTED: usize = 1_000;
/// Bytes kept from before where a file was read up to, to tell that it only
/// grew.
const TAIL: u64 = 64;

/// One launch a transcript holds: its call, its place among the call's
/// launches, when it was made, its prompt's digest and the session it
/// named. Never the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Found {
    call_id: String,
    index: u32,
    launch_ms: i64,
    digest: [u8; 32],
    session_id: Option<String>,
    /// It reads only the input its call was given, so its child's first
    /// input is its prompt alone: only such a launch may make a link.
    matchable: bool,
}

/// What a transcript file was read to hold.
#[derive(Debug)]
struct FileFacts {
    owner: String,
    device: i64,
    inode: i64,
    /// Just after the last complete line read.
    complete: u64,
    /// The bytes just before `complete`.
    tail: Vec<u8>,
    launches: Vec<Found>,
    /// When each `Bash` call naming `claude` that could not be read as
    /// what it runs was made: each may be a launch this match cannot see.
    refused: Vec<i64>,
    /// It holds a record of its owner naming `claude` with no time, or a
    /// line naming `claude` that is not a record: a launch whose time is
    /// not known.
    untimed: bool,
    /// It held more launches, or a longer line, than this version reads:
    /// launches it holds may be missing.
    overfull: bool,
}

/// One transcript being read, from `next`.
#[derive(Debug)]
struct FileRead {
    owner: String,
    path: PathBuf,
    file: File,
    length: u64,
    device: i64,
    inode: i64,
    next: u64,
    complete: u64,
    carry: Vec<u8>,
    /// A line longer than the bound is being passed over.
    skipping: bool,
    launches: Vec<Found>,
    refused: Vec<i64>,
    untimed: bool,
    /// A line was passed over, or launches dropped, since its first byte.
    overfull: bool,
    /// Commands naming `claude` that could not be read, this read.
    refused_commands: usize,
    _held: Reserved,
}

/// A possible child's first input, as last read.
#[derive(Clone, Debug)]
struct Opening {
    device: i64,
    inode: i64,
    /// The first eligible input the index holds, and its first record's
    /// time.
    index_uuid: String,
    first_record_ms: i64,
    /// What its own transcript's opening says; `None` when the whole
    /// transcript holds no eligible input, which the index says it has.
    first: Option<OwnFirst>,
}

/// A child's opening being read.
#[derive(Debug)]
struct ChildRead {
    native: String,
    device: i64,
    inode: i64,
    index_uuid: String,
    first_record_ms: i64,
    read: Box<OwnRead>,
}

/// Why a link the match found was not written now. The one list of reasons
/// a report counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeldBy {
    /// The launch's hour is not over: another session may still appear.
    Hour,
    /// More files or sessions than this version keeps, or a possible child
    /// that is not one indexed session.
    Unknown,
    /// A caller's transcript or helper folder could not be read whole.
    CallerUnread,
    /// A caller's transcript, helper file or helper folder no longer exists.
    CallerGone,
    /// A `Bash` command naming `claude` that could not be read.
    CommandUnread,
    /// A launch record naming `claude` with no time.
    Untimed,
    /// A possible child that could not be read.
    ChildUnread,
    /// A Codex launch naming the child is still open.
    Codex,
}

impl HeldBy {
    /// A closed label for reports.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hour => "hour_not_over",
            Self::Unknown => "too_much_or_unknown",
            Self::CallerUnread => "caller_unread",
            Self::CallerGone => "caller_gone",
            Self::CommandUnread => "command_unread",
            Self::Untimed => "untimed_launch",
            Self::ChildUnread => "child_unread",
            Self::Codex => "codex_launch_open",
        }
    }
}

/// One link the last match held, by identifiers only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldLink {
    pub child_native_session_id: String,
    pub parent_native_session_id: String,
    pub by: Vec<HeldBy>,
}

/// A span of time that may hold a launch the match does not know.
#[derive(Clone, Copy, Debug)]
struct LaunchGap {
    from: i64,
    to: i64,
    by: HeldBy,
}

/// What a match did.
enum Decision {
    Decided,
    /// The Codex launch work has not read every history since the match
    /// began: decided on a later pass.
    WaitCodex,
}

/// Literal-prompt launch work not yet done. A restarted worker sweeps every
/// indexed caller again; every write it repeats changes nothing.
#[derive(Debug, Default)]
pub(in crate::native::claude_launch) struct PromptBacklog {
    sweep: Option<Option<String>>,
    queue: VecDeque<String>,
    queued: HashSet<String>,
    /// Files still to look at, with the session owning them.
    due: VecDeque<(String, PathBuf)>,
    reading: Option<Box<FileRead>>,
    files: HashMap<PathBuf, FileFacts>,
    openings: HashMap<String, Opening>,
    /// The possible children of the match in progress, still to read.
    children: Option<VecDeque<String>>,
    /// Every possible child of the match in progress.
    listed: HashSet<String>,
    child_read: Option<ChildRead>,
    /// Something was read or imported since the match in progress listed
    /// its possible children.
    dirty: bool,
    /// A held link is due to be looked at again: a match runs, though
    /// nothing new was read.
    again: bool,
    /// The match in progress began after something was read or imported, so
    /// it holds display checks; one that only looks at held links again
    /// does not.
    holding: bool,
    /// When the match in progress listed its possible children: a launch's
    /// hour counts as over only if it was over by then, as a session saved
    /// in the hour but imported later is not in the list.
    listed_at: i64,
    /// A display check waited for this work; once it is done, the checks
    /// still waiting are looked at again ([`Self::take_wake`]).
    waited: bool,
    wake: bool,
    memory: Option<Allowance>,
    /// Transcript files that could not be read, with the session owning
    /// each: read again once a scan imports anything.
    unreadable: HashMap<PathBuf, String>,
    /// Callers whose helper-agent folder could not be listed whole: listed
    /// again once a scan imports anything.
    unlisted: HashSet<String>,
    /// Callers the index records with `Bash` calls whose transcript, helper
    /// file or helper folder no longer exists, or is not where it may be
    /// read: looked at again once a scan imports anything.
    gone: HashSet<String>,
    /// More callers or files were seen than this version keeps, so evidence
    /// was forgotten: no new link is written for the rest of this run.
    overflowed: bool,
    /// Possible children of the match in progress whose opening could not be
    /// read, and when each first saved a record.
    child_gaps: Vec<(String, i64)>,
    /// The possible children of the match in progress could not all be
    /// listed or told apart.
    children_unknown: bool,
    /// The Codex census epoch when the match in progress began: every Codex
    /// history must be read after it before the match decides.
    codex_epoch: Option<u64>,
    /// When a link held for its hour or for a Codex launch is looked at
    /// again.
    recheck_at: Option<i64>,
    /// The links the last match held, for a report.
    held: Vec<HeldLink>,
}

/// What one pass of literal-prompt launch work did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptProgress {
    /// Transcript files read, whole or what was appended.
    pub files_read: usize,
    /// Files that could not be read now.
    pub files_unreadable: usize,
    /// Callers whose transcript, helper file or helper folder is gone.
    pub callers_gone: usize,
    /// Matches run over every launch found.
    pub matches: usize,
    /// Passes whose match waited for the Codex launch work to read every
    /// history.
    pub codex_waits: usize,
    /// Launches found by the last match.
    pub launches: usize,
    /// Links written, or already stored.
    pub linked: usize,
    pub already: usize,
    /// Launches keeping more than one possible child, or whose child
    /// another launch keeps too.
    pub ambiguous: usize,
    /// Links the store refused or set aside.
    pub refused: usize,
    /// Rows that changed what a read shows.
    pub changed: usize,
    /// Launches with one possible child that were given other input (a `<`
    /// redirection, a here-document or a pipe), so make no link.
    pub other_input: usize,
    /// Links not written now, and how many each reason held ([`HeldBy`]; a
    /// link may be held by several).
    pub held: usize,
    pub held_hour: usize,
    pub held_unknown: usize,
    pub held_caller_unread: usize,
    pub held_caller_gone: usize,
    pub held_command_unread: usize,
    pub held_untimed: usize,
    pub held_child_unread: usize,
    pub held_codex: usize,
    /// Matches that ran with some evidence missing.
    pub incomplete_matches: usize,
    /// Possible children whose opening could not be read.
    pub children_unreadable: usize,
    /// `Bash` commands naming `claude` that could not be read.
    pub commands_refused: usize,
}

impl PromptProgress {
    fn count_held(&mut self, by: HeldBy) {
        let slot = match by {
            HeldBy::Hour => &mut self.held_hour,
            HeldBy::Unknown => &mut self.held_unknown,
            HeldBy::CallerUnread => &mut self.held_caller_unread,
            HeldBy::CallerGone => &mut self.held_caller_gone,
            HeldBy::CommandUnread => &mut self.held_command_unread,
            HeldBy::Untimed => &mut self.held_untimed,
            HeldBy::ChildUnread => &mut self.held_child_unread,
            HeldBy::Codex => &mut self.held_codex,
        };
        *slot += 1;
    }
}

impl PromptBacklog {
    /// The backlog a watcher starts with: a sweep over every indexed caller.
    pub fn starting() -> Self {
        Self {
            sweep: Some(None),
            ..Self::default()
        }
    }

    /// Claude sessions a scan imported: each may be a caller with new
    /// launches, or a child.
    pub fn add_sessions<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        let mut any = false;
        for native in natives {
            any = true;
            self.dirty = true;
            self.enqueue(native.to_owned());
        }
        // What could not be read before is read again.
        if any {
            let again: Vec<String> = self
                .unreadable
                .values()
                .chain(self.unlisted.iter())
                .chain(self.gone.iter())
                .cloned()
                .collect();
            for owner in again {
                self.enqueue(owner);
            }
        }
    }

    fn enqueue(&mut self, native: String) {
        if self.queued.contains(&native) {
            return;
        }
        if self.queued.len() >= MAX_FILES {
            self.overflowed = true;
            return;
        }
        self.queued.insert(native.clone());
        self.queue.push_back(native);
    }

    /// The callers whose launches may not all have been read while their
    /// files may still grow: those owning a file that could not be read
    /// whole, or a helper folder that could not be listed whole.
    fn hidden_callers(&self) -> HashSet<&str> {
        self.unreadable
            .values()
            .map(String::as_str)
            .chain(self.unlisted.iter().map(String::as_str))
            .chain(
                self.files
                    .values()
                    .filter(|facts| facts.overfull)
                    .map(|facts| facts.owner.as_str()),
            )
            .collect()
    }

    /// Whether reading or matching is still to be done. While it is, a
    /// display check of a Claude session waits: a launch not read or
    /// matched yet may have made it.
    fn busy(&self) -> bool {
        self.sweep.is_some()
            || !self.queue.is_empty()
            || !self.due.is_empty()
            || self.reading.is_some()
            || self.dirty
            || (self.holding && self.children.is_some())
    }

    /// Whether a later pass has anything to do: reading or matching, or a
    /// held link to look at again, now or once due.
    pub fn pending(&self) -> bool {
        self.busy() || self.again || self.children.is_some() || self.recheck_at.is_some()
    }

    /// Whether a display check must wait for this work now; one that must
    /// is remembered, so that the checks wake once the work is done. A link
    /// held for its hour or a Codex launch holds no check, and neither does
    /// the match that looks at it again while nothing new was read: the
    /// child's own check waits for that launch, and a link folds an
    /// already-checked child under its parent.
    pub fn holds_checks(&mut self) -> bool {
        let busy = self.busy();
        self.waited |= busy;
        busy
    }

    /// Whether this work finished since a display check waited for it.
    pub fn take_wake(&mut self) -> bool {
        std::mem::take(&mut self.wake)
    }

    /// [`Self::holds_checks`] without remembering it.
    pub fn busy_now(&self) -> bool {
        self.busy()
    }

    /// The links the last match held, by identifiers.
    pub fn held(&self) -> &[HeldLink] {
        &self.held
    }

    /// Look at a held link again no later than `at`.
    fn recheck(&mut self, at: i64) {
        self.recheck_at = Some(self.recheck_at.map_or(at, |known| known.min(at)));
    }
}

/// One bounded pass, within what is left of `budget`.
#[allow(clippy::too_many_arguments)]
pub(in crate::native::claude_launch) fn pass(
    store: &mut Store,
    home: &Path,
    backlog: &mut PromptBacklog,
    launches: &mut LaunchBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut PromptProgress,
) -> xt_store::Result<()> {
    let memory = backlog
        .memory
        .get_or_insert_with(|| Allowance::new(limits.thread_memory))
        .clone();
    // A held link is looked at again once due, by a new match.
    if backlog.recheck_at.is_some_and(|at| at <= recorded_at) {
        backlog.recheck_at = None;
        backlog.again = true;
    }
    let programs = command::installed_programs(home);
    while !budget.exhausted() && budget.entries < limits.max_pass_entries {
        // 1. A file being read.
        if let Some(reading) = backlog.reading.as_mut() {
            match reading.advance(budget, &programs, home, limits.max_line) {
                Ok(false) => return Ok(()),
                Ok(true) => {
                    let reading = backlog.reading.take().expect("reading");
                    progress.files_read += 1;
                    progress.commands_refused += reading.refused_commands;
                    backlog.dirty = true;
                    let tail = tail(&reading.file, reading.complete);
                    if backlog.files.len() >= MAX_FILES
                        && !backlog.files.contains_key(&reading.path)
                    {
                        // Forgotten launches could have made any child.
                        backlog.files.clear();
                        backlog.overflowed = true;
                    }
                    let overfull = reading.overfull
                        || reading.launches.len() > MAX_FILE_LAUNCHES
                        || reading.refused.len() > MAX_FILE_LAUNCHES;
                    let (launches, refused) = if overfull {
                        (Vec::new(), Vec::new())
                    } else {
                        (reading.launches, reading.refused)
                    };
                    backlog.unreadable.remove(&reading.path);
                    backlog.files.insert(
                        reading.path,
                        FileFacts {
                            owner: reading.owner,
                            device: reading.device,
                            inode: reading.inode,
                            complete: reading.complete,
                            tail,
                            launches,
                            refused,
                            untimed: reading.untimed,
                            overfull,
                        },
                    );
                }
                Err(()) => {
                    let reading = backlog.reading.take().expect("reading");
                    backlog.files.remove(&reading.path);
                    backlog.unreadable.insert(reading.path, reading.owner);
                    backlog.dirty = true;
                    progress.files_unreadable += 1;
                }
            }
            continue;
        }
        // 2. The next file of a caller.
        if let Some((owner, path)) = backlog.due.pop_front() {
            match start(backlog, &owner, &path, limits.max_line, &memory) {
                Start::Reading(reading) => backlog.reading = Some(reading),
                Start::Unchanged => {}
                Start::Missing => {
                    // Deleted history: what was read of it is kept, and its
                    // caller's indexed span is a gap.
                    backlog.gone.insert(owner);
                    backlog.dirty = true;
                    progress.callers_gone += 1;
                }
                Start::Failed => {
                    backlog.files.remove(&path);
                    backlog.unreadable.insert(path, owner);
                    backlog.dirty = true;
                    progress.files_unreadable += 1;
                }
            }
            continue;
        }
        // 3. The next caller: its transcript and its helpers' transcripts.
        if let Some(native) = backlog.queue.pop_front() {
            backlog.queued.remove(&native);
            // This caller's files are listed afresh: what could not be read
            // before is read again, or is gone.
            backlog.unreadable.retain(|_, owner| *owner != native);
            backlog.unlisted.remove(&native);
            if backlog.gone.remove(&native) {
                backlog.dirty = true;
            }
            let Some(path) = transcript(store, home, &native)? else {
                // No one transcript where the index recorded it: deleted, or
                // not where it may be read. A caller's launches are gone.
                if store.claude_bash_calls(&native)?.0 {
                    backlog.gone.insert(native);
                    backlog.dirty = true;
                    progress.callers_gone += 1;
                }
                continue;
            };
            let helpers = path.with_extension("").join("subagents");
            let mut found = Vec::new();
            let mut whole = true;
            let mut no_folder = false;
            match std::fs::read_dir(&helpers) {
                Ok(entries) => {
                    for entry in entries {
                        if !budget.entry() {
                            // Listed again, whole, next pass.
                            backlog.queued.insert(native.clone());
                            backlog.queue.push_front(native);
                            return Ok(());
                        }
                        let Ok(entry) = entry else {
                            whole = false;
                            break;
                        };
                        if !helper_name(&entry.path()) {
                            continue;
                        }
                        if found.len() >= MAX_HELPERS {
                            whole = false;
                            break;
                        }
                        found.push(entry.path());
                    }
                }
                // No helper folder: none ever, or one deleted, as the index
                // tells.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => no_folder = true,
                Err(_) => whole = false,
            }
            // The index records `Bash` calls of its helper agents: its helper
            // folder, and every helper file the index read, must still be
            // there, or a launch it held is gone.
            if whole
                && store.claude_bash_calls(&native)?.1
                && (no_folder || helper_lost(store, &helpers, &found)?)
            {
                backlog.gone.insert(native.clone());
                backlog.dirty = true;
                progress.callers_gone += 1;
            }
            if !whole {
                progress.files_unreadable += 1;
                backlog.unlisted.insert(native.clone());
                backlog.dirty = true;
            }
            found.sort();
            backlog.due.push_back((native.clone(), path));
            backlog
                .due
                .extend(found.into_iter().map(|path| (native.clone(), path)));
            continue;
        }
        // 4. The startup sweep, a page at a time.
        if let Some(after) = backlog.sweep.as_mut() {
            let page = store.claude_bash_callers(after.as_deref(), CALLER_PAGE)?;
            match page.last() {
                Some((last, _)) => *after = Some(last.clone()),
                None => backlog.sweep = None,
            }
            for (_, native) in page {
                backlog.enqueue(native);
            }
            continue;
        }
        // 5. One match over every launch found, once everything is read.
        if backlog.dirty || backlog.again || backlog.children.is_some() {
            if backlog.children.is_none() && backlog.codex_epoch.is_none() {
                // The Codex launch work reads every history after this.
                backlog.codex_epoch = Some(launches.census_epoch());
                launches.renew_census();
            }
            if !read_children(
                store,
                home,
                backlog,
                limits,
                budget,
                &memory,
                recorded_at,
                progress,
            )? {
                return Ok(());
            }
            match decide(store, launches, backlog, recorded_at, progress)? {
                Decision::WaitCodex => progress.codex_waits += 1,
                Decision::Decided => {
                    progress.matches += 1;
                    if backlog.waited && !backlog.busy() {
                        backlog.waited = false;
                        backlog.wake = true;
                    }
                }
            }
        }
        break;
    }
    Ok(())
}

/// Whether a helper-agent file the index read in the folder `helpers`, of a
/// name this reader lists, is not among the files `listed` now: deleted, so
/// what it held is not known. A folder whose path is not text cannot be
/// looked up, so it is taken as lost.
fn helper_lost(store: &Store, helpers: &Path, listed: &[PathBuf]) -> xt_store::Result<bool> {
    let Some(folder) = helpers.to_str() else {
        return Ok(true);
    };
    let pattern = format!("claude:{}/agent-%.jsonl", escape_like(folder));
    let listed: HashSet<&Path> = listed.iter().map(PathBuf::as_path).collect();
    Ok(store
        .source_cursors_like(SessionSource::Transcript, &[&pattern])?
        .iter()
        .filter_map(|cursor| cursor.cursor_key.strip_prefix("claude:").map(Path::new))
        .filter(|path| path.parent() == Some(helpers) && helper_name(path))
        .any(|path| !listed.contains(path)))
}

/// Whether a file in a helper folder is a helper agent's transcript this
/// reader reads.
fn helper_name(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy())
        .is_some_and(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
}

/// How a file's read begins.
enum Start {
    Reading(Box<FileRead>),
    /// Read to its end already, and only possibly grown by a partial line.
    Unchanged,
    /// It no longer exists: deleted history.
    Missing,
    Failed,
}

fn start(
    backlog: &PromptBacklog,
    owner: &str,
    path: &Path,
    max_line: usize,
    memory: &Allowance,
) -> Start {
    let (file, generation) = match source::open(path) {
        Ok(opened) => opened,
        Err(source::Unread::Missing) => return Start::Missing,
        Err(_) => return Start::Failed,
    };
    let Ok(length) = u64::try_from(generation.length) else {
        return Start::Failed;
    };
    let Ok(held) = memory.reserve(path.as_os_str().len() + max_line.min(1 << 20)) else {
        return Start::Failed;
    };
    // Only what was appended is read, when the file is the same one and the
    // bytes before where it was read up to are unchanged.
    let mut from = 0;
    let mut launches = Vec::new();
    let mut refused = Vec::new();
    let mut untimed = false;
    let mut overfull = false;
    if let Some(known) = backlog.files.get(path)
        && known.owner == owner
        && known.device == generation.device
        && known.inode == generation.inode
        && known.complete <= length
        && tail(&file, known.complete) == known.tail
    {
        if known.complete == length {
            return Start::Unchanged;
        }
        from = known.complete;
        launches = known.launches.clone();
        refused = known.refused.clone();
        untimed = known.untimed;
        overfull = known.overfull;
    }
    Start::Reading(Box::new(FileRead {
        owner: owner.to_owned(),
        path: path.to_path_buf(),
        file,
        length,
        device: generation.device,
        inode: generation.inode,
        next: from,
        complete: from,
        carry: Vec::new(),
        skipping: false,
        launches,
        refused,
        untimed,
        overfull,
        refused_commands: 0,
        _held: held,
    }))
}

/// The bytes of `file` just before `end`.
fn tail(file: &File, end: u64) -> Vec<u8> {
    let start = end.saturating_sub(TAIL);
    let mut bytes = vec![0_u8; usize::try_from(end - start).unwrap_or(0)];
    match file.read_exact_at(&mut bytes, start) {
        Ok(()) => bytes,
        Err(_) => Vec::new(),
    }
}

impl FileRead {
    /// Read on within `budget`: `Ok(true)` once every byte of the length
    /// observed at open was read, `Ok(false)` when paused.
    fn advance(
        &mut self,
        budget: &mut Budget<'_>,
        programs: &[PathBuf],
        home: &Path,
        max_line: usize,
    ) -> Result<bool, ()> {
        while self.next < self.length {
            let chunk =
                match source::read_chunk(&self.file, self.next, self.length - self.next, budget) {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => return Ok(false),
                    Err(_) => return Err(()),
                };
            let mut rest = chunk.as_slice();
            let mut at = self.next;
            while let Some(end) = rest.iter().position(|&byte| byte == b'\n') {
                if self.skipping {
                    self.skipping = false;
                } else if self.carry.is_empty() {
                    self.line(&rest[..end], programs, home);
                } else if self.carry.len() + end <= max_line {
                    let mut whole = std::mem::take(&mut self.carry);
                    whole.extend_from_slice(&rest[..end]);
                    self.line(&whole, programs, home);
                } else {
                    // A line whose end makes it longer than the bound is
                    // passed over: a launch it may hold is missing.
                    self.overfull = true;
                }
                self.carry.clear();
                at += end as u64 + 1;
                self.complete = at;
                rest = &rest[end + 1..];
            }
            if !self.skipping {
                if self.carry.len() + rest.len() > max_line {
                    // A longer line is passed over: a launch it may hold is
                    // missing.
                    self.carry = Vec::new();
                    self.skipping = true;
                    self.overfull = true;
                } else {
                    self.carry.extend_from_slice(rest);
                }
            }
            self.next += chunk.len() as u64;
        }
        // Only complete lines count: an unfinished last line is read again
        // when it is complete.
        self.carry = Vec::new();
        self.skipping = false;
        Ok(true)
    }

    /// The launches one complete line holds, and the commands naming
    /// `claude` it holds that could not be read.
    fn line(&mut self, bytes: &[u8], programs: &[PathBuf], home: &Path) {
        if self.launches.len() > MAX_FILE_LAUNCHES || self.refused.len() > MAX_FILE_LAUNCHES {
            self.overfull = true;
            return;
        }
        // A launch names `claude`; a line that escapes characters may spell
        // it otherwise, and is read whole.
        let names = contains(bytes, b"claude");
        if !(names || contains(bytes, b"\\u")) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            // A line naming `claude` that is not a record: what it held,
            // and when, is not known.
            self.untimed |= names;
            return;
        };
        // Only an assistant record of the session owning the transcript.
        if value.get("type").and_then(Value::as_str) != Some("assistant")
            || value.get("sessionId").and_then(Value::as_str) != Some(self.owner.as_str())
        {
            return;
        }
        let launch_ms = rows::timestamp(&value);
        let Some(blocks) = value["message"]["content"].as_array() else {
            return;
        };
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Bash")
            {
                continue;
            }
            let Some(command) = block["input"].get("command").and_then(Value::as_str) else {
                continue;
            };
            let Some(launch_ms) = launch_ms else {
                // A launch whose time is not known.
                self.untimed |= command.contains("claude");
                continue;
            };
            let Some(id) = block
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| token(id))
            else {
                if command.contains("claude") {
                    self.refused.push(launch_ms);
                    self.refused_commands += 1;
                }
                continue;
            };
            let read = read_command(command, programs, home);
            if read.refused {
                self.refused.push(launch_ms);
                self.refused_commands += 1;
            }
            for (index, launch) in read.launches.into_iter().enumerate() {
                let Ok(index) = u32::try_from(index) else {
                    self.overfull = true;
                    break;
                };
                self.launches.push(Found {
                    call_id: id.to_owned(),
                    index,
                    launch_ms,
                    digest: launch.digest,
                    session_id: launch.session_id,
                    matchable: launch.own_input,
                });
            }
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

/// One create-mode Claude print launch with a literal prompt.
struct Literal {
    digest: [u8; 32],
    session_id: Option<String>,
    own_input: bool,
}

/// What one recorded `Bash` command was read to hold.
struct CommandRead {
    /// The create-mode Claude print launches with a literal prompt, in
    /// order: each prompt's digest, the session it names and whether it
    /// reads only its call's input.
    launches: Vec<Literal>,
    /// It names `claude`, and part of it may run Claude in a way this
    /// reader cannot read: a launch it cannot see.
    refused: bool,
}

/// Shells and evaluators that run text given to them.
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "eval", "source", ".",
];

/// Programs that never run a word they are given as a program.
const NON_RUNNERS: &[&str] = &[
    "echo", "printf", "grep", "egrep", "fgrep", "rg", "ls", "cat", "head", "tail", "less", "which",
    "type", "whereis", "wc", "git", "jq", "sed", "awk", "tee", "touch", "mkdir", "rm", "rmdir",
    "cp", "mv", "ln", "chmod", "stat", "file", "du", "df", "open", "pgrep", "pkill", "kill", "ps",
    "lsof", "test", "[", "cd", "pushd", "popd", "basename", "dirname", "realpath", "readlink",
    "sort", "uniq", "cut", "tr", "diff", "tar", "zip", "unzip",
    // `command -v`, after the reader set `command` aside.
    "-v", "-V",
];

/// Read one recorded `Bash` command. Only a command naming `claude` can be
/// refused.
fn read_command(command: &str, programs: &[PathBuf], home: &Path) -> CommandRead {
    let names = command.contains("claude");
    let Some(commands) = words::simple_commands(command) else {
        return CommandRead {
            launches: Vec::new(),
            refused: names,
        };
    };
    let mut found = Vec::new();
    let mut refused = false;
    for simple in &commands {
        if simple
            .words
            .iter()
            .all(|word| matches!(word, words::Word::Assign(_)))
        {
            // Assignments alone run nothing.
            continue;
        }
        let Some((program, args)) = words::program(&simple.words) else {
            // A wrapper this reader cannot see through.
            refused |= names && mentions(&simple.words);
            continue;
        };
        let claude = match program {
            words::Word::Lit(name) => {
                name == "claude"
                    || programs
                        .iter()
                        .any(|known| known.as_os_str() == name.as_str())
            }
            words::Word::Home(rest) => programs.iter().any(|known| *known == home.join(rest)),
            words::Word::Opaque | words::Word::Assign(_) => false,
        };
        if !claude {
            refused |= names && hides_claude(program, args, &simple.words);
            continue;
        }
        let args: Vec<Option<String>> = args
            .iter()
            .map(|word| match word {
                words::Word::Lit(text) => Some(text.clone()),
                words::Word::Assign(text) => text.clone(),
                words::Word::Home(_) | words::Word::Opaque => None,
            })
            .collect();
        let Some(read) = options::read_opaque(&args) else {
            // A Claude command whose options or prompt are not read.
            refused = true;
            continue;
        };
        if read.action != Action::Create {
            continue;
        }
        let Some(prompt) = read.prompt.filter(|prompt| !prompt.is_empty()) else {
            // A launch whose prompt comes from elsewhere.
            refused = true;
            continue;
        };
        found.push(Literal {
            digest: sha2::Sha256::digest(prompt.as_bytes()).into(),
            session_id: read.session_id,
            own_input: simple.own_input,
        });
    }
    CommandRead {
        launches: found,
        refused,
    }
}

/// Whether a word names `claude`.
fn mentions(words: &[words::Word]) -> bool {
    words.iter().any(|word| match word {
        words::Word::Lit(text) | words::Word::Home(text) | words::Word::Assign(Some(text)) => {
            text.contains("claude")
        }
        words::Word::Opaque | words::Word::Assign(None) => false,
    })
}

/// Whether a simple command whose program is not a known Claude launcher
/// may still run Claude, in a command naming `claude`: a program the shell
/// supplies with a print option, a program named `claude` this reader does
/// not know, a shell given a script or input this reader did not read, or
/// any other program but a few that run nothing given a word naming the
/// Claude program (`sudo claude`, `xargs claude`).
fn hides_claude(program: &words::Word, args: &[words::Word], all: &[words::Word]) -> bool {
    let name = match program {
        words::Word::Opaque => {
            return args.iter().any(|word| {
                matches!(word, words::Word::Lit(text)
                    if text == "-p" || text == "--print" || text.starts_with("--print="))
            });
        }
        words::Word::Lit(text) | words::Word::Home(text) => text.as_str(),
        words::Word::Assign(_) => return false,
    };
    let base = name.rsplit('/').next().unwrap_or(name);
    if base == "claude" {
        return true;
    }
    if SHELLS.contains(&base) {
        if words::shell_script(all).is_some() {
            // Its script was read as commands.
            return false;
        }
        // A script naming Claude, or input it reads.
        return mentions(args)
            || args
                .iter()
                .all(|word| matches!(word, words::Word::Lit(text) if text.starts_with('-')));
    }
    if NON_RUNNERS.contains(&base) {
        return false;
    }
    args.iter().any(|word| match word {
        words::Word::Lit(text) | words::Word::Home(text) => {
            text == "claude" || text.ends_with("/claude")
        }
        _ => false,
    })
}

/// Read the opening of every indexed Claude session that may be a child of
/// a launch found: those whose first input came within
/// [`LISTED_AFTER_MS`] after a launch. `false` when the budget ended first;
/// the reads resume next pass. An opening that cannot be read now is a gap
/// of this match, never kept: the next match reads it again.
#[allow(clippy::too_many_arguments)]
fn read_children(
    store: &Store,
    home: &Path,
    backlog: &mut PromptBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    memory: &Allowance,
    recorded_at: i64,
    progress: &mut PromptProgress,
) -> xt_store::Result<bool> {
    if backlog.children.is_none() {
        // What is read or imported from now on needs another match.
        backlog.holding = backlog.dirty;
        backlog.dirty = false;
        backlog.again = false;
        backlog.listed_at = recorded_at;
        backlog.child_gaps.clear();
        backlog.children_unknown = false;
        let mut times: Vec<i64> = backlog
            .files
            .values()
            .flat_map(|facts| facts.launches.iter().map(|found| found.launch_ms))
            .collect();
        times.sort_unstable();
        times.dedup();
        // The spans after the launches, joined where they overlap.
        let mut spans: Vec<(i64, i64)> = Vec::new();
        for time in times {
            let end = time.saturating_add(LISTED_AFTER_MS);
            match spans.last_mut() {
                Some(last) if time <= last.1 => last.1 = last.1.max(end),
                _ => spans.push((time, end)),
            }
        }
        let mut born: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for (from, to) in spans {
            let left = MAX_CHILDREN + 1 - born.len();
            for (native, _) in store.claude_sessions_born_between(from, to, left)? {
                if seen.insert(native.clone()) {
                    born.push(native);
                }
            }
            if born.len() > MAX_CHILDREN {
                break;
            }
        }
        // Too many to look at: nothing is decided.
        let born: VecDeque<String> = if born.len() > MAX_CHILDREN {
            backlog.children_unknown = true;
            VecDeque::new()
        } else {
            born.into()
        };
        backlog.listed = born.iter().cloned().collect();
        backlog.children = Some(born);
    }
    loop {
        if let Some(mut reading) = backlog.child_read.take() {
            let first = match reading.read.advance(budget) {
                OwnOpening::Paused => {
                    backlog.child_read = Some(reading);
                    return Ok(false);
                }
                OwnOpening::Input(_, first) => Some(first),
                OwnOpening::NoInput(_) => None,
                OwnOpening::Failed(_) | OwnOpening::Again(_) => {
                    backlog.openings.remove(&reading.native);
                    backlog
                        .child_gaps
                        .push((reading.native, reading.first_record_ms));
                    progress.children_unreadable += 1;
                    continue;
                }
            };
            backlog.openings.insert(
                std::mem::take(&mut reading.native),
                Opening {
                    device: reading.device,
                    inode: reading.inode,
                    index_uuid: reading.index_uuid,
                    first_record_ms: reading.first_record_ms,
                    first,
                },
            );
            continue;
        }
        if budget.exhausted() {
            return Ok(false);
        }
        let Some(native) = backlog.children.as_mut().and_then(VecDeque::pop_front) else {
            return Ok(true);
        };
        let sessions = store.user_sessions_with_native(Host::Claude, &native)?;
        let [session] = sessions.as_slice() else {
            // Not one indexed session of this identity: which one holds
            // which input is not known.
            backlog.children_unknown = true;
            progress.children_unreadable += 1;
            backlog.openings.remove(&native);
            continue;
        };
        let Some(first_record_ms) = store.session_first_record_ms(session)? else {
            // Listed for a timed input, yet with no timed record.
            backlog.children_unknown = true;
            progress.children_unreadable += 1;
            backlog.openings.remove(&native);
            continue;
        };
        // Any reason it cannot be read leaves a gap where it was saved.
        let mut gap = |backlog: &mut PromptBacklog, native: String| {
            backlog.openings.remove(&native);
            backlog.child_gaps.push((native, first_record_ms));
            progress.children_unreadable += 1;
        };
        let Some((index_uuid, _)) = store.first_input_record(session)? else {
            gap(backlog, native);
            continue;
        };
        let Some(path) = transcript(store, home, &native)? else {
            gap(backlog, native);
            continue;
        };
        let Ok(now) = source::stat(&path) else {
            gap(backlog, native);
            continue;
        };
        if let Some(known) = backlog.openings.get_mut(&native)
            && known.device == now.device
            && known.inode == now.inode
            && known.index_uuid == index_uuid
        {
            known.first_record_ms = first_record_ms;
            continue;
        }
        match OwnRead::at(&path, &native, limits.max_line, memory) {
            Ok(read) => {
                backlog.child_read = Some(ChildRead {
                    native,
                    device: now.device,
                    inode: now.inode,
                    index_uuid,
                    first_record_ms,
                    read: Box::new(read),
                });
            }
            Err(_) => gap(backlog, native),
        }
    }
}

/// Whether a launch `found` in `owner`'s transcripts could have made the
/// possible child `native`, first saved at `first_record_ms`, if its first
/// input is the launch's prompt: another session, saved at or after the call
/// and no more than [`BIRTH_WINDOW_MS`] after it, and the one the launch
/// named, if it named one. Every time check of this match goes through here.
fn could_make(owner: &str, found: &Found, native: &str, first_record_ms: i64) -> bool {
    native != owner
        && first_record_ms >= found.launch_ms
        && first_record_ms - found.launch_ms <= BIRTH_WINDOW_MS
        && found
            .session_id
            .as_deref()
            .is_none_or(|named| named == native)
}

/// One launch as the match sees it.
struct Launch<'a> {
    owner: &'a str,
    found: &'a Found,
}

/// The earliest and latest times of a caller's indexed records: every time
/// it may have made a launch. A caller with no timed record could have made
/// one at any time.
fn caller_span(store: &Store, owner: &str) -> xt_store::Result<(i64, i64)> {
    Ok(match store.claude_record_span(owner)? {
        (Some(first), Some(last)) => (first, last),
        _ => (i64::MIN, i64::MAX),
    })
}

/// The launch gaps of what was read ([`LaunchGap`]).
fn launch_gaps(store: &Store, backlog: &PromptBacklog) -> xt_store::Result<Vec<LaunchGap>> {
    let mut spans: HashMap<String, (i64, i64)> = HashMap::new();
    let mut span = |owner: &str| -> xt_store::Result<(i64, i64)> {
        if let Some(known) = spans.get(owner) {
            return Ok(*known);
        }
        let found = caller_span(store, owner)?;
        spans.insert(owner.to_owned(), found);
        Ok(found)
    };
    let mut gaps = Vec::new();
    // Its files may still grow: from its first record on.
    let mut hidden: Vec<&str> = backlog.hidden_callers().into_iter().collect();
    hidden.sort_unstable();
    for owner in hidden {
        let (first, _) = span(owner)?;
        gaps.push(LaunchGap {
            from: first,
            to: i64::MAX,
            by: HeldBy::CallerUnread,
        });
    }
    // Deleted history: what it held, when its records were saved.
    let mut gone: Vec<&String> = backlog.gone.iter().collect();
    gone.sort_unstable();
    for owner in gone {
        let (from, to) = span(owner)?;
        gaps.push(LaunchGap {
            from,
            to,
            by: HeldBy::CallerGone,
        });
    }
    let mut paths: Vec<&PathBuf> = backlog.files.keys().collect();
    paths.sort();
    for path in paths {
        let facts = &backlog.files[path];
        for &at in &facts.refused {
            gaps.push(LaunchGap {
                from: at,
                to: at,
                by: HeldBy::CommandUnread,
            });
        }
        if facts.untimed {
            let (from, to) = span(&facts.owner)?;
            gaps.push(LaunchGap {
                from,
                to,
                by: HeldBy::Untimed,
            });
        }
    }
    Ok(gaps)
}

/// Add `reason` to `by` once.
fn hold(by: &mut Vec<HeldBy>, reason: HeldBy) {
    if !by.contains(&reason) {
        by.push(reason);
    }
}

/// Match every launch found with its possible children, and link the
/// launches with exactly one that no other launch could have made, once
/// nothing missing or still to come could change that. A stored link is
/// never withdrawn or moved.
fn decide(
    store: &mut Store,
    codex: &mut LaunchBacklog,
    backlog: &mut PromptBacklog,
    recorded_at: i64,
    progress: &mut PromptProgress,
) -> xt_store::Result<Decision> {
    // Not while the Codex launch work could still find a launch naming a
    // child: it has read every history since this match began.
    let any = backlog
        .files
        .values()
        .any(|facts| !facts.launches.is_empty());
    if any
        && !backlog
            .codex_epoch
            .is_some_and(|epoch| codex.discovered_since(epoch))
    {
        return Ok(Decision::WaitCodex);
    }
    backlog.codex_epoch = None;
    let children = backlog.children.take().unwrap_or_default();
    debug_assert!(children.is_empty());
    backlog.held.clear();
    // Only the sessions this match listed are possible children.
    let listed = std::mem::take(&mut backlog.listed);
    backlog.openings.retain(|native, _| listed.contains(native));
    let unknown = backlog.overflowed || backlog.children_unknown;
    let gaps = launch_gaps(store, backlog)?;
    // A possible child whose transcript holds no eligible input though the
    // index says it has one cannot be read either.
    let mut child_gaps = std::mem::take(&mut backlog.child_gaps);
    for (native, opening) in &backlog.openings {
        if opening.first.is_none() {
            child_gaps.push((native.clone(), opening.first_record_ms));
        }
    }
    if unknown || !gaps.is_empty() || !child_gaps.is_empty() {
        progress.incomplete_matches += 1;
    }
    // One entry per launch: the same call read in two files of its owner is
    // one launch.
    let mut seen = HashSet::new();
    let mut launches: Vec<Launch<'_>> = Vec::new();
    let mut paths: Vec<&PathBuf> = backlog.files.keys().collect();
    paths.sort();
    for path in paths {
        let facts = &backlog.files[path];
        for found in &facts.launches {
            if seen.insert((facts.owner.as_str(), found.call_id.as_str(), found.index)) {
                launches.push(Launch {
                    owner: &facts.owner,
                    found,
                });
            }
        }
    }
    progress.launches = launches.len();
    // The possible children, by the digest of their first input. One whose
    // transcript's first input is not the one the index holds may still have
    // been made by a launch, but is never linked.
    let mut by_digest: HashMap<[u8; 32], Vec<&str>> = HashMap::new();
    let mut unlinkable: HashSet<&str> = HashSet::new();
    for (native, opening) in &backlog.openings {
        let Some(first) = &opening.first else {
            continue;
        };
        let Some(digest) = first.digest else {
            continue;
        };
        if first.uuid.as_deref() != Some(opening.index_uuid.as_str()) {
            unlinkable.insert(native);
        }
        by_digest.entry(digest).or_default().push(native);
    }
    let candidates: Vec<Vec<&str>> = launches
        .iter()
        .map(|launch| {
            let mut possible: Vec<&str> = by_digest
                .get(&launch.found.digest)
                .into_iter()
                .flatten()
                .copied()
                .filter(|native| {
                    could_make(
                        launch.owner,
                        launch.found,
                        native,
                        backlog.openings[*native].first_record_ms,
                    )
                })
                .collect();
            possible.sort_unstable();
            possible
        })
        .collect();
    // Each possible child belongs to the latest launch that could have made
    // it: a retried prompt's child is the closest send before it. Launches
    // tied for latest all keep it, and so does a launch that named it, so
    // neither links it.
    let mut could: HashMap<&str, Vec<usize>> = HashMap::new();
    for (at, possible) in candidates.iter().enumerate() {
        for native in possible {
            could.entry(native).or_default().push(at);
        }
    }
    let owners: HashMap<&str, Vec<usize>> = could
        .into_iter()
        .map(|(native, ats)| {
            let latest = ats
                .iter()
                .map(|&at| launches[at].found.launch_ms)
                .max()
                .unwrap_or(i64::MIN);
            let owners: Vec<usize> = ats
                .into_iter()
                .filter(|&at| {
                    launches[at].found.launch_ms == latest
                        || launches[at].found.session_id.as_deref() == Some(native)
                })
                .collect();
            (native, owners)
        })
        .collect();
    // The children each launch keeps.
    let mut kept: Vec<Vec<&str>> = vec![Vec::new(); launches.len()];
    for (native, ats) in &owners {
        for &at in ats {
            kept[at].push(native);
        }
    }
    // A launch links only the one child it keeps, and only when no other
    // launch keeps that child too.
    let unique = |at: usize| -> Option<&str> {
        match kept[at].as_slice() {
            [child] if owners.get(child).map(Vec::as_slice) == Some(&[at][..]) => Some(*child),
            _ => None,
        }
    };
    let stored = store.claude_prompt_launches()?;
    let mut proofs = Vec::new();
    let mut held = Vec::new();
    let mut recheck: Option<i64> = None;
    for (at, launch) in launches.iter().enumerate() {
        let Some(child) = unique(at) else {
            if !kept[at].is_empty() {
                progress.ambiguous += 1;
            }
            continue;
        };
        // Given other input, its child's first input may not be its prompt
        // alone: it only stops other links.
        if !launch.found.matchable {
            progress.other_input += 1;
            continue;
        }
        if unlinkable.contains(child) {
            progress.ambiguous += 1;
            continue;
        }
        let this = |proof: &ClaudePromptLaunchProof| {
            proof.child_native_session_id == child
                && proof.parent_native_session_id == launch.owner
                && proof.launch_call_id == launch.found.call_id
                && proof.launch_operation_index == launch.found.index
        };
        // Already decided: stored links stay as they are.
        if stored
            .iter()
            .any(|(proof, accepted)| *accepted && this(proof))
        {
            progress.already += 1;
            continue;
        }
        if stored
            .iter()
            .any(|(proof, _)| proof.child_native_session_id == child)
        {
            progress.ambiguous += 1;
            continue;
        }
        let born = backlog.openings[child].first_record_ms;
        let launched = launch.found.launch_ms;
        let mut by: Vec<HeldBy> = Vec::new();
        // Another session of its hour may still appear, or may have been
        // imported after this match listed its possible children.
        let hour_over = launched.saturating_add(BIRTH_WINDOW_MS);
        if backlog.listed_at <= hour_over {
            hold(&mut by, HeldBy::Hour);
            recheck = Some(recheck.map_or(hour_over + 1, |known| known.min(hour_over + 1)));
        }
        if unknown {
            hold(&mut by, HeldBy::Unknown);
        }
        // A launch not known may have been made between the launch and its
        // child: the child's closest launch, or one tied with it.
        for gap in &gaps {
            if gap.from <= born && gap.to >= launched {
                hold(&mut by, gap.by);
            }
        }
        // A possible child not read may be this launch's too.
        if child_gaps
            .iter()
            .any(|(native, ms)| could_make(launch.owner, launch.found, native, *ms))
        {
            hold(&mut by, HeldBy::ChildUnread);
        }
        // A Codex launch naming the child still to be decided.
        if by.is_empty()
            && (store.claude_launch_child_open(child, Host::Claude)?
                || codex.unfinished_for(Host::Claude, child, i64::MAX))
        {
            hold(&mut by, HeldBy::Codex);
            let again = recorded_at.saturating_add(CODEX_RECHECK_MS);
            recheck = Some(recheck.map_or(again, |known| known.min(again)));
        }
        if !by.is_empty() {
            progress.held += 1;
            for &reason in &by {
                progress.count_held(reason);
            }
            if held.len() < MAX_HELD_REPORTED {
                held.push(HeldLink {
                    child_native_session_id: child.to_owned(),
                    parent_native_session_id: launch.owner.to_owned(),
                    by,
                });
            }
            continue;
        }
        let parents = store.user_sessions_with_native(Host::Claude, launch.owner)?;
        let children = store.user_sessions_with_native(Host::Claude, child)?;
        let ([parent], [child_session]) = (parents.as_slice(), children.as_slice()) else {
            progress.refused += 1;
            continue;
        };
        proofs.push(ClaudePromptLaunchProof {
            child_session_id: child_session.clone(),
            child_native_session_id: child.to_owned(),
            parent_session_id: parent.clone(),
            parent_native_session_id: launch.owner.to_owned(),
            first_record_uuid: backlog.openings[child].index_uuid.clone(),
            launch_call_id: launch.found.call_id.clone(),
            launch_operation_index: launch.found.index,
            evidence_version: CLAUDE_PROMPT_LAUNCH_VERSION,
        });
    }
    drop(launches);
    backlog.held = held;
    if let Some(at) = recheck {
        backlog.recheck(at);
    }
    for proof in proofs {
        let report = store.record_claude_prompt_launch(&proof, recorded_at)?;
        progress.changed += report.changed;
        match (report.fact, report.relation) {
            (
                ChildFactDisposition::Recorded | ChildFactDisposition::AlreadyRecorded,
                CreationDisposition::Recorded,
            ) => progress.linked += 1,
            (
                ChildFactDisposition::Recorded | ChildFactDisposition::AlreadyRecorded,
                CreationDisposition::AlreadyRecorded,
            ) => progress.already += 1,
            _ => progress.refused += 1,
        }
    }
    Ok(Decision::Decided)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Read `bytes` as a transcript with lines of at most `max_line`, in
    /// reads of `first` bytes and then the rest; whether it reports a line
    /// passed over.
    fn overfull_after(bytes: &[u8], max_line: usize, first: u64) -> bool {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("caller.jsonl");
        std::fs::write(&path, bytes).unwrap();
        let backlog = PromptBacklog::starting();
        let memory = Allowance::new(1 << 20);
        let Start::Reading(mut reading) = start(&backlog, "owner", &path, max_line, &memory) else {
            panic!("the transcript is read");
        };
        let mut budget = Budget::new(first, Duration::from_secs(5), None);
        assert_eq!(
            reading.advance(&mut budget, &[], temp.path(), max_line),
            Ok(false)
        );
        let mut budget = Budget::new(u64::MAX, Duration::from_secs(5), None);
        assert_eq!(
            reading.advance(&mut budget, &[], temp.path(), max_line),
            Ok(true)
        );
        reading.overfull
    }

    #[test]
    fn a_long_line_ending_in_a_later_read_is_reported_passed_over() {
        let mut bytes = vec![b'x'; 150];
        bytes.extend_from_slice(b"\n{}\n");
        // Its first 60 bytes fit the bound; the rest arrive with its end.
        assert!(overfull_after(&bytes, 100, 60));
        // Past the bound before its end arrives.
        assert!(overfull_after(&bytes, 100, 120));
        // A line within the bound, read in two parts, is read.
        assert!(!overfull_after(&bytes, 200, 60));
    }
}
