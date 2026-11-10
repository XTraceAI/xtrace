//! Parent links for Claude sessions a Claude agent launched with `claude -p`
//! from its own `Bash` tool, matched by their exact prompt.
//!
//! An agent's real launch commands seldom name the new session, and their
//! output usually goes to a file or a background job, so the stricter
//! [`super`] child facts (which compare the printed answer) cannot see them.
//! This match links a child only when all of these hold:
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
//!    unlinked. A link that later stops being so is withdrawn for good.
//! 5. The parent, the session owning the transcript, is exactly one indexed
//!    Claude user session other than the child.
//! 6. The launch reads only the input its `Bash` call was given
//!    ([`words::Command::own_input`]): no `<` redirection or here-document,
//!    and not after a `|`. A launch given other input may have sent its child
//!    more than the prompt, so it never makes a link; it still counts as a
//!    launch that could have made a child, so it can stop a link.
//! 7. Nothing the link could rest on is missing from what was read. A
//!    caller's transcript or helper folder that could not be read whole, or
//!    held more than this version keeps, may hide a launch made at or after
//!    that caller's first record: no new link is written for a child saved at
//!    or after it. A possible child whose opening could not be read may be
//!    the child of any launch made at or before it was saved: no such launch
//!    makes a new link. Too many files or sessions to keep stop every new
//!    link. A stored link stays, unless what was read withdraws it. What
//!    could not be read is read again once a scan imports anything, and a
//!    later match decides again. A transcript that no longer exists is
//!    history that was deleted, like one the index never located: its
//!    launches are not known and stop nothing, while a possible child that
//!    no longer exists still stops the launches that could have made it.
//! 8. The child is not already related, or named as a child, by any other
//!    evidence, such as a Codex launch naming its session: the store abstains
//!    ([`Store::record_claude_prompt_launch`]).
//!
//! The child fact and the relation are written together
//! ([`Store::record_claude_prompt_launch`]). Work lives in memory in a
//! [`PromptBacklog`] and is done in bounded passes after the other `Bash`
//! work: a startup sweep over every indexed caller, the sessions each scan
//! imports, then one match over every launch found. A transcript is read
//! once and then only what is appended to it. Only digests of prompts are
//! kept in memory; identifiers and positions are stored.

mod words;

use super::super::options::{self, Action};
use super::super::source::{self, Allowance, Budget, Reserved};
use super::super::{LaunchLimits, command, rows};
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
    Host, Store,
    child_fact::ChildFactDisposition,
    creation::{CLAUDE_PROMPT_LAUNCH_VERSION, ClaudePromptLaunchProof, CreationDisposition},
};

/// Callers one sweep page reads from the index.
const CALLER_PAGE: usize = 256;
/// Transcript files remembered; more forget them all (each is read again).
const MAX_FILES: usize = 100_000;
/// Helper-agent files one caller may have; more are not read.
const MAX_HELPERS: usize = 4_096;
/// Launches one file may hold; a file with more is not used.
const MAX_FILE_LAUNCHES: usize = 10_000;
/// Indexed sessions a match looks at as possible children; more decide
/// nothing.
const MAX_CHILDREN: usize = 100_000;
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
    /// A line was passed over, or launches dropped, since its first byte.
    overfull: bool,
    /// Commands naming `claude` that the word reader refused.
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
    /// What its own transcript's opening says; `None` when it holds no one
    /// text there or could not be read.
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
    /// Something was read or imported since the last match.
    dirty: bool,
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
    /// More callers or files were seen than this version keeps, so evidence
    /// was forgotten: no new link is written for the rest of this run.
    overflowed: bool,
    /// Possible children of the match in progress whose opening could not be
    /// read, and when each first saved a record.
    child_gaps: Vec<(String, i64)>,
    /// The possible children of the match in progress could not all be
    /// listed or told apart.
    children_unknown: bool,
}

/// What one pass of literal-prompt launch work did. Counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptProgress {
    /// Transcript files read, whole or what was appended.
    pub files_read: usize,
    /// Files that could not be read now.
    pub files_unreadable: usize,
    /// Matches run over every launch found.
    pub matches: usize,
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
    /// Stored links withdrawn.
    pub withdrawn: usize,
    /// Rows that changed what a read shows.
    pub changed: usize,
    /// Launches with one possible child that were given other input (a `<`
    /// redirection, a here-document or a pipe), so make no link.
    pub other_input: usize,
    /// Links not written because some evidence could not be read whole.
    pub held: usize,
    /// Matches that ran with some evidence missing.
    pub incomplete_matches: usize,
    /// Possible children whose opening could not be read.
    pub children_unreadable: usize,
    /// `Bash` commands naming `claude` that the word reader refused.
    pub commands_refused: usize,
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

    /// The callers whose launches may not all have been read: those owning a
    /// file that could not be read whole, or a helper folder that could not
    /// be listed whole.
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

    /// Whether a later pass has anything to do. While it has, a display
    /// check of a Claude session waits: a launch not read or matched yet may
    /// have made it.
    pub fn pending(&self) -> bool {
        self.sweep.is_some()
            || !self.queue.is_empty()
            || !self.due.is_empty()
            || self.reading.is_some()
            || self.dirty
            || self.children.is_some()
    }

    /// Whether a display check must wait for this work now; one that must
    /// is remembered, so that the checks wake once the work is done.
    pub fn holds_checks(&mut self) -> bool {
        let pending = self.pending();
        self.waited |= pending;
        pending
    }

    /// Whether this work finished since a display check waited for it.
    pub fn take_wake(&mut self) -> bool {
        std::mem::take(&mut self.wake)
    }
}

