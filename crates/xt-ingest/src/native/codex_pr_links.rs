//! Exact pull-request links for Codex sessions, from the Codex app's own
//! record that a thread attached a pull request.
//!
//! The Codex desktop app writes, in a thread's own rollout, one completed
//! tool call when it attaches a pull request to the thread:
//!
//! ```text
//! {"timestamp": "...", "type": "event_msg", "payload": {"type": "item_completed",
//!   "thread_id": "<thread>", "item": {"type": "McpToolCall", "server": "codex_app",
//!   "tool": "attach_artifact", "arguments": {"artifact_type": "pull_request",
//!   "url": "https://github.com/<owner>/<repo>/pull/<number>"},
//!   "status": "completed", "result": {"isError": false, ...}}}}
//! ```
//!
//! That line, and nothing else, is the witness ([`codex_pr_witness`]). The
//! same text copied into a `compacted` line's replacement history, a
//! `response_item`, a call that did not complete or failed, another artifact
//! type, server or tool, or a line naming another thread (history a fork
//! copied from the thread it was forked from) links nothing. The pull request
//! is the URL's canonical identity under the Store's own rules
//! ([`PrIdentity`]); the link's time is the line's own `timestamp`, never the
//! time of the read. A link is written through the Store's one link write
//! ([`Store::record_pr_link`]) with confidence `exact`, so a repeated witness
//! only widens the first/last seen times, and an unchanged one changes
//! nothing.
//!
//! A thread is read only once the index holds its one user session; the
//! files read are every rollout the index recorded for it, root and
//! continuations, however many there are ([`codex_sources`]: the title
//! read's exact matching without its cap), reached as the title read reaches
//! them. A long history is read across passes by its files' marks. Threads are
//! queued in a [`PrBacklog`] by every Codex scan that indexed them, and by a
//! sweep over every indexed Codex locator that each watcher run makes once,
//! from the first. A scan only queues: the threads are read in the watcher's
//! background passes that follow it, so reading never lengthens a scan. A
//! thread whose session is not indexed yet is dropped from the queue without
//! reading anything; the scan that indexes it queues it again, so its
//! witnesses are read then.
//!
//! Each file is read incrementally: the backlog keeps, in memory, where the
//! last read of each file ended, and a later read starts there, so a growing
//! rollout costs only what was appended. The marks are not stored: after a
//! restart the sweep reads every indexed rollout once again, and the link
//! write's idempotence makes that re-read change nothing. Reading is bounded
//! by the pass's byte budget, deadline and cancel; a file larger than what
//! is left is read in parts across passes. No part of a file is read when
//! what is left of a pass is less than the rest of the file and no more than
//! the line bound (1 MiB): such a pass reads nothing and reports no
//! progress, the thread keeps its place, and a later pass with more left
//! reads on. Any other part read is either the whole rest of the file or
//! longer than the bound, so it holds a line end or lies inside a line
//! longer than the bound, whose rest is skipped: it moves the mark, unless
//! all it holds is the file's unfinished last line.
//!
//! # Progress across passes
//!
//! One record holds all reading progress: the mark map, one mark per file,
//! the offset after the last complete line read (or inside a line too long
//! to inspect). Every read of a file starts from its mark and saves its new
//! mark there, whether the read reached the file's end or stopped partway.
//! What the next pass does, for each state:
//!
//! - **Thread queued, not yet started.** When it reaches the front, its
//!   recorded rollouts are listed afresh and each is read from its mark, or
//!   from its start without one.
//! - **Thread partly read** (a pass stopped inside it). It stays at the front
//!   and is resumed first, listed afresh. A file read to its end before is
//!   read from its mark, so only what was appended since is read: a witness
//!   appended to it meanwhile is found now, not when the thread is next
//!   queued.
//! - **File partly read.** Its mark says where the read stopped; the next
//!   pass continues from exactly there.
//! - **File read to its end.** Its mark is its end; a later read reads only
//!   what was appended.
//! - **Mark evicted.** The map holds at most a fixed number of marks beside
//!   the front thread's. The files the front thread listed in its latest
//!   read are protected until it leaves the queue (on every path: decided,
//!   not indexed, no single session, nothing recorded or readable), so no
//!   mark of the thread being read is ever evicted, and while it is read
//!   the map may hold its limit plus that thread's file count. Eviction
//!   removes the oldest unprotected mark, one at a time, never all of
//!   them; after the front thread leaves, the map is trimmed back to its
//!   limit the same way. An evicted mark belongs to a thread no longer being
//!   read; that file is read again from its start when its thread is next
//!   queued, which only re-records the same links.
//! - **Budget or deadline reached.** The pass stops with the mark saved;
//!   only the start of the line it stopped inside is read again. With room
//!   for more than one line a pass always moves a mark or finishes a file;
//!   with less it reads nothing and reports no progress (below), so the
//!   watcher backs off rather than spins.
//! - **Cancelled.** A cancel that lands at any point of a read, its last
//!   chunk included, records nothing that read found, and its mark does not
//!   move.
//! - **File replaced, shrunk or unreadable.** Its mark is dropped; the next
//!   read of it starts from its start, a replaced file under its new inode.
//!
//! So a thread being read finishes after finitely many passes with room:
//! each such pass moves a protected mark forward or finishes a file, and no
//! protected mark moves back, whatever the thread's file count and however
//! full the map. Finding the oldest unprotected mark skips the protected
//! ones, at most the front thread's file count per eviction.
//!
//! A mark is trusted while the file keeps its device and inode and is at
//! least as long as the mark. Codex only appends to a rollout; a file
//! rewritten in place under the same inode that grows past the mark would be
//! resumed mid-file, possibly mid-line, until the next app launch, whose
//! sweep reads every file from its start again and records anything missed.

