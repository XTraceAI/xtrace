//! Claude Code keeps its history as canonical JSONL under `~/.claude/projects`:
//! `<project>/<session-id>.jsonl` for the main transcript and
//! `<project>/<session-id>/subagents/**/*.jsonl` for sidechains, which belong to
//! the parent session. Each file is lifted into the shared stream shape (one
//! synthesized session header, then the file's records) and imported through
//! the same per-session writer as the reader-produced streams, one bounded
//! batch at a time, so a large transcript is never held in memory as a whole.
//! Files are only ever read.

use super::checkpoint::{self, FileIdentity, ResumeBasis, TailWindow};
use super::readers_cli::ReaderDiagnostic;
use super::stream::{SessionHeader, expected_conversation_id};
use super::{ScanMode, SessionOutcome, SessionResult, SessionWriter};
use crate::canonical::{Parsed, ParsedRecord, SourceContext, StopHookSummary, parse_with_context};
use crate::writer::{MAX_BATCH_RECORDS, PrWitness};
use sha2::Digest;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use xt_store::{Host, SessionSource, Store, batch::SourceCursor};

#[derive(Clone, Debug)]
pub struct ClaudeFile {
    pub path: PathBuf,
    /// The canonical (and native) session the file's records belong to.
    pub session_id: String,
    pub sidechain: bool,
    pub mtime_ns: u128,
    metadata: fs::Metadata,
}

impl ClaudeFile {
    /// The metadata the enumeration observed, which every later open of this
    /// file must still match.
    pub(super) fn observed(&self) -> &fs::Metadata {
        &self.metadata
    }
}

fn unreadable(path: &Path) -> ReaderDiagnostic {
    ReaderDiagnostic {
        code: "discovery_incomplete".into(),
        path: Some(path.to_string_lossy().into_owned()),
    }
}

