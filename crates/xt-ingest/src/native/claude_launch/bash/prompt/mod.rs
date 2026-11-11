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
//!    not after a `|` and not inside a process substitution (`>(…)`,
//!    `<(…)`). A launch given other input may have sent its child
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
//!      unknown option, a word the shell may split into no word or several
//!      ([`words::Word::Fields`]), a prompt that is not literal or not
//!      given): at that
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
//! once and then only what is appended to it. A read of a transcript that
//! held no launch is saved ([`saved`]), so after a restart an unchanged one
//! is not read again and a grown one only from where it was read up to; one
//! holding a launch is read whole on every start. Only digests of prompts
//! are kept in memory; identifiers and positions are stored.

mod words;

use super::super::options::{self, Action};
use super::super::source::{self, Allowance, Budget, Reserved};
use super::super::{LaunchBacklog, LaunchLimits, command, rows};
use super::saved::{self, Fingerprint, PROMPT_READ_VERSION};
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
    claude_launch::SegmentGeneration,
    creation::{CLAUDE_PROMPT_LAUNCH_VERSION, ClaudePromptLaunchProof, CreationDisposition},
    transcript_read::{SavedTranscriptRead, TranscriptReader},
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
    /// The rules and launchers it was read with ([`saved::rules_digest`]):
    /// a later read goes on from `complete` only with the same.
    rules: [u8; 32],
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
    /// The generation it had when this read began.
    generation: SegmentGeneration,
    /// The Claude launchers installed when this read began: every line is
    /// read with them, and a saved read names them ([`saved::rules_digest`]).
    programs: Vec<PathBuf>,
    rules: [u8; 32],
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
/// indexed caller again, reading only the files no saved read describes;
/// every write it repeats changes nothing.
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
    /// Reads of files that held no launch, to save at the end of the pass,
    /// and saved reads to forget.
    unsaved: Vec<SavedTranscriptRead>,
    forget: Vec<(TranscriptReader, String)>,
}

/// What one pass of literal-prompt launch work did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptProgress {
    /// Transcript files read, whole or what was appended.
    pub files_read: usize,
    /// Bytes this match read: caller transcripts and possible children's
    /// openings.
    pub bytes_read: u64,
    /// Transcript files not read again because a saved read showed them
    /// unchanged.
    pub files_skipped: usize,
    /// Passes whose reads could not be saved (each is read again after a
    /// restart).
    pub saves_failed: usize,
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
    /// redirection, a here-document, a pipe or a process substitution), so
    /// make no link.
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

    /// For acceptance reports: per transcript file this run knows, a digest
    /// of its path, where it was read up to, and a digest of what the match
    /// takes from it (owner, launches, unreadable command times, untimed,
    /// overfull). Digests and a length only; never a path or a prompt.
    pub fn file_digests(&self) -> Vec<([u8; 32], u64, [u8; 32])> {
        let mut out: Vec<([u8; 32], u64, [u8; 32])> = self
            .files
            .iter()
            .map(|(path, facts)| {
                let mut digest = sha2::Sha256::new();
                digest.update(facts.owner.as_bytes());
                digest.update([0]);
                for found in &facts.launches {
                    digest.update(found.call_id.as_bytes());
                    digest.update([0]);
                    digest.update(found.index.to_le_bytes());
                    digest.update(found.launch_ms.to_le_bytes());
                    digest.update(found.digest);
                    digest.update(found.session_id.as_deref().unwrap_or("-").as_bytes());
                    digest.update([u8::from(found.matchable), 0]);
                }
                digest.update([1]);
                for at in &facts.refused {
                    digest.update(at.to_le_bytes());
                }
                digest.update([u8::from(facts.untimed), u8::from(facts.overfull)]);
                (
                    sha2::Sha256::digest(path.as_os_str().as_encoded_bytes()).into(),
                    facts.complete,
                    digest.finalize().into(),
                )
            })
            .collect();
        out.sort();
        out
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
    let spent = budget.spent;
    let result = pass_within(
        store,
        home,
        backlog,
        launches,
        limits,
        budget,
        recorded_at,
        progress,
        &memory,
        &programs,
    );
    progress.bytes_read += budget.spent - spent;
    // What this pass read, saved in one transaction. Saved reads are only a
    // shortcut: one the store refuses is dropped, never in the way of the
    // match.
    if store
        .save_transcript_reads(&backlog.unsaved, &backlog.forget, recorded_at)
        .is_err()
    {
        progress.saves_failed += 1;
    }
    backlog.unsaved.clear();
    backlog.forget.clear();
    result
}