use super::readers_cli::CancelToken;
use super::session_source::host_roots;
use super::session_titles::{
    Batch, ReadMark, TitleLimits, Untitled, codex_contained, codex_path, codex_sources,
    codex_thread, contains, full_thread_id, gather, object,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};
use xt_store::{
    Error, Host, SessionSource, Store,
    pr_link::{PrConfidence, PrIdentity, PrLinkObservation},
};

/// Locators the sweep reads from the index at a time.
const SWEEP_PAGE: usize = 256;

/// The most marks kept beside the front thread's protected ones. Past it the
/// oldest unprotected mark is forgotten, which only costs reading that file
/// again from its start.
pub const MAX_MARKS: usize = 16_384;

/// The one rule for whether a Codex rollout line of the thread `native`
/// witnesses that the thread attached a pull request: that pull request's
/// canonical identity and the line's own time in UTC milliseconds, or none.
///
/// Every condition must hold: top-level `type` is `event_msg`;
/// `payload.type` is `item_completed`; `payload.thread_id` is exactly
/// `native`; `payload.item.type` is `McpToolCall`, its `server` `codex_app`
/// and its `tool` `attach_artifact`; `item.arguments.artifact_type` is
/// `pull_request`; `item.status` is `completed`; `item.result.isError` is not
/// `true`; `item.arguments.url` reconciles to one canonical pull request; and
/// the line's `timestamp` is RFC 3339.
pub fn codex_pr_witness(line: &[u8], native: &str) -> Option<(PrIdentity, i64)> {
    // Cheap first: a witness names its tool, and most lines do not.
    if !contains(line, b"attach_artifact") {
        return None;
    }
    let record = object(line)?;
    if record.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = record.get("payload")?.as_object()?;
    if text(payload.get("type")) != Some("item_completed")
        || text(payload.get("thread_id")) != Some(native)
    {
        return None;
    }
    let item = payload.get("item")?.as_object()?;
    if text(item.get("type")) != Some("McpToolCall")
        || text(item.get("server")) != Some("codex_app")
        || text(item.get("tool")) != Some("attach_artifact")
        || text(item.get("status")) != Some("completed")
        || item
            .get("result")
            .and_then(|result| result.get("isError"))
            .is_some_and(|failed| failed == &Value::Bool(true))
    {
        return None;
    }
    let arguments = item.get("arguments")?.as_object()?;
    if text(arguments.get("artifact_type")) != Some("pull_request") {
        return None;
    }
    let pull_request = PrIdentity::reconcile(Some(text(arguments.get("url"))?), None, None).ok()?;
    let (_, seen_at) = xt_store::timestamp::parse(text(record.get("timestamp"))?).ok()?;
    Some((pull_request, seen_at))
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

/// Codex PR-link work not yet done: threads to read, in the order they
/// arrived, where each file's last read ended, and how far a sweep over
/// every indexed Codex locator has read. It lives with whoever scans, in
/// memory only.
#[derive(Debug)]
pub struct PrBacklog {
    threads: VecDeque<String>,
    waiting: HashSet<String>,
    marks: Marks,
    /// `Some` while a sweep is to finish: the last locator it queued.
    sweep: Option<Option<String>>,
}

impl Default for PrBacklog {
    fn default() -> Self {
        Self {
            threads: VecDeque::new(),
            waiting: HashSet::new(),
            marks: Marks::new(MAX_MARKS),
            sweep: None,
        }
    }
}

/// Every file's mark: the one record of reading progress. Bounded by
/// `limit` beside the `pinned` paths, which are never evicted.
#[derive(Debug)]
struct Marks {
    limit: usize,
    next: u64,
    by_path: HashMap<PathBuf, (u64, ReadMark)>,
    /// Paths by when their mark was first recorded, oldest first.
    order: BTreeMap<u64, PathBuf>,
    /// The front thread's files, from its latest listing.
    pinned: HashSet<PathBuf>,
}

impl Marks {
    fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            next: 0,
            by_path: HashMap::new(),
            order: BTreeMap::new(),
            pinned: HashSet::new(),
        }
    }

    fn get(&self, path: &Path) -> Option<ReadMark> {
        self.by_path.get(path).map(|(_, mark)| *mark)
    }

    fn remove(&mut self, path: &Path) {
        if let Some((seq, _)) = self.by_path.remove(path) {
            self.order.remove(&seq);
        }
    }

    /// Save `mark` for `path`. A new path first makes room, if it can,
    /// by evicting the oldest unprotected marks down to the limit.
    fn insert(&mut self, path: PathBuf, mark: ReadMark) {
        if let Some(entry) = self.by_path.get_mut(&path) {
            entry.1 = mark;
            return;
        }
        self.trim(self.limit);
        let seq = self.next;
        self.next += 1;
        self.order.insert(seq, path.clone());
        self.by_path.insert(path, (seq, mark));
    }

    /// Evict the oldest unprotected marks, one at a time, until no more than
    /// `keep` unprotected marks are left or only protected ones remain.
    fn trim(&mut self, keep: usize) {
        let protected = self
            .pinned
            .iter()
            .filter(|path| self.by_path.contains_key(*path))
            .count();
        while self.by_path.len() - protected > keep {
            let Some(seq) = self
                .order
                .iter()
                .find(|(_, path)| !self.pinned.contains(*path))
                .map(|(seq, _)| *seq)
            else {
                break;
            };
            if let Some(path) = self.order.remove(&seq) {
                self.by_path.remove(&path);
            }
        }
    }

    /// The front thread left the queue: nothing is protected now.
    fn unpin(&mut self) {
        self.pinned.clear();
        self.trim(self.limit);
    }

    fn len(&self) -> usize {
        self.by_path.len()
    }
}