/// Every main transcript and sidechain file under the projects root, newest
/// first, plus a diagnostic for each project or entry that could not be
/// listed; the other projects still enumerate. `memory/` trees and non-JSONL
/// files are ignored; nothing is opened. Only an unreadable root is an error.
pub fn enumerate(projects: &Path) -> std::io::Result<(Vec<ClaudeFile>, Vec<ReaderDiagnostic>)> {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for project in fs::read_dir(projects)? {
        let Ok(project) = project.map(|entry| entry.path()) else {
            diagnostics.push(unreadable(projects));
            continue;
        };
        match fs::symlink_metadata(&project) {
            Ok(meta) if meta.file_type().is_symlink() => {
                diagnostics.push(unreadable(&project));
                continue;
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => continue,
            Err(_) => {
                diagnostics.push(unreadable(&project));
                continue;
            }
        }
        let Ok(entries) = fs::read_dir(&project) else {
            diagnostics.push(unreadable(&project));
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry.map(|entry| entry.path()) else {
                diagnostics.push(unreadable(&project));
                continue;
            };
            let Some(name) = entry_name(&entry, &mut diagnostics) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = fs::symlink_metadata(&entry).map(|meta| meta.file_type()) else {
                diagnostics.push(unreadable(&entry));
                continue;
            };
            if kind.is_symlink() {
                // An alias is not a discovered transcript: it is reported, not followed.
                diagnostics.push(unreadable(&entry));
                continue;
            }
            if kind.is_file() {
                if let Some(stem) = name.strip_suffix(".jsonl") {
                    if stem.trim().is_empty() {
                        diagnostics.push(unreadable(&entry));
                        continue;
                    }
                    match mtime_ns(&entry) {
                        Ok((mtime_ns, metadata)) => files.push(ClaudeFile {
                            mtime_ns,
                            metadata,
                            path: entry.clone(),
                            session_id: stem.to_owned(),
                            sidechain: false,
                        }),
                        Err(_) => diagnostics.push(unreadable(&entry)),
                    }
                }
            } else if kind.is_dir() && name != "memory" {
                if name.trim().is_empty() {
                    diagnostics.push(unreadable(&entry));
                    continue;
                }
                let subagents = entry.join("subagents");
                match fs::symlink_metadata(&subagents) {
                    Ok(meta) if meta.is_dir() => {
                        collect_jsonl(&subagents, name, &mut files, &mut diagnostics)
                    }
                    Ok(meta) if meta.file_type().is_symlink() => {
                        diagnostics.push(unreadable(&subagents))
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => diagnostics.push(unreadable(&subagents)),
                }
            }
        }
    }
    files.sort_by(|a, b| {
        b.mtime_ns
            .cmp(&a.mtime_ns)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok((files, diagnostics))
}

const PROBE_MAX_BYTES: u64 = 64 * 1024;
const PROBE_MAX_LINES: usize = 256;
const PROBE_MAX_LINE_BYTES: usize = 8 * 1024;

/// Only the app supplies this exact owned cwd. Encoded names narrow reads;
/// they never establish ownership because different working directories collide.
pub(super) struct UsageProbe {
    cwds: Vec<String>,
    projects: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ProbeMetadata {
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    native_session_id: Option<String>,
    conversation_id: Option<String>,
    source_platform: Option<String>,
    // Derived fields reject duplicate known keys, including null values.
    #[serde(rename = "type")]
    _kind: Option<String>,
    #[serde(rename = "uuid")]
    _uuid: Option<String>,
}

impl UsageProbe {
    pub(super) fn new(directory: &Path) -> Option<Self> {
        if !directory.is_absolute() {
            return None;
        }
        let mut cwds = vec![directory.to_str()?.to_owned()];
        // The app's data directory exists before its probe subdirectory does.
        if let (Some(parent), Some(name)) = (directory.parent(), directory.file_name())
            && let Ok(parent) = fs::canonicalize(parent)
            && let Some(real) = parent.join(name).to_str()
            && !cwds.iter().any(|cwd| cwd == real)
        {
            cwds.push(real.to_owned());
        }
        let projects = cwds
            .iter()
            .map(|cwd| {
                cwd.encode_utf16()
                    .map(|unit| {
                        char::from_u32(u32::from(unit))
                            .filter(char::is_ascii_alphanumeric)
                            .unwrap_or('-')
                    })
                    .collect()
            })
            .collect();
        Some(Self { cwds, projects })
    }

    pub(super) fn excludes(&self, file: &ClaudeFile) -> bool {
        // Main transcripts and nested sidechains both identify their project
        // by the direct child of the projects root, not by an arbitrary ancestor.
        let project = if file.sidechain {
            file.path
                .ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == "subagents"))
                .and_then(Path::parent)
                .and_then(Path::parent)
        } else {
            file.path.parent()
        };
        if project
            .and_then(Path::file_name)
            .and_then(|p| p.to_str())
            .is_none_or(|p| !self.projects.iter().any(|expected| expected == p))
        {
            return false;
        }
        self.confirm(file).unwrap_or(false)
    }

    fn confirm(&self, file: &ClaudeFile) -> std::io::Result<bool> {
        let (mut source, identity) = open_source(&file.path, file.observed())?;
        if !identity.known || identity.len > PROBE_MAX_BYTES {
            return Ok(false);
        }
        // Copy only the bounded observed length, even if the file grows.
        let snapshot = copy_snapshot(&mut source, identity.len)?;
        if FileIdentity::of(&source.metadata()?) != identity {
            return Ok(false);
        }
        let mut reader = BufReader::new(snapshot);
        let mut line = Vec::new();
        let mut records = 0;
        let mut lines = 0;
        loop {
            line.clear();
            let read = (&mut reader)
                .take(PROBE_MAX_LINE_BYTES as u64 + 1)
                .read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            lines += 1;
            if lines > PROBE_MAX_LINES || read > PROBE_MAX_LINE_BYTES || !line.ends_with(b"\n") {
                return Ok(false);
            }
            let Ok(text) = std::str::from_utf8(&line) else {
                return Ok(false);
            };
            if text.trim().is_empty() {
                continue;
            }
            let Ok(raw) = serde_json::from_str::<ProbeMetadata>(text) else {
                return Ok(false);
            };
            if raw.cwd.as_ref().is_some_and(|cwd| !self.cwds.contains(cwd))
                || [
                    raw.session_id.as_deref(),
                    raw.native_session_id.as_deref(),
                    raw.conversation_id.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|id| id != file.session_id)
                || raw
                    .source_platform
                    .as_deref()
                    .is_some_and(|host| host != "claude")
            {
                return Ok(false);
            }
            match parse_with_context(text, &context(&file.session_id)) {
                Ok(Parsed::Record(_))
                    if raw.cwd.is_some() && raw.session_id.as_deref() == Some(&file.session_id) =>
                {
                    records += 1
                }
                Ok(Parsed::Inert) => {}
                _ => return Ok(false),
            }
        }
        // A rename/replacement can leave our open descriptor unchanged. Verify
        // the path again after classification, as well as the original handle.
        Ok(records > 0 && same_probe_revision(file, &source, identity)?)
    }
}

fn same_probe_revision(
    file: &ClaudeFile,
    source: &fs::File,
    identity: FileIdentity,
) -> std::io::Result<bool> {
    let (_, current) = open_source(&file.path, file.observed())?;
    Ok(identity.known && current == identity && FileIdentity::of(&source.metadata()?) == identity)
}

pub(super) fn collect_jsonl(
    directory: &Path,
    session_id: &str,
    files: &mut Vec<ClaudeFile>,
    diagnostics: &mut Vec<ReaderDiagnostic>,
) {
    let _ = collect_jsonl_checked(directory, session_id, files, diagnostics, 0, &mut |_, _| {
        Ok::<_, std::io::Error>(())
    });
}

/// Shared traversal with cooperative checks before each entry and source.
pub(super) fn collect_jsonl_checked<E>(
    directory: &Path,
    session_id: &str,
    files: &mut Vec<ClaudeFile>,
    diagnostics: &mut Vec<ReaderDiagnostic>,
    depth: usize,
    check: &mut dyn FnMut(usize, bool) -> Result<(), E>,
) -> Result<(), E> {
    // Check the directory itself before opening it, including empty trees.
    check(depth, false)?;
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.push(unreadable(directory));
        return Ok(());
    };
    for entry in entries {
        check(depth, false)?;
        let Ok(entry) = entry.map(|entry| entry.path()) else {
            diagnostics.push(unreadable(directory));
            continue;
        };
        let Some(name) = entry_name(&entry, diagnostics) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = fs::symlink_metadata(&entry).map(|meta| meta.file_type()) else {
            diagnostics.push(unreadable(&entry));
            continue;
        };
        if kind.is_symlink() {
            diagnostics.push(unreadable(&entry));
        } else if kind.is_dir() {
            collect_jsonl_checked(&entry, session_id, files, diagnostics, depth + 1, check)?;
        } else if kind.is_file() && name.ends_with(".jsonl") {
            match mtime_ns(&entry) {
                Ok((mtime_ns, metadata)) => {
                    check(depth, true)?;
                    files.push(ClaudeFile {
                        mtime_ns,
                        metadata,
                        path: entry,
                        session_id: session_id.to_owned(),
                        sidechain: true,
                    });
                }
                Err(_) => diagnostics.push(unreadable(&entry)),
            }
        }
    }
    Ok(())
}

/// One file named directly, observed exactly as `enumerate` observes what it
/// finds: a regular file, reached without following an alias.
pub(super) fn observe(
    path: PathBuf,
    session_id: String,
    sidechain: bool,
) -> std::io::Result<ClaudeFile> {
    let (mtime_ns, metadata) = mtime_ns(&path)?;
    Ok(ClaudeFile {
        path,
        session_id,
        sidechain,
        mtime_ns,
        metadata,
    })
}

fn mtime_ns(path: &Path) -> std::io::Result<(u128, fs::Metadata)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("source is not a regular file"));
    }
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Ok((mtime, metadata))
}

pub(super) fn context(session: &str) -> SourceContext {
    SourceContext {
        conversation_id: Some(session.to_owned()),
        native_session_id: Some(session.to_owned()),
        source_platform: Some(Host::Claude.as_str().to_owned()),
        source_surface: None,
        started_at: None,
        source: Some(SessionSource::Transcript),
    }
}

/// The surface the index holds for a session, persisted with its rows.
fn stored_surface(store: &Store, session: &str) -> Option<String> {
    store
        .session(session)
        .ok()
        .flatten()
        .and_then(|stored| stored.meta.surface)
}

/// A record-derived label; a blank value is no label. The writer rejects a
/// record whose identity label is blank, so it must never reach the
/// discovered identity first, where a fill-once value would outlive the skip.
fn label(value: Option<&String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty()).cloned()
}