#[allow(clippy::too_many_arguments)]
fn pass_within(
    store: &mut Store,
    home: &Path,
    backlog: &mut PromptBacklog,
    launches: &mut LaunchBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut PromptProgress,
    memory: &Allowance,
    programs: &[PathBuf],
) -> xt_store::Result<()> {
    let digest = saved::rules_digest(programs);
    while !budget.exhausted() && budget.entries < limits.max_pass_entries {
        // 1. A file being read.
        if let Some(reading) = backlog.reading.as_mut() {
            match reading.advance(budget, home, limits.max_line) {
                Ok(false) => return Ok(()),
                Ok(true) => {
                    let mut reading = backlog.reading.take().expect("reading");
                    progress.files_read += 1;
                    progress.commands_refused += reading.refused_commands;
                    let tail = tail(&reading.file, reading.complete);
                    let overfull = reading.overfull
                        || reading.launches.len() > MAX_FILE_LAUNCHES
                        || reading.refused.len() > MAX_FILE_LAUNCHES;
                    let (launches, refused) = if overfull {
                        (Vec::new(), Vec::new())
                    } else {
                        (
                            std::mem::take(&mut reading.launches),
                            std::mem::take(&mut reading.refused),
                        )
                    };
                    let facts = FileFacts {
                        owner: reading.owner.clone(),
                        device: reading.device,
                        inode: reading.inode,
                        complete: reading.complete,
                        tail,
                        launches,
                        refused,
                        untimed: reading.untimed,
                        overfull,
                        rules: reading.rules,
                    };
                    note_read(backlog, &reading, &facts, limits.max_line, budget);
                    keep(backlog, reading.path.clone(), facts);
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
            let started = start(
                store,
                backlog,
                &owner,
                &path,
                limits.max_line,
                memory,
                programs,
                digest,
                budget,
            );
            let started = match started {
                Ok(started) => started,
                Err(error) => {
                    // Looked at again next pass.
                    backlog.due.push_front((owner, path));
                    return Err(error);
                }
            };
            match started {
                Start::Reading(reading) => backlog.reading = Some(reading),
                Start::Unchanged => {}
                Start::Deferred => {
                    backlog.due.push_front((owner, path));
                    return Ok(());
                }
                Start::Saved(facts) => {
                    // What a read would find again, as an earlier run saved
                    // it.
                    progress.files_skipped += 1;
                    keep(backlog, path, *facts);
                }
                Start::Missing => {
                    // Deleted history: what was read of it is kept, and its
                    // caller's indexed span is a gap.
                    forget(backlog, &path);
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
                memory,
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
    /// Read to its end by an earlier run, which saved what it found.
    Saved(Box<FileFacts>),
    /// What is left of the pass's bytes cannot check a saved read: looked at
    /// again next pass.
    Deferred,
    /// It no longer exists: deleted history.
    Missing,
    Failed,
}

#[allow(clippy::too_many_arguments)]
fn start(
    store: &Store,
    backlog: &PromptBacklog,
    owner: &str,
    path: &Path,
    max_line: usize,
    memory: &Allowance,
    programs: &[PathBuf],
    rules: [u8; 32],
    budget: &mut Budget<'_>,
) -> xt_store::Result<Start> {
    let (file, generation) = match source::open(path) {
        Ok(opened) => opened,
        Err(source::Unread::Missing) => return Ok(Start::Missing),
        Err(_) => return Ok(Start::Failed),
    };
    let Ok(length) = u64::try_from(generation.length) else {
        return Ok(Start::Failed);
    };
    // Only what was appended is read, when the file is the same one and the
    // bytes before where it was read up to are unchanged.
    let mut from = 0;
    let mut launches = Vec::new();
    let mut refused = Vec::new();
    let mut untimed = false;
    let mut overfull = false;
    if let Some(known) = backlog.files.get(path) {
        // What was read with other rules or launchers is read again whole.
        if known.owner == owner
            && known.rules == rules
            && known.device == generation.device
            && known.inode == generation.inode
            && known.complete <= length
            && tail(&file, known.complete) == known.tail
        {
            if known.complete == length {
                return Ok(Start::Unchanged);
            }
            from = known.complete;
            launches = known.launches.clone();
            refused = known.refused.clone();
            untimed = known.untimed;
            overfull = known.overfull;
        }
    } else if let Some(saved) = saved_read(store, owner, path, max_line, rules)?
        && saved.device == generation.device
        && saved.inode == generation.inode
        && saved.read_to <= length
    {
        match saved::fingerprint(&file, saved.read_to, budget) {
            Fingerprint::Deferred => return Ok(Start::Deferred),
            // An earlier run read it up to `read_to` and found no launch
            // there.
            Fingerprint::Digest(digest) if digest == saved.fingerprint => {
                if saved.read_to < length {
                    // It grew: only what was appended is read.
                    from = saved.read_to;
                    refused = saved.refused_ms;
                    untimed = saved.untimed;
                } else if saved.mtime_ns == generation.mtime_ns
                    && saved.ctime_ns == generation.ctime_ns
                {
                    // Still exactly the generation that read began at.
                    return Ok(Start::Saved(Box::new(FileFacts {
                        owner: owner.to_owned(),
                        device: generation.device,
                        inode: generation.inode,
                        complete: length,
                        tail: tail(&file, length),
                        launches: Vec::new(),
                        refused: saved.refused_ms,
                        untimed: saved.untimed,
                        overfull: false,
                        rules,
                    })));
                }
                // The same length at another time: rewritten in place, read
                // whole.
            }
            Fingerprint::Digest(_) | Fingerprint::Unreadable => {}
        }
    }
    let Ok(held) = memory.reserve(path.as_os_str().len() + max_line.min(1 << 20)) else {
        return Ok(Start::Failed);
    };
    Ok(Start::Reading(Box::new(FileRead {
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
        generation,
        programs: programs.to_vec(),
        rules,
        _held: held,
    })))
}

/// The read an earlier run saved of `owner`'s file at `path`, if it was read
/// by these rules, with this line bound and these launchers installed.
fn saved_read(
    store: &Store,
    owner: &str,
    path: &Path,
    max_line: usize,
    rules: [u8; 32],
) -> xt_store::Result<Option<SavedTranscriptRead>> {
    let Some(text) = path.to_str() else {
        return Ok(None);
    };
    Ok(store
        .saved_transcript_read(TranscriptReader::Prompt, text)?
        .filter(|saved| {
            saved.owner_native_session_id == owner
                && saved.reader_version == PROMPT_READ_VERSION
                && saved.max_line == max_line as u64
                && saved.rules_digest == rules
                && saved.refused_ms.len() <= MAX_FILE_LAUNCHES
        }))
}

/// Keep what a file was read, or saved, to hold: something new for the
/// match.
fn keep(backlog: &mut PromptBacklog, path: PathBuf, facts: FileFacts) {
    backlog.dirty = true;
    if backlog.files.len() >= MAX_FILES && !backlog.files.contains_key(&path) {
        // Forgotten launches could have made any child.
        backlog.files.clear();
        backlog.overflowed = true;
    }
    backlog.unreadable.remove(&path);
    backlog.files.insert(path, facts);
}

/// Save a whole read of a file that held no launch, so a restart need not
/// read it again; forget the saved read of one that holds launches, held
/// more than this version reads, or changed while it was read.
fn note_read(
    backlog: &mut PromptBacklog,
    reading: &FileRead,
    facts: &FileFacts,
    max_line: usize,
    budget: &mut Budget<'_>,
) {
    let path = reading.path.as_path();
    let Some(text) = path.to_str() else {
        return;
    };
    let unchanged = source::generation_of(&reading.file).ok() == Some(reading.generation);
    // A read not saved for want of bytes is only read again after a restart.
    let fingerprint = match saved::fingerprint(&reading.file, facts.complete, budget) {
        Fingerprint::Digest(digest) => Some(digest),
        Fingerprint::Unreadable | Fingerprint::Deferred => None,
    };
    let (true, false, true, Some(fingerprint)) = (
        unchanged,
        facts.overfull,
        facts.launches.is_empty(),
        fingerprint,
    ) else {
        forget(backlog, path);
        return;
    };
    // The latest read of a file is the one saved.
    backlog.forget.retain(|(_, known)| known != text);
    backlog.unsaved.retain(|saved| saved.path != text);
    backlog.unsaved.push(SavedTranscriptRead {
        reader: TranscriptReader::Prompt,
        path: text.to_owned(),
        owner_native_session_id: facts.owner.clone(),
        reader_version: PROMPT_READ_VERSION,
        max_line: max_line as u64,
        rules_digest: reading.rules,
        device: facts.device,
        inode: facts.inode,
        read_to: facts.complete,
        mtime_ns: reading.generation.mtime_ns,
        ctime_ns: reading.generation.ctime_ns,
        fingerprint,
        untimed: facts.untimed,
        refused_ms: facts.refused.clone(),
    });
}

/// Forget the saved read of the file at `path`.
fn forget(backlog: &mut PromptBacklog, path: &Path) {
    if let Some(text) = path.to_str() {
        backlog.unsaved.retain(|saved| saved.path != text);
        if !backlog.forget.iter().any(|(_, known)| known == text) {
            backlog
                .forget
                .push((TranscriptReader::Prompt, text.to_owned()));
        }
    }
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
        home: &Path,
        max_line: usize,
    ) -> Result<bool, ()> {
        let programs = self.programs.clone();
        let programs = programs.as_slice();
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
            // Assignments alone run nothing, but a value naming the Claude
            // program may be run later.
            refused |= names && carries_claude(&simple.words, &simple.data);
            continue;
        }
        let Some((program, args)) = words::program(&simple.words) else {
            // A wrapper this reader cannot see through.
            refused |= names && (mentions(&simple.words) || carries_claude(&[], &simple.data));
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
            words::Word::Opaque | words::Word::Fields | words::Word::Assign(_) => false,
        };
        if !claude {
            refused |= names && hides_claude(program, args, simple);
            continue;
        }
        // A word the shell may turn into no word or several may spell any
        // option (`--model $M` with `M='x --continue'`), and an expanding
        // word after the program that looks like an assignment is split
        // too: where the launch's words are is not known.
        if args
            .iter()
            .any(|word| matches!(word, words::Word::Fields | words::Word::Assign(None)))
        {
            refused = true;
            continue;
        }
        let args: Vec<Option<String>> = args
            .iter()
            .map(|word| match word {
                words::Word::Lit(text) => Some(text.clone()),
                words::Word::Assign(text) => text.clone(),
                words::Word::Home(_) | words::Word::Opaque | words::Word::Fields => None,
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
        words::Word::Opaque | words::Word::Fields | words::Word::Assign(None) => false,
    })
}

/// Whether words or data text name the Claude program as a command would
/// ([`words::names_claude`]).
fn carries_claude(all: &[words::Word], data: &[String]) -> bool {
    all.iter().any(|word| match word {
        words::Word::Lit(text) | words::Word::Home(text) | words::Word::Assign(Some(text)) => {
            words::names_claude(text)
        }
        words::Word::Opaque | words::Word::Fields | words::Word::Assign(None) => false,
    }) || data.iter().any(|text| words::names_claude(text))
}

/// Whether a simple command whose program is not a known Claude launcher
/// may still run Claude, in a command naming `claude`: the one rule for text
/// this reader takes as data. A program the shell supplies, given a print
/// option or text naming the Claude program; a program named `claude` this
/// reader does not know; a shell given a script or input this reader did
/// not read; or any other program but a few that never run what they are
/// given ([`NON_RUNNERS`]), given a word, a here-document or other data text
/// naming the Claude program (`sudo claude`, `xargs claude`,
/// `tmux new-session 'claude -p x'`, `python3 - <<EOF … claude … EOF`).
fn hides_claude(program: &words::Word, args: &[words::Word], simple: &words::Command) -> bool {
    let name = match program {
        words::Word::Opaque | words::Word::Fields => {
            return args.iter().any(|word| {
                matches!(word, words::Word::Lit(text)
                    if text == "-p" || text == "--print" || text.starts_with("--print="))
            }) || carries_claude(args, &simple.data);
        }
        words::Word::Lit(text) | words::Word::Home(text) => text.as_str(),
        words::Word::Assign(_) => return false,
    };
    let base = name.rsplit('/').next().unwrap_or(name);
    if base == "claude" {
        return true;
    }
    if SHELLS.contains(&base) {
        if words::shell_script(&simple.words).is_some() {
            // Its script was read as commands; what else it is given was not.
            return carries_claude(simple.words.get(3..).unwrap_or_default(), &simple.data);
        }
        // A script naming Claude, or input it reads.
        return mentions(args)
            || carries_claude(&[], &simple.data)
            || args
                .iter()
                .all(|word| matches!(word, words::Word::Lit(text) if text.starts_with('-')));
    }
    if NON_RUNNERS.contains(&base) {
        return false;
    }
    // What it is given, and the assignments before it, which it may run.
    carries_claude(&simple.words, &simple.data)
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
        let store = Store::open_in_memory().unwrap();
        let Ok(Start::Reading(mut reading)) = start(
            &store,
            &backlog,
            "owner",
            &path,
            max_line,
            &memory,
            &[],
            saved::rules_digest(&[]),
            &mut Budget::new(1 << 20, Duration::from_secs(5), None),
        ) else {
            panic!("the transcript is read");
        };
        let mut budget = Budget::new(first, Duration::from_secs(5), None);
        assert_eq!(
            reading.advance(&mut budget, temp.path(), max_line),
            Ok(false)
        );
        let mut budget = Budget::new(u64::MAX, Duration::from_secs(5), None);
        assert_eq!(
            reading.advance(&mut budget, temp.path(), max_line),
            Ok(true)
        );
        reading.overfull
    }

    /// How many literal launches one recorded command holds, and whether it
    /// is a command the reader cannot read.
    fn read(command: &str) -> (usize, bool) {
        let read = read_command(command, &[], Path::new("/home/synthetic"));
        (read.launches.len(), read.refused)
    }

    #[test]
    fn text_skipped_or_taken_as_data_naming_claude_is_a_gap() {
        for command in [
            // Expansions the shell runs, which this reader does not read.
            "OUTPUT=${OUTPUT:-$(claude -p 'same prompt')}",
            "${V:-$(claude -p x)}",
            "echo \"${V:-$(claude -p x)}\"",
            "n=$(( $(claude -p x | wc -c) ))",
            "n=$[ $(claude -p x | wc -c) ]",
            // Scripts and data given to a program that may run them.
            "bash -euc \"claude -p x\"",
            "bash -euc 'claude -p x'",
            "tmux new-session -d 'claude -p x'",
            "tmux new-session -d \"cd $S && claude -p x\"",
            "tmux new-session -d $'claude -p x'",
            "python3 - <<'EOF'\nimport subprocess\nsubprocess.run([\"claude\", \"-p\", \"x\"])\nEOF",
            "node -e 'spawn(\"/opt/homebrew/bin/claude\", [\"-p\", \"x\"])'",
            "bash -c 'cat' <<EOF\nclaude -p x\nEOF",
            "bash -c '\"$@\"' _ claude -p x",
            "RUN=\"claude -p x\"; $RUN",
            "$RUNNER \"claude -p x\"",
            "x=( claude -p arr )",
            "C=${C:-claude}",
            "python3 <<< 'import os; os.system(\"claude -p x\")'",
            "find /w -name '*.md' -exec sh -c 'claude -p x' \\;",
            // A script deeper than the reader reads.
            "sh -c \"sh -c 'sh -c \\\"sh -c claude\\\"'\"",
        ] {
            assert_eq!(read(command), (0, true), "{command:?}");
        }
        // A gap does not hide the launches the reader did read.
        assert_eq!(
            read("claude -p a; tmux new-session -d 'claude -p b'"),
            (1, true)
        );
    }

    #[test]
    fn text_that_only_names_claude_or_is_read_is_no_gap() {
        for command in [
            "echo \"claude -p x\"",
            "grep 'claude -p' f",
            "# claude -p x",
            "ls ~/.claude",
            "ls ~/.claude/projects && cat ~/.claude/settings.json",
            "command -v claude",
            "cat > /w/a.py <<'EOF'\nclaude -p \"inside\"\nEOF",
            "git commit -m \"$(cat <<'EOF'\nRun claude -p in a test\nEOF\n)\"",
            "echo \"$S claude -p x\"",
            "python3 /w/show.py /w/run1.jsonl 260 | cut -c1-320",
            "tmux new-session -d 'claude-code --help'",
            "echo ${HOME}/.claude",
            "n=$(( 1 << 2 ))",
            "grep -c x <<< 'claude -p x'",
        ] {
            assert_eq!(read(command), (0, false), "{command:?}");
        }
        // A launch's own words, its prompt among them, are what it reads.
        for command in [
            "claude -p \"ask claude to review\"",
            "sh -c 'claude -p x'",
            "bash -lc 'cd /w && claude -p \"ask claude\"'",
            "out=$(claude -p x 2>&1); echo \"$out\"",
            "cd \"$S\" && claude -p \"literal prompt\" --name \"x\" --mcp-config \"$S/mcp.json\" \
             --strict-mcp-config --allowedTools \"mcp__x__*\" --setting-sources project \
             --output-format stream-json --verbose > \"$S/run1.jsonl\" 2> \"$S/run1.err\"; \
             echo \"exit $?\"; python3 \"$S/show.py\" \"$S/run1.jsonl\" 260 | cut -c1-320",
        ] {
            assert_eq!(read(command), (1, false), "{command:?}");
        }
    }

    /// Whether each literal launch one recorded command holds reads only
    /// its call's input, with `programs` as the installed launchers.
    fn own_inputs(command: &str, programs: &[&str]) -> (Vec<bool>, bool) {
        let programs: Vec<PathBuf> = programs.iter().map(PathBuf::from).collect();
        let read = read_command(command, &programs, Path::new("/home/synthetic"));
        (
            read.launches
                .iter()
                .map(|launch| launch.own_input)
                .collect(),
            read.refused,
        )
    }

    #[test]
    fn a_launch_word_the_shell_may_split_is_a_gap() {
        // An expansion that may become no word or several may spell an
        // option that resumes (`MODEL='sonnet --continue'`).
        for command in [
            "claude --model $MODEL -p 'same prompt'",
            "claude -p 'x' --mcp-config $CFG",
            "claude -p x --tools ${T}",
            "claude -p x --model $(cat /w/model)",
            "claude -p x --model `cat /w/model`",
            "claude -p x --max-turns $((N + 1))",
            "claude -p x --model $1",
            "claude -p x --model \"$@\"",
            "claude -p x --add-dir \"${dirs[@]}\"",
            "claude -p x --add-dir *",
            "claude -p x --add-dir /w/{a,b}",
            "claude -p x --model a=$B",
            // zsh's splitting and glob flags, also quoted, and its subscripts.
            "claude --model $=M -p x",
            "claude --model $~M -p x",
            "claude --model $^M -p x",
            "claude --model \"$=M\" -p x",
            "claude --model \"${=M}\" -p x",
            "claude --model \"${(f)M}\" -p x",
            "claude --model \"${(s: :)M}\" -p x",
            "claude --model \"${(z)M}\" -p x",
            "claude --model \"${~M}\" -p x",
            "claude --model \"$M[@]\" -p x",
            // A zsh form nested in a quoted default.
            "claude --model \"${U:-${=M}}\" -p x",
            "claude --model \"${U:-$=M}\" -p x",
            "claude --model \"${U-$=M}\" -p x",
            "claude --model \"${M:+$=M}\" -p x",
            "claude --model \"${U:-${(f)X}}\" -p x",
            "S=/w; claude -p x --mcp-config $=S/c.json",
            // A variable the command changes, or that may not be one word.
            "S=/w; S+=' --continue'; claude -p x --model $S",
            "S=/w; read S; claude -p x --model $S",
            "S=/w; export S; claude -p x --model $S",
            "S=/w; claude -p x --model ${S:-y}",
            "S='a b'; claude -p x --model $S",
            "S=; claude -p x --model $S",
            "IFS=/; S=/w; claude -p x --mcp-config $S/c.json",
            "S=/w; IFS=/; claude -p x --mcp-config $S/c.json",
            "S=/w; eval \"$X\"; claude -p x --mcp-config $S/c.json",
            "S=/w; . /w/env; claude -p x --mcp-config $S/c.json",
            "S=/w; . ./env; claude -p x --mcp-config $S/c.json",
            "S=/w; read \"$N\"; claude -p x --mcp-config $S/c.json",
            "S=/w; printf -v \"$N\" x; claude -p x --mcp-config $S/c.json",
            "S=/w; $CMD; claude -p x --mcp-config $S/c.json",
            "S=/w; (eval \"$X\"; claude -p x --mcp-config $S/c.json)",
            "false && S=/w; claude -p x --mcp-config $S/c.json",
            "(S=/w); claude -p x --mcp-config $S/c.json",
            "S=/w || true; claude -p x --mcp-config $S/c.json",
            "PATH=/w; claude -p x --mcp-config $PATH/c.json",
            // An `sh -c` script does not see an unexported variable.
            "S=/w bash -c 'claude -p x --mcp-config $S/c.json'",
            "S=/w; bash -c 'claude -p x --mcp-config $S/c.json'",
        ] {
            assert_eq!(read(command), (0, true), "{command:?}");
        }
        // One word whatever its value: quoted, or a variable the command set
        // to a plain value at its start and never touches again.
        for command in [
            "claude --model \"$MODEL\" -p 'x'",
            "claude -p x --model \"$*\" --name \"${N}\" --add-dir ~/w",
            "S=/w/run; cd $S && claude -p 'x' --mcp-config $S/test.mcp.json --strict-mcp-config",
            "S=/w && cd \"$S\" && claude -p 'x' --mcp-config \"$S/mcp.json\"",
            "  S=/w T='/w/t' ; claude -p x --mcp-config ${S}/c.json --add-dir $T",
            "S=/w; claude -p x --mcp-config $S/c.json; echo \"$(ls $S)\"",
            // Prompt words are never programs.
            "S=/w; claude -p 'Fix it . Then read the source, eval it.' --mcp-config $S/c.json",
            "S=/w; cd . && claude -p \"source . eval read\" --mcp-config $S/c.json",
        ] {
            assert_eq!(read(command), (1, false), "{command:?}");
        }
        // The two shapes of an agent's real test runs.
        let launcher = "/home/synthetic/.local/bin/claude";
        for command in [
            format!(
                "S=/private/tmp/synthetic/scratchpad; cd $S/p3b-run && {launcher} -p \"Check the \
                 synthetic tool.\" --name \"p3b run\" --mcp-config $S/p3b-run/test.mcp.json \
                 --strict-mcp-config --setting-sources project --output-format stream-json \
                 --verbose > run1.jsonl 2> run1.err; echo \"exit $?\""
            ),
            "S=/private/tmp/synthetic; cd $S/trialcwd && claude -p \"Run the synthetic trial.\" \
             --name \"trial\" --mcp-config $S/trial-mcp.json --strict-mcp-config \
             --output-format stream-json --verbose > $S/trial.jsonl 2>&1"
                .to_owned(),
        ] {
            assert_eq!(
                own_inputs(&command, &[launcher]),
                (vec![true], false),
                "{command:?}"
            );
        }
    }

    #[test]
    fn a_launch_in_a_process_substitution_is_given_other_input() {
        for command in [
            "printf extra > >(claude -p 'same prompt')",
            "cat f | cat <(claude -p 'same prompt')",
        ] {
            assert_eq!(
                own_inputs(command, &[]),
                (vec![false], false),
                "{command:?}"
            );
        }
        assert_eq!(
            own_inputs("cat <(echo a) && claude -p 'same prompt'", &[]),
            (vec![true], false)
        );
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