/// One bounded pass, within what is left of `budget`.
pub(in crate::native::claude_launch) fn pass(
    store: &mut Store,
    home: &Path,
    backlog: &mut PromptBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    recorded_at: i64,
    progress: &mut PromptProgress,
) -> xt_store::Result<()> {
    let memory = backlog
        .memory
        .get_or_insert_with(|| Allowance::new(limits.thread_memory))
        .clone();
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
                    let overfull = reading.overfull || reading.launches.len() > MAX_FILE_LAUNCHES;
                    let launches = if overfull {
                        Vec::new()
                    } else {
                        reading.launches
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
                    if backlog.files.remove(&path).is_some() {
                        backlog.dirty = true;
                    }
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
            if let Some(path) = transcript(store, home, &native)? {
                let helpers = path.with_extension("").join("subagents");
                let mut found = Vec::new();
                let mut whole = true;
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
                            let name = entry.file_name();
                            let name = name.to_string_lossy();
                            if !(name.starts_with("agent-") && name.ends_with(".jsonl")) {
                                continue;
                            }
                            if found.len() >= MAX_HELPERS {
                                whole = false;
                                break;
                            }
                            found.push(entry.path());
                        }
                    }
                    // No helper agents.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => whole = false,
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
            }
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
        if backlog.dirty || backlog.children.is_some() {
            if !read_children(store, home, backlog, limits, budget, &memory, progress)? {
                return Ok(());
            }
            decide(store, backlog, recorded_at, progress)?;
            backlog.dirty = false;
            progress.matches += 1;
            if backlog.waited && !backlog.pending() {
                backlog.waited = false;
                backlog.wake = true;
            }
        }
        break;
    }
    Ok(())
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

    /// The launches one complete line holds.
    fn line(&mut self, bytes: &[u8], programs: &[PathBuf], home: &Path) {
        // A launch names `claude`; a line that escapes characters may spell
        // it otherwise, and is read whole.
        if self.launches.len() > MAX_FILE_LAUNCHES {
            self.overfull = true;
            return;
        }
        if !(contains(bytes, b"claude") || contains(bytes, b"\\u")) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return;
        };
        // Only an assistant record of the session owning the transcript.
        if value.get("type").and_then(Value::as_str) != Some("assistant")
            || value.get("sessionId").and_then(Value::as_str) != Some(self.owner.as_str())
        {
            return;
        }
        let Some(launch_ms) = rows::timestamp(&value) else {
            return;
        };
        let Some(blocks) = value["message"]["content"].as_array() else {
            return;
        };
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Bash")
            {
                continue;
            }
            let Some(id) = block
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| token(id))
            else {
                continue;
            };
            let Some(command) = block["input"].get("command").and_then(Value::as_str) else {
                continue;
            };
            let Some(found) = launches(command, programs, home) else {
                if command.contains("claude") {
                    self.refused_commands += 1;
                }
                continue;
            };
            for (index, launch) in found.into_iter().enumerate() {
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

/// The create-mode Claude print launches with a literal prompt one command
/// holds, in order: each prompt's digest, the session it names and whether
/// it reads only its call's input. `None` when the command cannot be read.
fn launches(command: &str, programs: &[PathBuf], home: &Path) -> Option<Vec<Literal>> {
    let commands = words::simple_commands(command)?;
    let mut found = Vec::new();
    for simple in &commands {
        let Some((program, args)) = words::program(&simple.words) else {
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
            continue;
        };
        if read.action != Action::Create {
            continue;
        }
        let Some(prompt) = read.prompt.filter(|prompt| !prompt.is_empty()) else {
            continue;
        };
        found.push(Literal {
            digest: sha2::Sha256::digest(prompt.as_bytes()).into(),
            session_id: read.session_id,
            own_input: simple.own_input,
        });
    }
    Some(found)
}

/// Read the opening of every indexed Claude session that may be a child of
/// a launch found: those first saving an input at or after the earliest
/// launch. `false` when the budget ended first; the reads resume next pass.
/// An opening that cannot be read now is a gap of this match, never kept:
/// the next match reads it again.
fn read_children(
    store: &Store,
    home: &Path,
    backlog: &mut PromptBacklog,
    limits: &LaunchLimits,
    budget: &mut Budget<'_>,
    memory: &Allowance,
    progress: &mut PromptProgress,
) -> xt_store::Result<bool> {
    if backlog.children.is_none() {
        backlog.child_gaps.clear();
        backlog.children_unknown = false;
        let earliest = backlog
            .files
            .values()
            .flat_map(|facts| facts.launches.iter().map(|found| found.launch_ms))
            .min();
        let Some(earliest) = earliest else {
            backlog.children = Some(VecDeque::new());
            return Ok(true);
        };
        let born = store.claude_sessions_born_between(earliest, i64::MAX, MAX_CHILDREN + 1)?;
        // Too many to look at: nothing is decided.
        let born: VecDeque<String> = if born.len() > MAX_CHILDREN {
            backlog.children_unknown = true;
            VecDeque::new()
        } else {
            born.into_iter().map(|(native, _)| native).collect()
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
            // More than one indexed session of this identity: which one
            // holds which input is not known.
            if sessions.len() > 1 {
                backlog.children_unknown = true;
                progress.children_unreadable += 1;
            }
            backlog.openings.remove(&native);
            continue;
        };
        let (Some((index_uuid, _)), Some(first_record_ms)) = (
            store.first_input_record(session)?,
            store.session_first_record_ms(session)?,
        ) else {
            backlog.openings.remove(&native);
            continue;
        };
        let Some(path) = transcript(store, home, &native)? else {
            backlog.openings.remove(&native);
            continue;
        };
        let Ok(now) = source::stat(&path) else {
            backlog.openings.remove(&native);
            backlog.child_gaps.push((native, first_record_ms));
            progress.children_unreadable += 1;
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
            Err(_) => {
                backlog.openings.remove(&native);
                backlog.child_gaps.push((native, first_record_ms));
                progress.children_unreadable += 1;
            }
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

/// Match every launch found with its possible children, link the launches
/// with exactly one that no other launch could have made, and withdraw a
/// stored link that is no longer so.
fn decide(
    store: &mut Store,
    backlog: &mut PromptBacklog,
    recorded_at: i64,
    progress: &mut PromptProgress,
) -> xt_store::Result<()> {
    let children = backlog.children.take().unwrap_or_default();
    debug_assert!(children.is_empty());
    // Only the sessions this match listed are possible children.
    let listed = std::mem::take(&mut backlog.listed);
    backlog.openings.retain(|native, _| listed.contains(native));
    // Where evidence is missing, a launch that looks unique may not be: a
    // caller not read whole may hide a launch that made any child saved at
    // or after its first record, and every child, when too much was seen
    // to keep. A stored link may still be withdrawn by what was read.
    let mut hidden_from: Option<i64> =
        (backlog.overflowed || backlog.children_unknown).then_some(i64::MIN);
    for owner in backlog.hidden_callers() {
        // A caller with no timed record could have made any child.
        let mut first = None;
        let mut untimed = false;
        for session in store.user_sessions_with_native(Host::Claude, owner)? {
            match store.session_first_record_ms(&session)? {
                Some(ms) => first = Some(first.map_or(ms, |known: i64| known.min(ms))),
                None => untimed = true,
            }
        }
        let first = if untimed {
            i64::MIN
        } else {
            first.unwrap_or(i64::MIN)
        };
        hidden_from = Some(hidden_from.map_or(first, |known| known.min(first)));
    }
    if hidden_from.is_some() || !backlog.child_gaps.is_empty() {
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
        // A child stored as another launch's, even one since withdrawn, is
        // not linked again: a link that stops being sure is gone for good.
        if stored.iter().any(|(proof, _)| {
            proof.child_native_session_id == child
                && !(proof.parent_native_session_id == launch.owner
                    && proof.launch_call_id == launch.found.call_id
                    && proof.launch_operation_index == launch.found.index)
        }) {
            progress.ambiguous += 1;
            continue;
        }
        // A child saved after a hidden launch could be its child, and a
        // possible child that could not be read could be this launch's.
        let born = backlog.openings[child].first_record_ms;
        if hidden_from.is_some_and(|from| born >= from)
            || backlog.child_gaps.iter().any(|(native, first_record_ms)| {
                could_make(launch.owner, launch.found, native, *first_record_ms)
            })
        {
            progress.held += 1;
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
    // A stored link whose launch is still found and now keeps another
    // session, or whose child another launch keeps.
    let mut withdraw = Vec::new();
    for (stored, accepted) in stored {
        if !accepted {
            continue;
        }
        let at = launches.iter().position(|launch| {
            launch.owner == stored.parent_native_session_id
                && launch.found.call_id == stored.launch_call_id
                && launch.found.index == stored.launch_operation_index
        });
        let owner = owners.get(stored.child_native_session_id.as_str());
        let doubtful = at.is_some_and(|at| {
            kept[at].len() > 1 || owner.is_some_and(|owner| owner.as_slice() != [at])
        }) || owner.is_some_and(|owner| owner.len() > 1);
        if doubtful {
            withdraw.push(stored);
        }
    }
    drop(launches);
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
    for stored in withdraw {
        let changed = store.withhold_claude_prompt_launch(&stored)?;
        progress.changed += changed;
        progress.withdrawn += usize::from(changed > 0);
    }
    Ok(())
}