// Fork snapshots can zero the aggregate counters while retaining the original
// single-message iteration. Recover only that explicit same-model measurement;
// this is not a zero-is-unknown merge rule or a multi-model cost calculation.
fn restore_iteration_usage(record: &mut ParsedRecord, text: &str) -> Result<(), &'static str> {
    if record.canonical.record_type != xt_store::model::RecordType::Assistant {
        return Ok(());
    }
    let Some(usage) = record.canonical.message.usage.as_mut() else {
        return Ok(());
    };
    let eligible_for_restoration = [
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_input_tokens,
        usage.cache_creation_input_tokens,
    ]
    .iter()
    .all(|v| *v == Some(0));
    let raw: serde_json::Value = serde_json::from_str(text).map_err(|_| "invalid usage record")?;
    let Some(iterations) = raw
        .pointer("/message/usage/iterations")
        .and_then(|v| v.as_array())
    else {
        return Ok(());
    };
    if iterations.len() != 1
        || iterations[0].get("type").and_then(|v| v.as_str()) != Some("message")
    {
        return Ok(());
    }
    let iteration = &iterations[0];
    if let Some(model) = iteration.get("model").filter(|v| !v.is_null()) {
        let Some(model) = model.as_str() else {
            return if eligible_for_restoration {
                Err("invalid native iteration model")
            } else {
                Ok(())
            };
        };
        if Some(model) != record.canonical.message.model.as_deref() {
            return Ok(());
        }
    }
    let parsed: xt_store::Usage = match serde_json::from_value(iteration.clone()) {
        Ok(value) => value,
        Err(_) => {
            return if eligible_for_restoration {
                Err("invalid native usage iteration")
            } else {
                Ok(())
            };
        }
    };
    if [
        parsed.input_tokens,
        parsed.output_tokens,
        parsed.cache_read_input_tokens,
        parsed.cache_creation_input_tokens,
    ]
    .iter()
    .any(|v| v.is_none_or(|n| n < 0))
    {
        return if eligible_for_restoration {
            Err("incomplete native usage iteration")
        } else {
            Ok(())
        };
    }
    let aggregates = [
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_input_tokens,
        usage.cache_creation_input_tokens,
    ];
    let confirmed = [
        parsed.input_tokens,
        parsed.output_tokens,
        parsed.cache_read_input_tokens,
        parsed.cache_creation_input_tokens,
    ];
    if aggregates.iter().all(|v| *v == Some(0)) || aggregates == confirmed {
        usage.input_tokens = parsed.input_tokens;
        usage.output_tokens = parsed.output_tokens;
        usage.cache_read_input_tokens = parsed.cache_read_input_tokens;
        usage.cache_creation_input_tokens = parsed.cache_creation_input_tokens;
        record.native.iteration_usage_confirmed = true;
    }
    Ok(())
}

fn header(
    file: &ClaudeFile,
    first: &[ParsedRecord],
    summaries: &[StopHookSummary],
) -> SessionHeader {
    SessionHeader {
        context_page: false,
        kind: "session".into(),
        host: Host::Claude.as_str().into(),
        conversation_id: expected_conversation_id(Host::Claude, &file.session_id),
        native_session_id: file.session_id.clone(),
        source_surface: first
            .iter()
            .find_map(|r| label(r.canonical.source_surface.as_ref()))
            .or_else(|| {
                summaries.iter().find_map(|summary| {
                    label(summary.source.source_surface.as_ref())
                        .or_else(|| label(summary.native.entrypoint.as_ref()))
                })
            }),
        started_at: None,
        cwd: first.iter().find_map(|r| label(r.canonical.cwd.as_ref())),
        git_branch: first
            .iter()
            .find_map(|r| label(r.canonical.git_branch.as_ref())),
        title: None,
        path: file.path.to_string_lossy().into_owned(),
        mtime: file.mtime_ns as f64 / 1_000_000_000.0,
        origin_evidence: Default::default(),
    }
}