impl PrBacklog {
    /// The backlog a watcher starts with: nothing queued, and a sweep over
    /// every indexed Codex locator pending.
    pub fn starting() -> Self {
        Self {
            sweep: Some(None),
            ..Self::default()
        }
    }

    /// Queue threads to read; one already waiting keeps its place.
    pub fn add<'a>(&mut self, natives: impl IntoIterator<Item = &'a str>) {
        for native in natives {
            if self.waiting.insert(native.to_owned()) {
                self.threads.push_back(native.to_owned());
            }
        }
    }

    /// This backlog keeping at most `limit` marks (at least one) beside the
    /// front thread's, instead of [`MAX_MARKS`]. Tests lower it.
    pub fn with_mark_limit(mut self, limit: usize) -> Self {
        self.marks.limit = limit.max(1);
        self
    }

    /// Files whose mark is kept.
    pub fn marks(&self) -> usize {
        self.marks.len()
    }

    /// Threads still waiting.
    pub fn threads(&self) -> usize {
        self.threads.len()
    }

    /// Whether a later pass has anything to do.
    pub fn pending(&self) -> bool {
        !self.threads.is_empty() || self.sweep.is_some()
    }
}

/// What one pass over the backlog did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrProgress {
    /// Threads taken off the backlog: read to their end, or with nothing to
    /// read (not indexed, no recorded rollout, gone).
    pub decided: usize,
    /// Of those, threads with no indexed user session.
    pub not_indexed: usize,
    /// Witness lines read.
    pub witnesses: usize,
    /// Witnessed pull requests the Store refused (a conflicting stored
    /// identity, or a session that went away).
    pub rejected: usize,
    /// Links created or widened.
    pub changed: usize,
    /// Locators the sweep queued the threads of.
    pub swept: usize,
    pub bytes_read: u64,
    /// Whether the pass moved any work forward.
    pub advanced: bool,
}

/// One bounded pass over the backlog with its own budget, deadline and
/// cancel: the sweep (when `sweep`) queues the threads of every indexed
/// Codex locator, then waiting threads are read in order. What the pass
/// could not reach stays in the backlog.
pub fn continue_codex_pr_links(
    store: &mut Store,
    home: &Path,
    backlog: &mut PrBacklog,
    limits: TitleLimits,
    cancel: Option<&CancelToken>,
    sweep: bool,
) -> xt_store::Result<PrProgress> {
    let mut progress = PrProgress::default();
    let mut batch = Batch::new(limits, cancel);
    let result = continue_within(store, home, backlog, &mut batch, sweep, &mut progress);
    progress.bytes_read = batch.bytes_read();
    result.map(|()| progress)
}

