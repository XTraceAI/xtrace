//! Bounded Codex pages through the pinned producer; no separate normalizer.
use super::super::readers_cli::{ContextRefusal, DetailRefusal, run_context_detail};
use super::*;
use serde::Deserialize;
use serde_json::json;
use std::time::Instant;

pub const CODEX_PAGE_RECORDS: usize = 200;
pub const CODEX_PAGE_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexPageSelection {
    First,
    Cursor(String),
    ToolCall {
        record_uuid: String,
        block: u32,
        generation: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexPageError {
    Source(SourceUnavailable),
    Context(ContextRefusal),
    UnsupportedHistory,
}
impl From<SourceUnavailable> for CodexPageError {
    fn from(value: SourceUnavailable) -> Self {
        Self::Source(value)
    }
}
pub struct CodexPage {
    pub session: LoadedSession,
    pub digest: String,
    pub start: usize,
    pub total: usize,
    pub target: Option<usize>,
    pub backwards: bool,
    pub deadline: Instant,
}
impl CodexPage {
    pub fn cursor(&self, backwards: bool, boundary: usize) -> String {
        format!(
            "v1:{}:{boundary}:{}",
            if backwards { "before" } else { "after" },
            self.digest
        )
    }
}
fn invalid() -> CodexPageError {
    CodexPageError::Context(ContextRefusal::InvalidCursor)
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn decode(cursor: &str) -> Result<(bool, usize, &str), CodexPageError> {
    if cursor.len() > 128 {
        return Err(invalid());
    }
    let parts: Vec<_> = cursor.split(':').collect();
    if parts.len() != 4
        || parts[0] != "v1"
        || !matches!(parts[1], "before" | "after")
        || parts[2].is_empty()
        || !parts[2].bytes().all(|b| b.is_ascii_digit())
        || !digest_valid(parts[3])
    {
        return Err(invalid());
    }
    let position = parts[2].parse::<usize>().map_err(|_| invalid())?;
    if position > MAX_SESSION_RECORDS {
        return Err(invalid());
    }
    Ok((parts[1] == "before", position, parts[3]))
}

pub fn load_codex_page(
    request: &SessionSourceRequest<'_>,
    selection: &CodexPageSelection,
    reader: Option<&DetailReader<'_>>,
) -> Result<CodexPage, CodexPageError> {
    if cancelled(request.cancel) {
        return Err(SourceUnavailable::Cancelled.into());
    }
    if request.host != Host::Codex {
        return Err(SourceUnavailable::UnsupportedHost.into());
    }
    if !names_one_session(request.native_session_id) {
        return Err(SourceUnavailable::InvalidIdentifier.into());
    }
    let reader = reader.ok_or(SourceUnavailable::PrerequisiteUnavailable)?;
    let (wire, backwards) = match selection {
        CodexPageSelection::First => (json!({"version":1,"kind":"first"}), false),
        CodexPageSelection::Cursor(cursor) => {
            let (back, position, generation) = decode(cursor)?;
            (
                json!({"version":1,"kind":if back {"before"} else {"after"},"position":position,"generation":generation}),
                back,
            )
        }
        CodexPageSelection::ToolCall {
            record_uuid,
            block,
            generation,
        } => {
            if record_uuid.is_empty()
                || record_uuid.len() > 4096
                || generation.as_deref().is_some_and(|g| !digest_valid(g))
            {
                return Err(invalid());
            }
            (
                json!({"version":1,"kind":"anchor","record_uuid":record_uuid,"block":block,
                    "generation":generation,"expected":"tool_call"}),
                false,
            )
        }
    };
    let home = request
        .home
        .canonicalize()
        .map_err(|_| SourceUnavailable::Unreadable {
            detail: "history home could not be resolved".into(),
        })?;
    let roots = host_roots(Host::Codex, &home);
    let deadline = Instant::now() + reader.bounds.deadline;
    let mut bounds = reader.bounds;
    bounds.max_stdout = bounds.max_stdout.min(CODEX_PAGE_OUTPUT_BYTES);
    let bytes = run_context_detail(
        reader.python,
        reader.producer,
        &home,
        request.native_session_id,
        bounds,
        request.cancel,
        &wire.to_string(),
    )
    .map_err(|failure| match failure {
        DetailFailure::Cancelled => CodexPageError::Source(SourceUnavailable::Cancelled),
        DetailFailure::Refused(DetailRefusal::Context(reason)) => CodexPageError::Context(reason),
        DetailFailure::Refused(DetailRefusal::StoreUnsupported) => {
            CodexPageError::UnsupportedHistory
        }
        other => CodexPageError::Source(SourceUnavailable::Reader(other)),
    })?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Metadata {
        version: u32,
        digest: String,
        start: usize,
        end: usize,
        total: usize,
        target: Option<usize>,
    }
    #[derive(Deserialize)]
    struct Header {
        context_page: Metadata,
    }
    let first = bytes.split(|b| *b == b'\n').next().unwrap_or_default();
    let meta = serde_json::from_slice::<Header>(first)
        .map_err(|_| broken("invalid page metadata"))?
        .context_page;
    let session = session_from_stream_mode(request, &roots, &bytes, true)?;
    let expected_digest = match selection {
        CodexPageSelection::ToolCall { generation, .. } => generation.as_deref(),
        CodexPageSelection::Cursor(cursor) => Some(decode(cursor)?.2),
        _ => None,
    };
    if (matches!(selection, CodexPageSelection::First) && meta.start != 0)
        || meta.version != 1
        || meta.start > meta.end
        || meta.end > meta.total
        || meta.total > MAX_SESSION_RECORDS
        || (meta.total > 0 && meta.start == meta.end)
        || meta.end - meta.start != session.records.len()
        || session.records.len() > CODEX_PAGE_RECORDS
        || session.dropped_records != 0
        || !digest_valid(&meta.digest)
        || expected_digest.is_some_and(|d| d != meta.digest)
        || matches!(selection, CodexPageSelection::ToolCall { .. }) != meta.target.is_some()
        || meta.target.is_some_and(|t| t < meta.start || t >= meta.end)
    {
        return Err(broken("inconsistent page metadata").into());
    }
    if let CodexPageSelection::Cursor(cursor) = selection {
        let (back, boundary, _) = decode(cursor)?;
        if if back {
            meta.end != boundary
        } else {
            meta.start != boundary
        } {
            return Err(broken("page boundary changed").into());
        }
    }
    if let CodexPageSelection::ToolCall {
        record_uuid, block, ..
    } = selection
    {
        let record = &session.records[meta.target.unwrap() - meta.start];
        let block_kind = record
            .canonical
            .message
            .content
            .as_ref()
            .and_then(|c| c.get(*block as usize))
            .and_then(|b| b.get("type"))
            .and_then(|t| t.as_str());
        if record.canonical.uuid.as_deref() != Some(record_uuid)
            || block_kind != Some("tool_use")
            || session
                .records
                .iter()
                .filter(|r| r.canonical.uuid.as_deref() == Some(record_uuid))
                .count()
                != 1
        {
            return Err(broken("page target changed").into());
        }
    }
    if cancelled(request.cancel) {
        return Err(SourceUnavailable::Cancelled.into());
    }
    if Instant::now() >= deadline {
        return Err(SourceUnavailable::Reader(DetailFailure::Deadline).into());
    }
    Ok(CodexPage {
        session,
        digest: meta.digest,
        start: meta.start,
        total: meta.total,
        target: meta.target,
        backwards,
        deadline,
    })
}