enum SummaryError {
    Unsupported(&'static str),
    Surface(&'static str),
}
impl SummaryError {
    fn reason(&self) -> &'static str {
        match self {
            Self::Unsupported(reason) | Self::Surface(reason) => reason,
        }
    }
}

/// The same summary boundary in prevalidation and import. Unsupported identities
/// stop prevalidation without lending a label to earlier batches. A valid native
/// summary with contradictory/unusable surface labels fails surface validation.
fn summary_surface(
    summary: &mut StopHookSummary,
    context: &SourceContext,
) -> Result<Option<String>, SummaryError> {
    if summary
        .uuid
        .as_deref()
        .is_none_or(|id| id.trim().is_empty())
    {
        return Err(SummaryError::Unsupported(
            "structural summary has no stable identity",
        ));
    }
    if [
        summary.native.session_id.as_deref(),
        summary.source.native_session_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|id| id.trim().is_empty())
        || summary
            .source
            .native_session_id
            .as_ref()
            .zip(summary.native.session_id.as_ref())
            .is_some_and(|(a, b)| a != b)
    {
        return Err(SummaryError::Unsupported(
            "structural summary identity labels disagree",
        ));
    }
    // A fork may carry the original container ID; this file is its context.
    summary.native.session_id = context.native_session_id.clone();
    summary.source.native_session_id = context.native_session_id.clone();
    let mut identity = summary.clone();
    identity.source.source_surface = None;
    identity.native.entrypoint = None;
    // Reuse the shared identity rules before treating this summary as evidence
    // about surfaces. Surface agreement is checked separately below.
    crate::writer::resolve_session(&crate::writer::WriteBatch {
        context,
        declared_host: Some(Host::Claude),
        records: &[],
        hook_summaries: std::slice::from_ref(&identity),
        pr_witnesses: &[],
        title: None,
        cwd: None,
        git_branch: None,
        namespace: None,
        keep_content: false,
        observed_at: 0,
        receipt: None,
        cursor: None,
        discovery: None,
        checkpoint: None,
    })
    .map_err(|_| SummaryError::Unsupported("structural summary identity labels disagree"))?;
    if [
        summary.source.source_surface.as_deref(),
        summary.native.entrypoint.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|label| label.trim().is_empty())
    {
        return Err(SummaryError::Surface(
            "structural summary has an unusable surface",
        ));
    }
    if summary
        .source
        .source_surface
        .as_ref()
        .zip(summary.native.entrypoint.as_ref())
        .is_some_and(|(a, b)| a != b)
    {
        return Err(SummaryError::Surface(
            "structural summary surface labels disagree",
        ));
    }
    Ok(summary
        .source
        .source_surface
        .clone()
        .or_else(|| summary.native.entrypoint.clone()))
}

/// The surface the whole file agrees on, settled before any row is written:
/// the first non-blank surface a record or supported summary names, which
/// every later supported observation must repeat. `settled` is the surface the bytes before the snapshot already
/// named, when the snapshot holds only the bytes behind a checkpoint. One
/// streaming pass, nothing retained; a line the parser rejects ends the pass
/// early, and the import pass reports it precisely.
fn file_surface(
    snapshot: &mut fs::File,
    context: &SourceContext,
    settled: Option<String>,
) -> std::io::Result<Result<Option<String>, (usize, &'static str)>> {
    snapshot.rewind()?;
    let mut reader = BufReader::new(snapshot);
    let mut surface = settled;
    let mut line_number = 0usize;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            return Ok(Ok(surface));
        }
        line_number += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(Ok(surface));
        };
        if text.trim().is_empty() {
            continue;
        }
        let named = match parse_with_context(text, context) {
            Ok(Parsed::Record(record)) => match record.canonical.source_surface.as_ref() {
                Some(surface) if surface.trim().is_empty() => {
                    return Ok(Err((line_number, "record has an unusable surface")));
                }
                surface => surface.cloned(),
            },
            Ok(Parsed::StructuralEvent(mut summary)) => {
                match summary_surface(&mut summary, context) {
                    Ok(surface) => surface,
                    Err(SummaryError::Unsupported(_)) => return Ok(Ok(surface)),
                    Err(error) => return Ok(Err((line_number, error.reason()))),
                }
            }
            Ok(Parsed::Dropped(_)) => {
                let raw: serde_json::Value = serde_json::from_str(text)?;
                let value = raw
                    .get("source_surface")
                    .filter(|value| !value.is_null())
                    .or_else(|| raw.get("entrypoint"))
                    .filter(|value| !value.is_null());
                match value {
                    None => None,
                    Some(value) => match value.as_str().filter(|value| !value.trim().is_empty()) {
                        Some(surface) => Some(surface.to_owned()),
                        None => {
                            return Ok(Err((
                                line_number,
                                "dropped record has an unusable surface",
                            )));
                        }
                    },
                }
            }
            Ok(_) => continue,
            Err(_) => return Ok(Ok(surface)),
        };
        if let Some(named) = named {
            match &surface {
                Some(known) if *known != named => {
                    return Ok(Err((
                        line_number,
                        "record surface disagrees with the file's surface",
                    )));
                }
                Some(_) => {}
                None => surface = Some(named),
            }
        }
    }
}

