//! Stateless complete-record pages from one verified source generation.
use super::streaming::{Scan, discover, identities, scan_file, verify};
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, time::Instant};

pub const MAX_PAGE_RECORDS: usize = 200;
/// Charge retained source line lengths, not an exact measurement of heap use.
pub const MAX_PAGE_SOURCE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PAGE_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_PAGE_CURSOR_BYTES: usize = 128;
pub const MAX_PAGE_ANCHOR_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageTargetKind {
    Message,
    ToolCall,
    ToolBlock,
}
impl PageTargetKind {
    pub(super) fn matches(self, record: &ParsedRecord, block: Option<u32>) -> bool {
        let block_kind = block.and_then(|index| {
            record
                .canonical
                .message
                .content
                .as_ref()?
                .get(index as usize)?
                .get("type")?
                .as_str()
        });
        match self {
            Self::Message => block.is_none(),
            Self::ToolCall => block_kind == Some("tool_use"),
            Self::ToolBlock => matches!(block_kind, Some("tool_use" | "tool_result")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageSelection {
    First,
    Cursor(String),
    Anchor {
        record_uuid: String,
        block: Option<u32>,
        kind: PageTargetKind,
        generation: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageUnavailable {
    Read(ExcerptUnavailable),
    InvalidCursor,
    Changed,
    RecordTooLarge,
}
impl From<ExcerptUnavailable> for PageUnavailable {
    fn from(reason: ExcerptUnavailable) -> Self {
        Self::Read(reason)
    }
}
#[derive(Debug)]
pub struct PageRecord {
    pub record: ParsedRecord,
    pub ordinal: usize,
    pub source: usize,
    source_bytes: usize,
}
#[derive(Debug)]
pub struct LoadedPage {
    pub generation: GenerationBasis,
    pub digest: String,
    pub records: Vec<PageRecord>,
    pub total_records: usize,
    pub dropped_records: u64,
    pub target: Option<usize>,
    /// A preceding page keeps its suffix when reducing its encoded size.
    pub backwards: bool,
    deadline: Instant,
}
impl LoadedPage {
    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
    pub fn cursor(&self, backwards: bool, boundary: usize) -> String {
        format!(
            "v1:{}:{boundary}:{}",
            if backwards { "before" } else { "after" },
            self.digest
        )
    }
}
fn digest(request: &SessionSourceRequest<'_>, files: &[ClaudeFile]) -> String {
    let mut hash = Sha256::new();
    let mut field = |value: &[u8]| {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    };
    field(b"xtrace-claude-page-v1");
    field(request.native_session_id.as_bytes());
    field(&(files.len() as u64).to_le_bytes());
    for file in files {
        field(file.path.as_os_str().as_encoded_bytes());
        let id = FileIdentity::of(file.observed());
        field(&id.dev.to_le_bytes());
        field(&id.ino.to_le_bytes());
        field(&id.ctime_ns.to_le_bytes());
        field(&id.len.to_le_bytes());
        field(&[id.known as u8]);
    }
    format!("{:x}", hash.finalize())
}
struct Window {
    records: VecDeque<PageRecord>,
    bytes: usize,
}
impl Window {
    fn pop_front(&mut self) {
        if let Some(r) = self.records.pop_front() {
            self.bytes -= r.source_bytes;
        }
    }
    fn push(&mut self, record: PageRecord) {
        self.bytes += record.source_bytes;
        self.records.push_back(record);
    }
    fn fits(&self, bytes: usize) -> bool {
        self.records.len() < MAX_PAGE_RECORDS
            && self.bytes.saturating_add(bytes) <= MAX_PAGE_SOURCE_BYTES
    }
}

pub fn load_session_page_with(
    request: &SessionSourceRequest<'_>,
    selection: &PageSelection,
    limits: ExcerptLimits,
) -> Result<LoadedPage, PageUnavailable> {
    load_session_page_with_reader(request, selection, None, limits)
}

pub fn load_session_page_with_reader(
    request: &SessionSourceRequest<'_>,
    selection: &PageSelection,
    reader: Option<&DetailReader<'_>>,
    limits: ExcerptLimits,
) -> Result<LoadedPage, PageUnavailable> {
    load_page_with_context(request, selection, reader, limits, 1)
}

/// Claude windows follow primary then subagent source order; `source` identifies
/// each record's file. Codex retains the pinned producer's fixed twelve predecessors.
pub fn load_session_context_with_reader(
    request: &SessionSourceRequest<'_>,
    selection: &PageSelection,
    reader: Option<&DetailReader<'_>>,
    limits: ExcerptLimits,
) -> Result<LoadedPage, PageUnavailable> {
    load_page_with_context(request, selection, reader, limits, 12)
}

fn load_page_with_context(
    request: &SessionSourceRequest<'_>,
    selection: &PageSelection,
    reader: Option<&DetailReader<'_>>,
    limits: ExcerptLimits,
    preceding: usize,
) -> Result<LoadedPage, PageUnavailable> {
    if request.host == Host::Codex {
        let mut reader = reader.copied();
        if let Some(reader) = &mut reader {
            reader.bounds.deadline = reader.bounds.deadline.min(limits.deadline);
        }
        let page =
            codex_page::load_page(request, selection, reader.as_ref()).map_err(page_error)?;
        return Ok(LoadedPage {
            generation: page.session.generation,
            digest: page.digest,
            records: page
                .session
                .records
                .into_iter()
                .enumerate()
                .map(|(i, record)| PageRecord {
                    record,
                    ordinal: page.start + i,
                    source: 0,
                    source_bytes: 0,
                })
                .collect(),
            total_records: page.total,
            dropped_records: page.session.dropped_records,
            target: page.target,
            backwards: page.backwards,
            deadline: page.deadline,
        });
    }
    if request.host != Host::Claude {
        return Err(ExcerptUnavailable::Source(SourceUnavailable::UnsupportedHost).into());
    }
    if !names_one_session(request.native_session_id) {
        return Err(ExcerptUnavailable::Source(SourceUnavailable::InvalidIdentifier).into());
    }
    let (backwards, boundary, expected) = match selection {
        PageSelection::Cursor(cursor) => {
            let (back, pos, digest) = codex_page::decode(cursor).map_err(page_error)?;
            (back, pos, Some(digest))
        }
        PageSelection::Anchor {
            record_uuid,
            generation,
            ..
        } => {
            if record_uuid.trim().is_empty() || record_uuid.len() > MAX_PAGE_ANCHOR_BYTES {
                return Err(ExcerptUnavailable::TargetMissing.into());
            }
            if generation
                .as_ref()
                .is_some_and(|d| !codex_page::digest_valid(d))
            {
                return Err(PageUnavailable::InvalidCursor);
            }
            (false, 0, generation.as_deref())
        }
        PageSelection::First => (false, 0, None),
    };
    let mut scan = Scan {
        cancel: request.cancel,
        deadline: Instant::now() + limits.deadline,
        limits,
        bytes: 0,
        lines: 0,
        dropped_records: 0,
        entries: 0,
        files: 0,
        budget: Budget::new(SourceLimits {
            max_bytes: limits.scan_bytes,
            max_records: limits.records,
        }),
    };
    scan.check()?;
    let files = discover(request, &mut scan)?;
    let snapshot = identities(&files);
    let digest = digest(request, &files);
    if expected.is_some_and(|expected| expected != digest) {
        return Err(PageUnavailable::Changed);
    }
    let mut window = Window {
        records: VecDeque::new(),
        bytes: 0,
    };
    let mut total = 0;
    let mut target = None;
    let mut matches = 0;
    let mut oversized = false;
    let mut full = false;
    let mut generation = GenerationBasis::Indexed { appended: false };
    for (source_index, file) in files.iter().enumerate() {
        let basis = scan_file(request, file, &mut scan, |record, bytes| {
            let ordinal = total;
            total += 1;
            let candidate = PageRecord {
                record,
                ordinal,
                source: source_index,
                source_bytes: bytes,
            };
            if let PageSelection::Anchor {
                record_uuid,
                block,
                kind,
                ..
            } = selection
            {
                if candidate.record.canonical.uuid.as_deref() == Some(record_uuid) {
                    matches += 1;
                    if matches > 1 {
                        return Err(ExcerptUnavailable::TargetAmbiguous);
                    }
                    if !kind.matches(&candidate.record, *block) {
                        return Err(ExcerptUnavailable::BlockMissing);
                    }
                    target = Some(ordinal);
                    while !window.fits(bytes) && !window.records.is_empty() {
                        window.pop_front();
                    }
                    if !window.fits(bytes) {
                        oversized = true;
                        full = true;
                    } else {
                        window.push(candidate);
                    }
                } else if target.is_none() {
                    // Diagnosis can retain more preceding context than a UI page.
                    while !window.records.is_empty()
                        && (window.records.len() >= preceding || !window.fits(bytes))
                    {
                        window.pop_front();
                    }
                    if window.fits(bytes) {
                        window.push(candidate);
                    }
                } else if !full {
                    if window.fits(bytes) {
                        window.push(candidate);
                    } else {
                        full = true;
                    }
                }
            } else if backwards {
                if ordinal < boundary {
                    while !window.fits(bytes) && !window.records.is_empty() {
                        window.pop_front();
                    }
                    if window.fits(bytes) {
                        window.push(candidate);
                        oversized = false;
                    } else {
                        oversized = true;
                    }
                }
            } else if ordinal >= boundary && !full {
                if window.fits(bytes) {
                    window.push(candidate);
                } else {
                    oversized = window.records.is_empty();
                    full = true;
                }
            }
            Ok(())
        })?;
        generation = generation.and(basis);
    }
    let after = discover(request, &mut scan)?;
    if identities(&after) != snapshot {
        return Err(PageUnavailable::Changed);
    }
    for file in &files {
        scan.check()?;
        super::excerpt::settled_for_test(&file.path);
        let (source, identity) = claude_fs::open_source(&file.path, file.observed())
            .map_err(ExcerptUnavailable::from)?;
        let expected = FileIdentity::of(file.observed());
        if !unchanged(&identity, &expected) {
            return Err(PageUnavailable::Changed);
        }
        verify(file, &source, &expected)?;
    }
    scan.check()?;
    if matches!(selection, PageSelection::Anchor { .. }) && matches == 0 {
        return Err(ExcerptUnavailable::TargetMissing.into());
    }
    if oversized {
        return Err(PageUnavailable::RecordTooLarge);
    }
    if boundary > total
        || (backwards && boundary == 0)
        || (!backwards && boundary == total && total != 0)
    {
        return Err(PageUnavailable::InvalidCursor);
    }
    Ok(LoadedPage {
        generation,
        digest,
        records: window.records.into(),
        total_records: total,
        dropped_records: scan.dropped_records,
        target,
        backwards,
        deadline: scan.deadline,
    })
}

fn page_error(error: CodexPageError) -> PageUnavailable {
    use super::super::readers_cli::ContextRefusal as C;
    match error {
        CodexPageError::Source(SourceUnavailable::Reader(DetailFailure::Deadline)) => {
            ExcerptUnavailable::Deadline.into()
        }
        CodexPageError::Source(reason) => ExcerptUnavailable::Source(reason).into(),
        CodexPageError::UnsupportedHistory => {
            ExcerptUnavailable::Source(SourceUnavailable::UnsupportedHistory).into()
        }
        CodexPageError::Context(reason) => match reason {
            C::Changed => PageUnavailable::Changed,
            C::InvalidCursor => PageUnavailable::InvalidCursor,
            C::RecordTooLarge => PageUnavailable::RecordTooLarge,
            C::TargetMissing => ExcerptUnavailable::TargetMissing.into(),
            C::TargetAmbiguous => ExcerptUnavailable::TargetAmbiguous.into(),
            C::BlockMissing => ExcerptUnavailable::BlockMissing.into(),
        },
    }
}