/// [`continue_codex_pr_links`] within a caller's batch, counting into
/// `progress` as it goes, so a pass that fails part-way still says what it
/// committed before the failure.
pub(super) fn continue_within(
    store: &mut Store,
    home: &Path,
    backlog: &mut PrBacklog,
    batch: &mut Batch<'_>,
    sweep: bool,
    progress: &mut PrProgress,
) -> xt_store::Result<()> {
    while sweep && batch.stopped().is_none() {
        let Some(after) = backlog.sweep.clone() else {
            break;
        };
        let keys = store.source_locator_keys_after(
            SessionSource::ReadersCli,
            "codex:",
            after.as_deref(),
            SWEEP_PAGE,
        )?;
        progress.advanced = true;
        let Some(last) = keys.last().cloned() else {
            backlog.sweep = None;
            break;
        };
        let natives: Vec<String> = keys
            .iter()
            .filter_map(|key| Some(codex_thread(Path::new(codex_path(key)?))?.0))
            .collect();
        progress.swept += keys.len();
        backlog.sweep = Some(Some(last));
        backlog.add(natives.iter().map(String::as_str));
    }
    let roots = host_roots(Host::Codex, home);
    while let Some(native) = backlog.threads.front().cloned() {
        if batch.stopped().is_some() {
            break;
        }
        if !read_thread(store, &native, &roots, backlog, batch, progress)? {
            break;
        }
        // Every way a thread leaves the queue passes here.
        backlog.threads.pop_front();
        backlog.waiting.remove(&native);
        backlog.marks.unpin();
        progress.decided += 1;
        progress.advanced = true;
    }
    Ok(())
}

/// Read one thread's recorded rollouts from where each was last read and
/// record every pull request they witness. Whether the thread is finished:
/// `false` when the budget, deadline or a cancel stopped the read, leaving
/// it for a later pass.
fn read_thread(
    store: &mut Store,
    native: &str,
    roots: &[PathBuf],
    backlog: &mut PrBacklog,
    batch: &mut Batch<'_>,
    progress: &mut PrProgress,
) -> xt_store::Result<bool> {
    if !full_thread_id(native) {
        return Ok(true);
    }
    let session = match store
        .user_sessions_with_native(Host::Codex, native)?
        .as_slice()
    {
        [one] => one.clone(),
        [] => {
            // Not indexed yet: the scan that indexes it queues it again.
            progress.not_indexed += 1;
            return Ok(true);
        }
        _ => return Ok(true),
    };
    // Every recorded rollout, however many: a long history is read across
    // passes by its files' marks, never skipped for its length.
    let sources = codex_sources(store, native)?;
    if sources.is_empty() {
        return Ok(true);
    }
    let Ok((candidates, _outside)) = gather(
        sources
            .iter()
            .filter_map(|source| Some((codex_path(&source.locator)?, source))),
        native,
        roots,
        |path, roots| codex_contained(path, roots, native),
    ) else {
        // Nothing recorded for it can be read now.
        return Ok(true);
    };
    // Its files' marks are not evicted while it is read.
    backlog.marks.pinned = candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    for candidate in &candidates {
        let from = backlog.marks.get(&candidate.path);
        let before = batch.bytes_read();
        // Each pull request once, with its earliest and latest witness.
        let mut seen = BTreeMap::<PrIdentity, (i64, i64)>::new();
        let read = batch.appended(candidate, from, &mut |line| {
            if let Some((pull_request, at)) = codex_pr_witness(line, native) {
                progress.witnesses += 1;
                let times = seen.entry(pull_request).or_insert((at, at));
                times.0 = times.0.min(at);
                times.1 = times.1.max(at);
            }
        });
        let (mark, halted) = match read {
            Ok(read) => read,
            Err(_) => {
                // Shrunk, replaced or unreadable: read it whole next time.
                backlog.marks.remove(&candidate.path);
                continue;
            }
        };
        // A cancel that landed at any point, the last chunk included, records
        // nothing this read found and leaves the mark where it was.
        if halted == Some(Untitled::Cancelled) || batch.cancelled() {
            return Ok(false);
        }
        for (pull_request, (first_seen_at, last_seen_at)) in seen {
            let observation = PrLinkObservation {
                session_id: session.clone(),
                pull_request,
                confidence: PrConfidence::Exact,
                first_seen_at,
                last_seen_at,
            };
            match store.record_pr_link(&observation) {
                Ok(outcome) => progress.changed += usize::from(outcome.link_changed),
                Err(Error::InvalidInput(_)) => progress.rejected += 1,
                Err(error) => return Err(error),
            }
        }
        // Progress only when bytes were read and the mark moved: a read that
        // was not started, or that ended where it began, is none.
        if batch.bytes_read() > before && from != Some(mark) {
            progress.advanced = true;
        }
        backlog.marks.insert(candidate.path.clone(), mark);
        if halted.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}