/// Import one file as a stream of complete lines. The file name identifies
/// the session, so its discovered identity is registered before anything is
/// written. In `Resume` mode the file's checkpoint is proven first (same
/// inode, position still held, trailing bytes unchanged): a proven file is
/// read only behind its position. An unchanged file skips parsing; it may
/// refresh its checkpoint or recover a missing discovery surface from committed
/// identity-consistent metadata. A replaced, truncated or rewritten file is read whole again. The
/// surface is settled over the bytes to be read before any row is written;
/// each batch fills in whichever labels are still unknown, and those labels
/// persist only with a committed batch. Batches commit as they fill, each
/// carrying the checkpoint through its last complete line, which the store
/// records only if every record of the batch is stored; a scan that dropped
/// or rejected any record advances no checkpoint, and a zero-position source
/// locator is recorded after the last batch only for a file with no rejected
/// or dropped record. A trailing partial line is left unconsumed. A malformed
/// or non-UTF-8 line stops the file with the committed count named; the
/// checkpoints of earlier batches stay.
pub fn import_file(
    store: &mut Store,
    file: &ClaudeFile,
    observed_at: i64,
    mode: ScanMode,
) -> std::io::Result<SessionResult> {
    let key = checkpoint::file_key(&file.path);
    let context = context(&file.session_id);
    let known = match mode {
        ScanMode::Resume => match store.native_checkpoint(SessionSource::Transcript, &key) {
            Ok(known) => known,
            Err(error) => {
                return Ok(skipped(
                    file,
                    format!("checkpoint could not be read: {error}"),
                ));
            }
        },
        ScanMode::Replay => None,
    };
    let opened = open_source(&file.path, &file.metadata).and_then(|(mut source, identity)| {
        let resume = checkpoint::resume_point(known.as_ref(), &mut source, &identity)?;
        Ok((source, identity, resume))
    });
    let (mut source, identity, resume) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            // The file name still identifies the session: a transcript that
            // vanished or became unreadable since enumeration is a known session
            // whose read failure the caller diagnoses.
            if !file.sidechain
                && let Err(failure) = store.observe_own_check(&file.session_id, None)
            {
                return Ok(skipped(
                    file,
                    format!("display check could not be withdrawn: {failure}"),
                ));
            }
            if let Err(skipped) = SessionWriter::begin(
                store,
                Host::Claude,
                SessionSource::Transcript,
                &header(file, &[], &[]),
                observed_at,
            ) {
                return Ok(*skipped);
            }
            return Err(error);
        }
    };
    if resume.basis == ResumeBasis::Unchanged {
        // Proven unchanged by digesting the whole prefix (its change time had
        // moved): the checkpoint takes the current identity, so the next
        // proof is the cheap one again instead of another full digest.
        if resume.refresh
            && let Err(error) = store.record_native_checkpoint(&checkpoint::file_checkpoint(
                &key,
                &identity,
                resume.start,
                &resume.prefix,
                &TailWindow::seeded(resume.tail.clone()),
                resume.lines,
                observed_at,
            ))
        {
            return Ok(skipped(
                file,
                format!("checkpoint could not be refreshed: {error}"),
            ));
        }
        // Older summary imports could commit the canonical label while leaving
        // discovery unknown. Recover only that identity-consistent committed
        // fact; healthy unchanged scans do no discovery write.
        let surface =
            match store.repair_claude_discovery_surface(&file.session_id, &file.session_id) {
                Ok(surface) => surface,
                Err(error) => {
                    return Ok(skipped(
                        file,
                        format!("committed surface could not be reconciled: {error}"),
                    ));
                }
            };
        return Ok(SessionResult {
            native_session_id: Some(file.session_id.clone()),
            conversation_id: Some(file.session_id.clone()),
            source_surface: surface,
            path: Some(file.path.to_string_lossy().into_owned()),
            outcome: SessionOutcome::Imported {
                records_new: 0,
                records_enriched: 0,
            },
        });
    }
    // A session's own transcript read whole again (replaced, truncated,
    // rewritten, or with no checkpoint) may open differently: its display
    // check observes the inputs it rests on before any row of this read
    // commits. A read behind a proven checkpoint leaves them as they were.
    if !file.sidechain && !matches!(resume.basis, ResumeBasis::Appended) {
        let existing = match store.child_check_attempt(&file.session_id) {
            Ok(existing) => existing,
            Err(error) => {
                return Ok(skipped(
                    file,
                    format!("display check could not be read: {error}"),
                ));
            }
        };
        if existing.is_some() {
            let Some(key) =
                super::claude_launch::check::claude_own_key_at(&file.path, &file.session_id)
            else {
                return Ok(skipped(
                    file,
                    "display check inputs could not be read before this import".into(),
                ));
            };
            if let Err(error) = store.observe_own_check(&file.session_id, key.as_deref()) {
                return Ok(skipped(
                    file,
                    format!("display check could not be recorded: {error}"),
                ));
            }
        }
    }
    let mut writer = match SessionWriter::begin(
        store,
        Host::Claude,
        SessionSource::Transcript,
        &header(file, &[], &[]),
        observed_at,
    ) {
        Ok(started) => started,
        Err(skipped) => return Ok(*skipped),
    };
    // For a session the index knows, the surface is settled already: the
    // index holds the one its records named, persisted with its rows. The
    // report names it from the start, whether the file is read behind its
    // checkpoint or whole again (replaced, truncated, rewritten, replayed),
    // and every record read must repeat it, as it would have to in the first
    // read of a file.
    let settled = stored_surface(store, &file.session_id);
    if let Some(surface) = &settled {
        writer.resume_surface(surface.clone());
    }
    let stop = |writer: &SessionWriter, line: u64, reason: &str| {
        writer.abandon(format!(
            "{reason} (stream line {line}); {} earlier batches stay committed",
            writer.batches
        ))
    };
    // Only the bytes behind the resume point are read, into one bounded
    // anonymous snapshot shared by validation and import.
    source.seek(SeekFrom::Start(resume.start))?;
    let mut snapshot = copy_snapshot(source, identity.len - resume.start)?;
    // The bytes to be read must agree on their surface before the first batch
    // can commit that label to the canonical row and the fill-once discovered
    // identity; the check stays per line below in case the file grows.
    let mut surface = match file_surface(&mut snapshot, &context, settled)? {
        Ok(surface) => surface,
        Err((line, reason)) => return Ok(stop(&writer, resume.lines + line as u64, reason)),
    };
    snapshot.rewind()?;
    let mut reader = BufReader::new(snapshot);
    let mut batch: Vec<ParsedRecord> = Vec::new();
    // Structural hook summaries read from the same lines as `batch`. They are
    // not records, so they are counted and committed separately, never as
    // assistant tool calls.
    let mut summaries: Vec<StopHookSummary> = Vec::new();
    // Explicit `pr-link` witnesses read from the same lines: exact PR evidence
    // that commits with the batch covering it, or not at all.
    let mut witnesses: Vec<PrWitness> = Vec::new();
    let mut dropped = 0;
    let mut lines = resume.lines;
    let mut consumed: u64 = 0;
    let mut window = TailWindow::seeded(resume.tail);
    let mut prefix = resume.prefix;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 || !buffer.ends_with(b"\n") {
            break; // end of file, or a partial trailing line that is not consumed
        }
        lines += 1;
        let Ok(text) = std::str::from_utf8(&buffer) else {
            return Ok(stop(&writer, lines, "transcript is not UTF-8"));
        };
        if !text.trim().is_empty() {
            match parse_with_context(text, &context) {
                Ok(Parsed::Record(record)) => {
                    // sessionId in a fork's inherited prefix can name its
                    // original container. Work identity is the native UUID;
                    // this file is an additional session context. Do not erase
                    // contradictory explicit native labels during normalization.
                    let mut record = record;
                    if let Err(reason) = restore_iteration_usage(&mut record, text) {
                        return Ok(stop(&writer, lines, reason));
                    }
                    if [
                        record.native.session_id.as_deref(),
                        record.canonical.native_session_id.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|id| id.trim().is_empty())
                        || record
                            .canonical
                            .native_session_id
                            .as_ref()
                            .zip(record.native.session_id.as_ref())
                            .is_some_and(|(canonical, native)| canonical != native)
                    {
                        return Ok(stop(
                            &writer,
                            lines,
                            "native record identity labels disagree",
                        ));
                    }
                    record.native.session_id = Some(file.session_id.clone());
                    record.canonical.native_session_id = Some(file.session_id.clone());
                    if let Some(named) = label(record.canonical.source_surface.as_ref()) {
                        match &surface {
                            Some(known) if *known != named => {
                                return Ok(stop(
                                    &writer,
                                    lines,
                                    "record surface disagrees with the file's surface",
                                ));
                            }
                            Some(_) => {}
                            None => surface = Some(named),
                        }
                    }
                    batch.push(*record);
                }
                Ok(Parsed::Dropped(_)) => dropped += 1,
                Ok(Parsed::StructuralEvent(summary)) => {
                    let mut summary = summary;
                    let named = match summary_surface(&mut summary, &context) {
                        Ok(named) => named,
                        Err(error) => return Ok(stop(&writer, lines, error.reason())),
                    };
                    if let Some(named) = named {
                        match &surface {
                            Some(known) if *known != named => {
                                return Ok(stop(
                                    &writer,
                                    lines,
                                    "structural summary surface disagrees with the file's surface",
                                ));
                            }
                            Some(_) => {}
                            None => surface = Some(named),
                        }
                    }
                    summaries.push(*summary);
                }
                Ok(Parsed::PrLink(link)) => {
                    // A witness names its session like a record does and is
                    // reconciled by the same rules, except that the session is
                    // required: without it nothing says whose evidence this is.
                    let mut link = link;
                    let Some(named) = link
                        .native
                        .session_id
                        .clone()
                        .filter(|id| !id.trim().is_empty())
                    else {
                        return Ok(stop(&writer, lines, "PR witness names no native session"));
                    };
                    if link
                        .source
                        .native_session_id
                        .as_ref()
                        .is_some_and(|source| *source != named)
                    {
                        return Ok(stop(&writer, lines, "PR witness identity labels disagree"));
                    }
                    // The witness's own time is its only event time.
                    if link.timestamp.is_none() {
                        return Ok(stop(&writer, lines, "PR witness has no timestamp"));
                    }
                    if xt_store::pr_link::PrIdentity::reconcile(
                        Some(&link.raw_url),
                        Some(&link.raw_repository),
                        Some(link.number),
                    )
                    .is_err()
                    {
                        return Ok(stop(
                            &writer,
                            lines,
                            "PR witness does not name one canonical pull request",
                        ));
                    }
                    // This file is the session context, as for a record; the
                    // session the line names is kept as whose evidence it is,
                    // so a fork's inherited copy stays its original's.
                    link.native.session_id = Some(file.session_id.clone());
                    link.source.native_session_id = Some(file.session_id.clone());
                    let named_surface = label(link.source.source_surface.as_ref())
                        .or_else(|| label(link.native.entrypoint.as_ref()));
                    if let Some(named_surface) = named_surface {
                        match &surface {
                            Some(known) if *known != named_surface => {
                                return Ok(stop(
                                    &writer,
                                    lines,
                                    "PR witness surface disagrees with the file's surface",
                                ));
                            }
                            Some(_) => {}
                            None => surface = Some(named_surface),
                        }
                    }
                    witnesses.push(PrWitness {
                        link: *link,
                        named_session: named,
                    });
                }
                Ok(Parsed::Inert) => {}
                Err(_) => {
                    return Ok(stop(
                        &writer,
                        lines,
                        "canonical record does not match the shared stream contract",
                    ));
                }
            }
        }
        // The line is consumed only once it parsed: progress never covers a
        // line the scan could not read.
        consumed += read as u64;
        window.push(&buffer);
        prefix.update(&buffer);
        if batch.len() + summaries.len() + witnesses.len() == MAX_BATCH_RECORDS {
            if !writer.labels_complete() {
                writer.enrich(&header(file, &batch, &summaries));
            }
            let progress = (dropped == 0 && writer.gapless()).then(|| {
                checkpoint::file_checkpoint(
                    &key,
                    &identity,
                    resume.start + consumed,
                    &prefix,
                    &window,
                    lines,
                    observed_at,
                )
            });
            if let Err(skipped) = writer.write_with_checkpoint(
                store,
                &batch,
                &summaries,
                &witnesses,
                observed_at,
                progress.as_ref(),
            ) {
                return Ok(*skipped);
            }
            batch.clear();
            summaries.clear();
            witnesses.clear();
        }
    }
    writer.note_dropped(dropped);
    if !writer.labels_complete() {
        writer.enrich(&header(file, &batch, &summaries));
    }
    let position = resume.start + consumed;
    let progress = (dropped == 0 && writer.gapless()).then(|| {
        checkpoint::file_checkpoint(
            &key,
            &identity,
            position,
            &prefix,
            &window,
            lines,
            observed_at,
        )
    });
    // A file without a storable record or summary keeps its
    // discovered identity and nothing else: an empty batch commits no row,
    // and no cursor is recorded without a committed batch.
    let carried = !batch.is_empty() || !summaries.is_empty() || !witnesses.is_empty();
    if let Err(skipped) = writer.write_with_checkpoint(
        store,
        &batch,
        &summaries,
        &witnesses,
        observed_at,
        progress.as_ref(),
    ) {
        return Ok(*skipped);
    }
    let gapless = dropped == 0 && writer.gapless();
    let cursor = SourceCursor {
        source: SessionSource::Transcript,
        cursor_key: key.clone(),
        position: 0,
        updated_at: observed_at,
    };
    let result = writer.complete(store, Some(&cursor));
    if !gapless || !matches!(result.outcome, SessionOutcome::Imported { .. }) {
        return Ok(result);
    }
    // Progress no batch carried is recorded on its own, after the rows: the
    // last complete lines were inert, or the scan consumed nothing under a
    // new generation, whose stale checkpoint is then forgotten.
    let recorded = if position == 0 {
        store.clear_native_checkpoint(SessionSource::Transcript, &key)
    } else if carried {
        Ok(())
    } else {
        match &progress {
            Some(progress) => store.record_native_checkpoint(progress),
            None => Ok(()),
        }
    };
    Ok(match recorded {
        Ok(()) => result,
        Err(error) => SessionResult {
            outcome: SessionOutcome::Skipped {
                reason: format!("checkpoint could not be recorded after the rows: {error}"),
            },
            ..result
        },
    })
}

fn skipped(file: &ClaudeFile, reason: String) -> SessionResult {
    SessionResult {
        native_session_id: Some(file.session_id.clone()),
        conversation_id: Some(file.session_id.clone()),
        source_surface: None,
        path: Some(file.path.to_string_lossy().into_owned()),
        outcome: SessionOutcome::Skipped { reason },
    }
}

fn entry_name<'a>(path: &'a Path, diagnostics: &mut Vec<ReaderDiagnostic>) -> Option<&'a str> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => Some(name),
        None => {
            diagnostics.push(unreadable(path));
            None
        }
    }
}

#[cfg(all(test, unix))]
mod name_tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    #[test]
    fn undecodable_name_adds_a_discovery_diagnostic() {
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"invalid-\xff.jsonl".to_vec()));
        let mut diagnostics = Vec::new();
        assert!(entry_name(&path, &mut diagnostics).is_none());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "discovery_incomplete");
        assert_eq!(
            entry_name(Path::new("valid.jsonl"), &mut diagnostics),
            Some("valid.jsonl")
        );
        assert_eq!(diagnostics.len(), 1);
    }
}

/// One anonymous temporary file shared by validation and import. Later source
/// appends are outside this scan; no named transcript copy remains on disk.
#[cfg(test)]
fn snapshot_source(path: &Path, expected: &fs::Metadata) -> std::io::Result<fs::File> {
    let (source, identity) = open_source(path, expected)?;
    copy_snapshot(source, identity.len)
}

/// Open the enumerated transcript without following aliases and confirm it is
/// still the file that was enumerated; its identity fixes the length this scan
/// may read, so later appends stay outside it.
pub(super) fn open_source(
    path: &Path,
    expected: &fs::Metadata,
) -> std::io::Result<(fs::File, FileIdentity)> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let source = options.open(path)?;
    let opened = source.metadata()?;
    if !opened.is_file() {
        return Err(std::io::Error::other("source is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != expected.dev() || opened.ino() != expected.ino() {
            return Err(std::io::Error::other("source changed since discovery"));
        }
    }
    let identity = FileIdentity::of(&source.metadata()?);
    Ok((source, identity))
}

fn copy_snapshot(source: impl Read, length: u64) -> std::io::Result<fs::File> {
    let mut snapshot = tempfile::tempfile()?;
    if std::io::copy(&mut source.take(length), &mut snapshot)? != length {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "source shortened while copying",
        ));
    }
    snapshot.rewind()?;
    Ok(snapshot)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn short_snapshot_copy_is_rejected_before_import() {
        assert_eq!(
            copy_snapshot(&b"short"[..], 10).unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        let mut snapshot = copy_snapshot(&b"exact-plus-append"[..], 5).unwrap();
        let mut bytes = String::new();
        snapshot.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "exact");
    }

    #[test]
    fn append_between_validation_and_import_cannot_change_snapshot() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("session.jsonl");
        let original = "{\"type\":\"assistant\",\"entrypoint\":\"cli\"}\n";
        fs::write(&source, original).unwrap();
        let mut snapshot = snapshot_source(&source, &fs::metadata(&source).unwrap()).unwrap();
        assert_eq!(
            file_surface(&mut snapshot, &context("session"), None)
                .unwrap()
                .unwrap()
                .as_deref(),
            Some("cli")
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"{\"type\":\"assistant\",\"entrypoint\":\"sdk\"}\n")
            .unwrap();
        snapshot.rewind().unwrap();
        let mut imported_bytes = String::new();
        snapshot.read_to_string(&mut imported_bytes).unwrap();
        assert_eq!(imported_bytes, original);
        let mut next = snapshot_source(&source, &fs::metadata(&source).unwrap()).unwrap();
        assert!(
            file_surface(&mut next, &context("session"), None)
                .unwrap()
                .is_err()
        );
    }
}

#[cfg(test)]
mod usage_probe_tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    const SID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000002";

    fn record(id: &str, cwd: &Path, sid: &str) -> String {
        json!({"uuid":id,"type":"user","sessionId":sid,"cwd":cwd,
            "message":{"role":"user","content":"synthetic"}})
        .to_string()
            + "\n"
    }

    struct Fixture {
        _root: tempfile::TempDir,
        home: PathBuf,
        cwd: PathBuf,
        project: PathBuf,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            let cwd = root
                .path()
                .join("Application Support/xtrace-claude-usage-probe");
            fs::create_dir_all(cwd.parent().unwrap()).unwrap();
            let probe = UsageProbe::new(&cwd).unwrap();
            let project = home.join(".claude/projects").join(&probe.projects[0]);
            fs::create_dir_all(&project).unwrap();
            let path = project.join(format!("{SID}.jsonl"));
            Self {
                _root: root,
                home,
                cwd,
                project,
                path,
            }
        }
        fn files(&self) -> Vec<ClaudeFile> {
            let (files, diagnostics) = enumerate(&self.home.join(".claude/projects")).unwrap();
            assert!(diagnostics.is_empty());
            files
        }
        fn excluded(&self, text: &str) -> bool {
            fs::write(&self.path, text).unwrap();
            UsageProbe::new(&self.cwd)
                .unwrap()
                .excludes(&self.files()[0])
        }
        fn scan(
            &self,
            store: &mut Store,
            configured: bool,
            mode: ScanMode,
        ) -> super::super::ImportReport {
            use super::super::{ImportRequest, ProducerSource, session_creation, session_titles};
            let producer = ProducerSource::Checkout {
                pin: self.home.join("unused"),
                plugin_root: None,
            };
            let request = ImportRequest {
                home: &self.home,
                hosts: &[Host::Claude],
                producer: &producer,
                python: None,
                observed_at: 1_790_000_000_000,
                cancel: None,
            };
            super::super::scan_native_continued_with_probe(
                store,
                &request,
                mode,
                &mut |_| {},
                &mut session_creation::SpawnBacklog::default(),
                session_titles::TitleLimits::default(),
                configured.then_some(self.cwd.as_path()),
            )
        }
    }

    #[test]
    fn usage_probe_collision_keeps_users_and_excludes_only_proven_file_before_writes() {
        let f = Fixture::new();
        let collision = f
            ._root
            .path()
            .join("Application-Support/xtrace-claude-usage-probe");
        assert_eq!(
            UsageProbe::new(&collision).unwrap().projects[0],
            UsageProbe::new(&f.cwd).unwrap().projects[0]
        );
        fs::write(&f.path, record("probe", &f.cwd, SID)).unwrap();
        fs::write(
            f.project.join(format!("{OTHER}.jsonl")),
            record("user", &collision, OTHER),
        )
        .unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let report = f.scan(&mut store, true, ScanMode::Resume);
        assert!(report.complete(), "{report:?}");
        assert_eq!(report.hosts[0].sessions.len(), 1);
        assert_eq!(store.counts().unwrap().sessions, 1);
        assert!(store.session(SID).unwrap().is_none());
        assert_eq!(store.records(OTHER).unwrap().len(), 1);
        assert!(
            store
                .discovered_sessions(Host::Claude)
                .unwrap()
                .iter()
                .all(|row| row.native_session_id != SID)
        );
        assert!(
            store
                .native_checkpoint(SessionSource::Transcript, &checkpoint::file_key(&f.path))
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .source_cursor(SessionSource::Transcript, &checkpoint::file_key(&f.path))
                .unwrap()
                .is_none()
        );
        // Replay is still excluded without acquiring a discovery/cursor/checkpoint.
        assert_eq!(
            f.scan(&mut store, true, ScanMode::Replay).hosts[0]
                .sessions
                .len(),
            1
        );
        assert!(store.session(SID).unwrap().is_none());
        // An ordinary caller with no configured ownership path imports both.
        assert_eq!(
            f.scan(&mut store, false, ScanMode::Resume).hosts[0]
                .sessions
                .len(),
            2
        );
        assert!(store.session(SID).unwrap().is_some());
    }

    #[test]
    fn usage_probe_underscore_suffix_folder_is_ordinary_history() {
        let f = Fixture::new();
        let cwd = f
            ._root
            .path()
            .join("Application Support/xtrace_claude_usage_probe");
        assert_eq!(
            UsageProbe::new(&cwd).unwrap().projects[0],
            UsageProbe::new(&f.cwd).unwrap().projects[0]
        );
        assert!(!f.excluded(&record("user", &cwd, SID)));
        let mut store = Store::open_in_memory().unwrap();
        assert!(f.scan(&mut store, true, ScanMode::Resume).complete());
        assert_eq!(store.records(SID).unwrap().len(), 1);
    }

    #[test]
    fn usage_probe_accepts_only_the_configured_or_resolved_parent_spelling() {
        let f = Fixture::new();
        let real = fs::canonicalize(f.cwd.parent().unwrap())
            .unwrap()
            .join(f.cwd.file_name().unwrap());
        let probe = UsageProbe::new(&f.cwd).unwrap();
        let project = f
            .home
            .join(".claude/projects")
            .join(probe.projects.last().unwrap());
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(format!("{SID}.jsonl")),
            record("probe", &real, SID),
        )
        .unwrap();
        assert!(probe.excludes(&f.files()[0]));
        assert!(UsageProbe::new(Path::new("relative/probe")).is_none());
    }

    #[test]
    fn usage_probe_missing_conflicting_and_duplicate_metadata_never_excludes() {
        let f = Fixture::new();
        let good = record("probe", &f.cwd, SID);
        assert!(f.excluded(&good));
        for changed in [
            good.replace(&format!("\"cwd\":{},", json!(f.cwd)), ""),
            good.replace(&format!("\"sessionId\":\"{SID}\","), ""),
            good.replace(SID, OTHER),
            format!("{good}{}", record("later", Path::new("/ordinary"), SID)),
            format!("{good}{{\"type\":\"summary\",\"cwd\":\"/ordinary\"}}\n"),
            good.replacen('{', "{\"cwd\":\"/ordinary\",", 1),
            good.replacen('{', &format!("{{\"sessionId\":\"{OTHER}\","), 1),
            good.replacen('{', "{\"cwd\":null,", 1),
            good.replacen('{', &format!("{{\"native_session_id\":\"{OTHER}\","), 1),
            good.replacen('{', &format!("{{\"conversation_id\":\"{OTHER}\","), 1),
            good.replacen('{', "{\"source_platform\":\"codex\",", 1),
            format!("{good}{{broken\n"),
            good.trim_end().to_owned(),
        ] {
            assert!(!f.excluded(&changed), "{changed}");
        }
        assert!(!f.excluded("{\"type\":\"summary\"}\n"));
        assert!(!f.excluded(""));
    }

    #[test]
    fn usage_probe_bounds_fall_back_to_ordinary_import() {
        let f = Fixture::new();
        let good = record("probe", &f.cwd, SID);
        assert!(!f.excluded(&(good.clone() + &" ".repeat(PROBE_MAX_BYTES as usize))));
        assert!(!f.excluded(&(good.clone() + &"\n".repeat(PROBE_MAX_LINES))));
        let long = good.replace("synthetic", &"x".repeat(PROBE_MAX_LINE_BYTES));
        assert!(!f.excluded(&long));
        let mut store = Store::open_in_memory().unwrap();
        assert!(f.scan(&mut store, true, ScanMode::Resume).complete());
        assert_eq!(store.records(SID).unwrap().len(), 1);
    }

    #[test]
    fn usage_probe_sidechains_need_their_own_evidence() {
        let f = Fixture::new();
        let dir = f.project.join(SID).join("subagents/nested");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("agent.jsonl"), record("probe-child", &f.cwd, SID)).unwrap();
        let probe = UsageProbe::new(&f.cwd).unwrap();
        assert!(probe.excludes(&f.files()[0]));
        fs::write(
            dir.join("agent.jsonl"),
            record("user-child", Path::new("/ordinary"), SID),
        )
        .unwrap();
        assert!(!probe.excludes(&f.files()[0]));
    }

    #[test]
    fn usage_probe_appends_and_rewrites_are_reconsidered_without_a_cache() {
        let f = Fixture::new();
        assert!(f.excluded(&record("probe", &f.cwd, SID)));
        fs::OpenOptions::new()
            .append(true)
            .open(&f.path)
            .unwrap()
            .write_all(record("user", Path::new("/ordinary"), SID).as_bytes())
            .unwrap();
        let mut store = Store::open_in_memory().unwrap();
        assert!(f.scan(&mut store, true, ScanMode::Resume).complete());
        assert_eq!(store.records(SID).unwrap().len(), 2);
        fs::write(&f.path, record("rewritten", Path::new("/ordinary"), SID)).unwrap();
        assert!(f.scan(&mut store, true, ScanMode::Resume).complete());
        assert!(
            store
                .records(SID)
                .unwrap()
                .iter()
                .any(|r| r.uuid == "rewritten")
        );
    }

    #[test]
    fn usage_probe_final_revision_check_refuses_replacement_growth_and_unknown_identity() {
        let f = Fixture::new();
        fs::write(&f.path, record("probe", &f.cwd, SID)).unwrap();
        let file = f.files().remove(0);
        let (source, identity) = open_source(&f.path, file.observed()).unwrap();
        assert!(same_probe_revision(&file, &source, identity).unwrap());
        assert!(
            !same_probe_revision(
                &file,
                &source,
                FileIdentity {
                    known: false,
                    ..identity
                }
            )
            .unwrap()
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&f.path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert!(!same_probe_revision(&file, &source, identity).unwrap());
        fs::rename(&f.path, f.project.join("old.txt")).unwrap();
        fs::write(&f.path, record("replacement", &f.cwd, SID)).unwrap();
        assert!(same_probe_revision(&file, &source, identity).is_err());
        #[cfg(unix)]
        {
            fs::remove_file(&f.path).unwrap();
            std::os::unix::fs::symlink(f.project.join("old.txt"), &f.path).unwrap();
            assert!(same_probe_revision(&file, &source, identity).is_err());
        }
    }
}
