//! Transient, bounded context for one saved message. Every relevant source is
//! checked before a unique target is returned. At most three records are
//! retained around it (the one before, the target, the one after), plus, when
//! the caller names one, a single earlier *lead* record from the target's own
//! source (see [`LeadKept`]), so at most four. A lead that is missing, named
//! twice, in another source, after the target or already beside it is left
//! out and the excerpt is read as without one: the lead never fails a read.
use super::streaming::{Scan, discover, identities, verify};
use super::*;
use std::time::{Duration, Instant};

pub const MAX_EXCERPT_SCAN_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_EXCERPT_LINE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EXCERPT_RAW_LINES: u64 = 2_000_000;
pub const MAX_EXCERPT_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_DISCOVERY_ENTRIES: usize = 100_000;
const MAX_DISCOVERY_DEPTH: usize = 32;
const MAX_DISCOVERY_FILES: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcerptLimit {
    ScanBytes,
    LineBytes,
    Records,
    RawLines,
    OutputBytes,
    DiscoveryEntries,
    DiscoveryDepth,
    SourceFiles,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExcerptUnavailable {
    Source(SourceUnavailable),
    Limit(ExcerptLimit),
    Deadline,
    Incomplete,
    TargetMissing,
    TargetAmbiguous,
    BlockMissing,
}
impl From<std::io::Error> for ExcerptUnavailable {
    fn from(_: std::io::Error) -> Self {
        Self::Source(SourceUnavailable::Unreadable {
            detail: "source could not be read".into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedExcerpt {
    pub generation: GenerationBasis,
    pub records: Vec<ParsedRecord>,
    pub target: usize,
    /// Set when `records[0]` is the lead record the read was asked for, kept
    /// ahead of the target's usual neighbours.
    pub lead: Option<LeadKept>,
}

/// The lead record an excerpt kept: an earlier record of the target's own
/// source, such as the person's message an admitted mistake answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeadKept {
    /// Records lie between the lead and the next record kept, and are not
    /// shown. `false` when the lead is directly before it.
    pub gap: bool,
}

impl LoadedExcerpt {
    /// The same excerpt without its lead record, as it is read without one.
    pub fn without_lead(mut self) -> Self {
        if self.lead.take().is_some() && !self.records.is_empty() {
            self.records.remove(0);
            self.target = self.target.saturating_sub(1);
        }
        self
    }
}

/// The lead to keep, by its position among one source's records, when the
/// first record kept around the target is at `first_kept`: only an earlier
/// record that is not already kept.
fn lead_before(lead_at: usize, first_kept: usize) -> Option<LeadKept> {
    (lead_at < first_kept).then_some(LeadKept {
        gap: lead_at + 1 < first_kept,
    })
}

/// Limits may be lowered for deterministic refusal tests. Defaults apply to
/// the entire open, including both discovery passes. The deadline is cooperative:
/// blocking filesystem calls and one bounded canonical parse cannot be interrupted.
#[derive(Clone, Copy, Debug)]
pub struct ExcerptLimits {
    pub scan_bytes: u64,
    pub line_bytes: usize,
    pub records: usize,
    pub raw_lines: u64,
    pub deadline: Duration,
    pub discovery_entries: usize,
    pub discovery_depth: usize,
    pub source_files: usize,
}
impl Default for ExcerptLimits {
    fn default() -> Self {
        Self {
            scan_bytes: MAX_EXCERPT_SCAN_BYTES,
            line_bytes: MAX_EXCERPT_LINE_BYTES,
            records: MAX_SESSION_RECORDS,
            raw_lines: MAX_EXCERPT_RAW_LINES,
            deadline: super::super::readers_cli::DETAIL_DEADLINE,
            discovery_entries: MAX_DISCOVERY_ENTRIES,
            discovery_depth: MAX_DISCOVERY_DEPTH,
            source_files: MAX_DISCOVERY_FILES,
        }
    }
}

/// A bounded excerpt read. Claude scans verified files without retaining the
/// session. Other supported hosts keep the pinned full reader's current limits.
pub fn load_session_excerpt_with(
    request: &SessionSourceRequest<'_>,
    record_uuid: &str,
    block: Option<u32>,
    reader: Option<&DetailReader<'_>>,
    limits: ExcerptLimits,
) -> Result<LoadedExcerpt, ExcerptUnavailable> {
    load_session_excerpt_with_lead(request, record_uuid, None, block, reader, limits)
}

/// [`load_session_excerpt_with`], also keeping the record `lead` names when
/// it is an earlier record of the target's own source ([`LeadKept`]). Which
/// record is the lead is the caller's decision; a lead that cannot be kept
/// changes nothing else about the read.
pub fn load_session_excerpt_with_lead(
    request: &SessionSourceRequest<'_>,
    record_uuid: &str,
    lead: Option<&str>,
    block: Option<u32>,
    reader: Option<&DetailReader<'_>>,
    limits: ExcerptLimits,
) -> Result<LoadedExcerpt, ExcerptUnavailable> {
    let lead = lead.filter(|lead| !lead.trim().is_empty() && *lead != record_uuid);
    if cancelled(request.cancel) {
        return Err(ExcerptUnavailable::Source(SourceUnavailable::Cancelled));
    }
    if !names_one_session(request.native_session_id) {
        return Err(ExcerptUnavailable::Source(
            SourceUnavailable::InvalidIdentifier,
        ));
    }
    if record_uuid.trim().is_empty() || record_uuid.len() > MAX_EXCERPT_LINE_BYTES {
        return Err(ExcerptUnavailable::TargetMissing);
    }
    if request.host != Host::Claude {
        return match load_session_source_with(request, reader) {
            SessionSourceOutcome::Unavailable(reason) => Err(ExcerptUnavailable::Source(reason)),
            SessionSourceOutcome::Loaded(loaded) => {
                excerpt_from_loaded(*loaded, record_uuid, lead, block)
            }
        };
    }
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
    let files = discover(request, &mut scan)?;
    let before = identities(&files);
    let mut selected = Vec::new();
    let mut target = 0;
    let mut matches = 0;
    let mut leads = 0;
    let mut kept_lead = None;
    let mut generation = GenerationBasis::Indexed { appended: false };
    for file in &files {
        let found = scan_file(request, file, record_uuid, lead, &mut scan)?;
        generation = generation.and(found.generation);
        matches += found.matches;
        leads += found.leads;
        if matches > 1 {
            return Err(ExcerptUnavailable::TargetAmbiguous);
        }
        if found.matches == 1 {
            selected = found.window;
            target = found.offset;
            kept_lead = found.lead;
        }
    }
    // A subagent added during the scan could contain a second target. Check
    // the complete source set again, under the same cumulative discovery budget.
    let after = discover(request, &mut scan)?;
    if identities(&after) != before {
        return Err(ExcerptUnavailable::Source(SourceUnavailable::Replaced {
            basis: ResumeBasis::Replaced,
        }));
    }
    for file in &files {
        scan.check()?;
        settled_for_test(&file.path);
        let (source, identity) = claude_fs::open_source(&file.path, file.observed())?;
        let expected = FileIdentity::of(file.observed());
        if !unchanged(&identity, &expected) {
            return Err(ExcerptUnavailable::Source(SourceUnavailable::Replaced {
                basis: replacement_basis(&identity, &expected),
            }));
        }
        verify(file, &source, &expected)?;
    }
    scan.check()?;
    if matches == 0 {
        return Err(ExcerptUnavailable::TargetMissing);
    }
    validate_block(&selected[target], block)?;
    let mut excerpt = LoadedExcerpt {
        generation,
        records: selected,
        target,
        lead: None,
    };
    // A lead named by more than one record, in any file, is not kept.
    if leads == 1
        && let Some((record, kept)) = kept_lead
    {
        excerpt.records.insert(0, record);
        excerpt.target += 1;
        excerpt.lead = Some(kept);
    }
    Ok(excerpt)
}

#[cfg(test)]
thread_local! { static SETTLED: ActOnObserved = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn settled_for_test(path: &Path) {
    SETTLED.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(path);
        }
    });
}
#[cfg(not(test))]
pub(super) fn settled_for_test(_path: &Path) {}

/// What one Claude file holds of an excerpt.
struct FileExcerpt {
    generation: GenerationBasis,
    /// The target and its neighbours, when the target is in this file.
    window: Vec<ParsedRecord>,
    offset: usize,
    matches: usize,
    /// How many records in this file the lead names.
    leads: usize,
    /// The lead, when it is in this file before the target and not already
    /// in `window` (one saved copy, only the file's first).
    lead: Option<(ParsedRecord, LeadKept)>,
}

fn scan_file(
    request: &SessionSourceRequest<'_>,
    file: &ClaudeFile,
    uuid: &str,
    lead: Option<&str>,
    scan: &mut Scan<'_>,
) -> Result<FileExcerpt, ExcerptUnavailable> {
    let mut previous = None;
    let mut window = Vec::new();
    let mut offset = 0;
    let mut matches = 0;
    let mut want_next = false;
    let mut position = 0;
    let mut leads = 0;
    let mut saved_lead: Option<(usize, ParsedRecord)> = None;
    let mut kept_lead = None;
    let generation = super::streaming::scan_file(request, file, scan, |record, _| {
        let id = record.canonical.uuid.as_deref();
        if id == Some(uuid) {
            matches += 1;
            if matches > 1 {
                return Err(ExcerptUnavailable::TargetAmbiguous);
            }
            let first_kept = if previous.is_some() {
                position - 1
            } else {
                position
            };
            if let Some(before) = previous.take() {
                window.push(before);
            }
            offset = window.len();
            window.push(record.clone());
            want_next = true;
            kept_lead = saved_lead
                .take()
                .and_then(|(at, record)| lead_before(at, first_kept).map(|kept| (record, kept)));
        } else {
            if lead.is_some() && id == lead {
                leads += 1;
                if leads == 1 && matches == 0 {
                    saved_lead = Some((position, record.clone()));
                }
            }
            if want_next {
                window.push(record.clone());
                want_next = false;
            }
        }
        previous = Some(record);
        position += 1;
        Ok(())
    })?;
    Ok(FileExcerpt {
        generation,
        window,
        offset,
        matches,
        leads,
        lead: kept_lead,
    })
}

fn validate_block(record: &ParsedRecord, block: Option<u32>) -> Result<(), ExcerptUnavailable> {
    if let Some(index) = block {
        let kind = record
            .canonical
            .message
            .content
            .as_ref()
            .and_then(|blocks| blocks.get(index as usize))
            .and_then(|block| block.get("type"))
            .and_then(serde_json::Value::as_str);
        if !matches!(kind, Some("tool_use" | "tool_result")) {
            return Err(ExcerptUnavailable::BlockMissing);
        }
    }
    Ok(())
}
/// The excerpt around `uuid` in a session read whole, with the record `lead`
/// names kept ahead of it when that is an earlier record of the target's own
/// source ([`LeadKept`]).
pub fn excerpt_from_loaded(
    loaded: LoadedSession,
    uuid: &str,
    lead: Option<&str>,
    block: Option<u32>,
) -> Result<LoadedExcerpt, ExcerptUnavailable> {
    if !loaded.gaps.is_empty() {
        return Err(ExcerptUnavailable::Incomplete);
    }
    let mut targets = loaded
        .records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.canonical.uuid.as_deref() == Some(uuid));
    let Some((target, record)) = targets.next() else {
        return Err(ExcerptUnavailable::TargetMissing);
    };
    if targets.next().is_some() {
        return Err(ExcerptUnavailable::TargetAmbiguous);
    }
    validate_block(record, block)?;
    let mut offset = 0;
    for source in &loaded.sources {
        let end = offset + source.record_count;
        if offset <= target && target < end {
            let start = offset.max(target.saturating_sub(1));
            let end = end.min(target + 2);
            let lead =
                lead.filter(|lead| *lead != uuid)
                    .and_then(|lead| {
                        let mut named =
                            loaded.records.iter().enumerate().filter(|(_, record)| {
                                record.canonical.uuid.as_deref() == Some(lead)
                            });
                        let (at, _) = named.next()?;
                        named.next().is_none().then_some(at)
                    })
                    .filter(|&at| offset <= at)
                    .and_then(|at| Some((at, lead_before(at, start)?)));
            let mut records = Vec::with_capacity(end - start + 1);
            if let Some((at, _)) = lead {
                records.push(loaded.records[at].clone());
            }
            records.extend_from_slice(&loaded.records[start..end]);
            return Ok(LoadedExcerpt {
                generation: loaded.generation,
                records,
                target: target - start + usize::from(lead.is_some()),
                lead: lead.map(|(_, kept)| kept),
            });
        }
        offset = end;
    }
    Err(ExcerptUnavailable::Incomplete)
}

/// The excerpt around a bounded page's target: the target and the records
/// beside it from its own source, and `lead` when the page holds it earlier
/// in that source ([`LeadKept`]). A lead outside the page, named twice in it
/// or of another source is left out; a page without its target is
/// [`ExcerptUnavailable::TargetMissing`].
pub fn excerpt_from_page(
    page: LoadedPage,
    record_uuid: &str,
    lead: Option<&str>,
) -> Result<LoadedExcerpt, ExcerptUnavailable> {
    let target = page
        .target
        .and_then(|ordinal| page.records.iter().position(|r| r.ordinal == ordinal))
        .ok_or(ExcerptUnavailable::TargetMissing)?;
    let source = page.records[target].source;
    let start = target.saturating_sub(1);
    let end = page.records.len().min(target + 2);
    let first_kept = if page.records[start].source == source {
        start
    } else {
        target
    };
    let lead = lead
        .filter(|lead| *lead != record_uuid)
        .and_then(|lead| {
            let mut named = page
                .records
                .iter()
                .enumerate()
                .filter(|(_, r)| r.record.canonical.uuid.as_deref() == Some(lead));
            let (at, _) = named.next()?;
            named.next().is_none().then_some(at)
        })
        .filter(|&at| page.records[at].source == source)
        .and_then(|at| Some((at, lead_before(at, first_kept)?)));
    let mut records = Vec::new();
    let mut target_index = 0;
    for (index, record) in page.records.into_iter().enumerate() {
        if lead.is_some_and(|(at, _)| at == index) {
            records.push(record.record);
        } else if (start..end).contains(&index) && record.source == source {
            if index == target {
                target_index = records.len();
            }
            records.push(record.record);
        }
    }
    Ok(LoadedExcerpt {
        generation: page.generation,
        records,
        target: target_index,
        lead: lead.map(|(_, kept)| kept),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    const SESSION: &str = "synthetic-excerpt";
    fn line(id: &str, text: &str) -> String {
        format!(
            "{}\n",
            json!({"type":"user", "uuid":id, "sessionId":SESSION,
            "timestamp":"2026-10-25T12:00:00Z", "message":{"role":"user", "content":[{"type":"text", "text":text}]}})
        )
    }
    struct Fixture {
        temp: tempfile::TempDir,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let path = temp
                .path()
                .join(format!(".claude/projects/project/{SESSION}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                [
                    line("before", "before"),
                    line("target", "trigger"),
                    line("after", "after"),
                ]
                .concat(),
            )
            .unwrap();
            Self { temp, path }
        }
        fn request(&self) -> SessionSourceRequest<'_> {
            SessionSourceRequest {
                home: self.temp.path(),
                host: Host::Claude,
                native_session_id: SESSION,
                indexed: &[],
                cancel: None,
                limits: SourceLimits::default(),
            }
        }
        fn read(&self, id: &str) -> Result<LoadedExcerpt, ExcerptUnavailable> {
            load_session_excerpt_with(&self.request(), id, None, None, ExcerptLimits::default())
        }
        fn sidechain(&self, text: &str) -> PathBuf {
            let path = self
                .path
                .parent()
                .unwrap()
                .join(SESSION)
                .join("subagents/agent-a.jsonl");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }
    }
    fn ids(read: &LoadedExcerpt) -> Vec<&str> {
        read.records
            .iter()
            .map(|r| r.canonical.uuid.as_deref().unwrap())
            .collect()
    }
    #[test]
    fn sixty_six_mib_refuses_full_read_but_loads_early_middle_and_late_excerpts() {
        let f = Fixture::new();
        let mut file = fs::File::create(&f.path).unwrap();
        let text = "synthetic padding ".repeat(4000);
        for n in 0..1024 {
            file.write_all(line(&format!("record-{n}"), &text).as_bytes())
                .unwrap();
        }
        drop(file);
        assert!(fs::metadata(&f.path).unwrap().len() > 66 * 1024 * 1024);
        assert!(matches!(
            load_session_source(&f.request()),
            SessionSourceOutcome::Unavailable(SourceUnavailable::TooLarge {
                limit: SourceCeiling::Bytes,
                ..
            })
        ));
        for (n, expected) in [
            (0, vec!["record-0", "record-1"]),
            (512, vec!["record-511", "record-512", "record-513"]),
            (1023, vec!["record-1022", "record-1023"]),
        ] {
            let read = f.read(&format!("record-{n}")).unwrap();
            assert_eq!(ids(&read), expected);
            assert_eq!(
                read.records[read.target].canonical.uuid.as_deref(),
                Some(format!("record-{n}").as_str())
            );
        }
    }
    /// The excerpt around `target` with `lead`, read both ways: the Claude
    /// streaming scan, and from the session read whole (the Codex and Cursor
    /// path). Both must agree.
    fn with_lead(f: &Fixture, target: &str, lead: &str) -> LoadedExcerpt {
        let streamed = load_session_excerpt_with_lead(
            &f.request(),
            target,
            Some(lead),
            None,
            None,
            ExcerptLimits::default(),
        )
        .unwrap();
        let SessionSourceOutcome::Loaded(loaded) = load_session_source(&f.request()) else {
            panic!("loaded");
        };
        let whole = excerpt_from_loaded(*loaded, target, Some(lead), None).unwrap();
        assert_eq!(streamed, whole);
        streamed
    }
    fn write(f: &Fixture, records: &[&str]) {
        fs::write(
            &f.path,
            records
                .iter()
                .map(|id| line(id, "synthetic"))
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn an_earlier_lead_is_kept_ahead_of_the_neighbours_with_a_gap_when_records_lie_between() {
        let f = Fixture::new();
        write(
            &f,
            &[
                "first", "lead", "step-1", "step-2", "target", "after", "last",
            ],
        );
        let read = with_lead(&f, "target", "lead");
        assert_eq!(ids(&read), ["lead", "step-2", "target", "after"]);
        assert_eq!(read.target, 2);
        assert_eq!(read.lead, Some(LeadKept { gap: true }));
        // Directly before the record before the target: kept, no gap.
        write(&f, &["lead", "step", "target", "after"]);
        let read = with_lead(&f, "target", "lead");
        assert_eq!(ids(&read), ["lead", "step", "target", "after"]);
        assert_eq!(read.lead, Some(LeadKept { gap: false }));
        // Without the lead the read is as before.
        assert_eq!(ids(&f.read("target").unwrap()), ["step", "target", "after"]);
        assert_eq!(f.read("target").unwrap().lead, None);
    }
    #[test]
    fn a_lead_already_beside_the_target_is_not_shown_twice() {
        let f = Fixture::new();
        write(&f, &["lead", "target", "after"]);
        let read = with_lead(&f, "target", "lead");
        assert_eq!(ids(&read), ["lead", "target", "after"]);
        assert_eq!((read.target, read.lead), (1, None));
        // The lead is the target itself.
        let read = with_lead(&f, "target", "target");
        assert_eq!(ids(&read), ["lead", "target", "after"]);
        assert_eq!(read.lead, None);
    }
    #[test]
    fn a_lead_that_is_missing_named_twice_later_or_elsewhere_leaves_the_excerpt_as_before() {
        let as_before = |f: &Fixture, lead: &str| {
            let read = with_lead(f, "target", lead);
            assert_eq!(ids(&read), ["step", "target", "after"], "{lead}");
            assert_eq!((read.target, read.lead), (1, None), "{lead}");
        };
        let f = Fixture::new();
        write(&f, &["lead", "x", "step", "target", "after"]);
        assert_eq!(ids(&with_lead(&f, "target", "lead")).len(), 4);
        // Missing.
        as_before(&f, "nowhere");
        // Named twice in the same file, before and after the target.
        write(&f, &["lead", "x", "step", "target", "after", "lead"]);
        as_before(&f, "lead");
        // After the target.
        write(&f, &["x", "step", "target", "after", "y", "lead"]);
        as_before(&f, "lead");
        // In another source file only.
        write(&f, &["x", "step", "target", "after"]);
        f.sidechain(&line("lead", "synthetic"));
        as_before(&f, "lead");
        // Before the target, and once more in another source file.
        write(&f, &["lead", "x", "step", "target", "after"]);
        as_before(&f, "lead");
    }
    #[test]
    fn the_target_is_still_refused_when_named_twice_whatever_the_lead() {
        let f = Fixture::new();
        write(&f, &["lead", "x", "target", "target"]);
        assert_eq!(
            load_session_excerpt_with_lead(
                &f.request(),
                "target",
                Some("lead"),
                None,
                None,
                ExcerptLimits::default()
            ),
            Err(ExcerptUnavailable::TargetAmbiguous)
        );
    }
    #[test]
    fn without_lead_gives_the_excerpt_read_without_one() {
        let f = Fixture::new();
        write(&f, &["lead", "x", "step", "target", "after"]);
        assert_eq!(
            with_lead(&f, "target", "lead").without_lead(),
            f.read("target").unwrap()
        );
    }
    #[test]
    fn a_bounded_page_keeps_the_lead_only_when_it_holds_it_earlier_in_the_same_source() {
        let f = Fixture::new();
        let page = |target: &str| {
            load_session_context_with_reader(
                &f.request(),
                &page_anchor(target),
                None,
                ExcerptLimits::default(),
            )
            .unwrap()
        };
        write(
            &f,
            &["first", "lead", "step-1", "step-2", "target", "after"],
        );
        let read = excerpt_from_page(page("target"), "target", Some("lead")).unwrap();
        assert_eq!(ids(&read), ["lead", "step-2", "target", "after"]);
        assert_eq!((read.target, read.lead), (2, Some(LeadKept { gap: true })));
        // Without a lead, or one the page does not hold: as before.
        let plain = excerpt_from_page(page("target"), "target", None).unwrap();
        assert_eq!(ids(&plain), ["step-2", "target", "after"]);
        assert_eq!((plain.target, plain.lead), (1, None));
        assert_eq!(
            excerpt_from_page(page("target"), "target", Some("nowhere")).unwrap(),
            plain
        );
        // Beside the target already, or after it: not shown twice.
        let read = excerpt_from_page(page("target"), "target", Some("step-2")).unwrap();
        assert_eq!((ids(&read).len(), read.lead), (3, None));
        let read = excerpt_from_page(page("target"), "target", Some("after")).unwrap();
        assert_eq!((ids(&read).len(), read.lead), (3, None));
        // Directly before the record before the target: no gap.
        let read = excerpt_from_page(page("target"), "target", Some("step-1")).unwrap();
        assert_eq!(read.lead, Some(LeadKept { gap: false }));
        // Earlier than the page holds (a plain page keeps one record before).
        let short = load_session_page_with(
            &f.request(),
            &page_anchor("target"),
            ExcerptLimits::default(),
        )
        .unwrap();
        let read = excerpt_from_page(short, "target", Some("lead")).unwrap();
        assert_eq!(
            (ids(&read), read.lead),
            (vec!["step-2", "target", "after"], None)
        );
        // Named twice in the page.
        write(&f, &["lead", "x", "lead", "step", "target", "after"]);
        let read = excerpt_from_page(page("target"), "target", Some("lead")).unwrap();
        assert_eq!(
            (ids(&read), read.lead),
            (vec!["step", "target", "after"], None)
        );
        // In another source than the target.
        write(&f, &["x", "y", "z"]);
        f.sidechain(&[line("lead", "s"), line("a", "s"), line("b", "s")].concat());
        fs::write(&f.path, [line("x", "s"), line("target", "s")].concat()).unwrap();
        let read = excerpt_from_page(page("target"), "target", Some("lead")).unwrap();
        assert_eq!((ids(&read), read.lead), (vec!["x", "target"], None));
    }
    #[test]
    fn neighbors_never_cross_source_files() {
        let f = Fixture::new();
        f.sidechain(&[line("agent-first", "agent"), line("agent-last", "agent")].concat());
        assert_eq!(ids(&f.read("after").unwrap()), ["target", "after"]);
        assert_eq!(
            ids(&f.read("agent-first").unwrap()),
            ["agent-first", "agent-last"]
        );
    }
    #[test]
    fn duplicate_targets_in_any_file_refuse_every_record() {
        for sidechain in [false, true] {
            let f = Fixture::new();
            if sidechain {
                f.sidechain(&line("target", "duplicate"));
            } else {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&f.path)
                    .unwrap()
                    .write_all(line("target", "duplicate").as_bytes())
                    .unwrap();
            }
            assert_eq!(f.read("target"), Err(ExcerptUnavailable::TargetAmbiguous));
        }
    }
    #[test]
    fn tool_block_selection_requires_the_exact_target_block() {
        let f = Fixture::new();
        fs::write(
            &f.path,
            format!(
                "{}\n",
                json!({"type":"assistant", "uuid":"target", "sessionId":SESSION,
                "timestamp":"2026-10-25T12:00:00Z", "message":{"role":"assistant", "content":[
                    {"type":"text", "text":"synthetic"},
                    {"type":"tool_use", "id":"synthetic-call", "name":"Read", "input":{}},
                    {"type":"tool_result", "tool_use_id":"synthetic-call", "content":"synthetic"}
                ]}})
            ),
        )
        .unwrap();
        for (block, expected) in [(0, false), (1, true), (2, true), (3, false)] {
            let page = load_session_page_with(
                &f.request(),
                &PageSelection::Anchor {
                    record_uuid: "target".into(),
                    block: Some(block),
                    kind: PageTargetKind::ToolBlock,
                    generation: None,
                },
                ExcerptLimits::default(),
            );
            assert_eq!(page.is_ok(), expected);
            let result = load_session_excerpt_with(
                &f.request(),
                "target",
                Some(block),
                None,
                ExcerptLimits::default(),
            );
            if expected {
                assert_eq!(ids(&result.unwrap()), ["target"]);
            } else {
                assert_eq!(result, Err(ExcerptUnavailable::BlockMissing));
            }
        }
    }
    #[test]
    fn scan_and_discovery_limits_accumulate_across_sources_and_passes() {
        let f = Fixture::new();
        let sidechain = f.sidechain(&line("agent", "synthetic"));
        for (limits, expected) in [
            (
                ExcerptLimits {
                    scan_bytes: fs::metadata(&f.path).unwrap().len(),
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::ScanBytes,
            ),
            (
                ExcerptLimits {
                    records: 3,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::Records,
            ),
            (
                ExcerptLimits {
                    raw_lines: 3,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::RawLines,
            ),
            (
                ExcerptLimits {
                    source_files: 2,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::SourceFiles,
            ),
        ] {
            assert!(fs::metadata(&sidechain).unwrap().len() < limits.scan_bytes);
            assert_eq!(
                load_session_excerpt_with(&f.request(), "target", None, None, limits),
                Err(ExcerptUnavailable::Limit(expected))
            );
        }
    }
    #[test]
    fn partial_discovery_and_stopped_sources_cannot_claim_a_unique_target() {
        let f = Fixture::new();
        f.sidechain("not json\n");
        assert_eq!(f.read("target"), Err(ExcerptUnavailable::Incomplete));
        fs::remove_file(
            f.path
                .parent()
                .unwrap()
                .join(SESSION)
                .join("subagents/agent-a.jsonl"),
        )
        .unwrap();
        let absent = IndexedSource {
            locator: format!(
                "claude:{}",
                f.path
                    .parent()
                    .unwrap()
                    .join(SESSION)
                    .join("subagents/missing.jsonl")
                    .display()
            ),
            checkpoint: None,
        };
        let indexed = [absent];
        let mut request = f.request();
        request.indexed = &indexed;
        assert_eq!(
            load_session_excerpt_with(&request, "target", None, None, ExcerptLimits::default()),
            Err(ExcerptUnavailable::Incomplete)
        );
    }
    #[test]
    fn all_resource_ceilings_refuse_explicitly() {
        for (limits, expected) in [
            (
                ExcerptLimits {
                    scan_bytes: 1,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::ScanBytes,
            ),
            (
                ExcerptLimits {
                    line_bytes: 10,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::LineBytes,
            ),
            (
                ExcerptLimits {
                    records: 1,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::Records,
            ),
            (
                ExcerptLimits {
                    raw_lines: 1,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::RawLines,
            ),
            (
                ExcerptLimits {
                    discovery_entries: 0,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::DiscoveryEntries,
            ),
            (
                ExcerptLimits {
                    source_files: 0,
                    ..ExcerptLimits::default()
                },
                ExcerptLimit::SourceFiles,
            ),
        ] {
            let f = Fixture::new();
            assert_eq!(
                load_session_excerpt_with(&f.request(), "target", None, None, limits),
                Err(ExcerptUnavailable::Limit(expected))
            );
        }
        let f = Fixture::new();
        assert_eq!(
            load_session_excerpt_with(
                &f.request(),
                "target",
                None,
                None,
                ExcerptLimits {
                    deadline: Duration::ZERO,
                    ..ExcerptLimits::default()
                }
            ),
            Err(ExcerptUnavailable::Deadline)
        );
    }
    #[test]
    fn empty_directory_beyond_discovery_depth_is_refused_before_opening() {
        let f = Fixture::new();
        let directory = f
            .path
            .parent()
            .unwrap()
            .join(SESSION)
            .join("subagents/nested");
        fs::create_dir_all(directory).unwrap();
        let limits = ExcerptLimits {
            discovery_depth: 0,
            ..ExcerptLimits::default()
        };
        assert_eq!(
            load_session_excerpt_with(&f.request(), "target", None, None, limits),
            Err(ExcerptUnavailable::Limit(ExcerptLimit::DiscoveryDepth))
        );
    }
    #[test]
    fn cancelled_before_or_during_scan_returns_no_content() {
        let f = Fixture::new();
        let cancel = CancelToken::new();
        cancel.cancel();
        let mut request = f.request();
        request.cancel = Some(&cancel);
        assert_eq!(
            load_session_excerpt_with(&request, "target", None, None, ExcerptLimits::default()),
            Err(ExcerptUnavailable::Source(SourceUnavailable::Cancelled))
        );
        let cancel = CancelToken::new();
        let copy = cancel.clone();
        request.cancel = Some(&cancel);
        OBSERVED.with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| copy.cancel())));
        assert_eq!(
            load_session_excerpt_with(&request, "target", None, None, ExcerptLimits::default()),
            Err(ExcerptUnavailable::Source(SourceUnavailable::Cancelled))
        );
    }
    #[test]
    fn settling_refuses_a_new_subagent_with_a_duplicate_target() {
        let f = Fixture::new();
        let directory = f.path.parent().unwrap().join(SESSION).join("subagents");
        OBSERVED.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("added.jsonl"), line("target", "duplicate")).unwrap();
            }))
        });
        assert!(matches!(
            f.read("target"),
            Err(ExcerptUnavailable::Source(
                SourceUnavailable::Replaced { .. }
            ))
        ));
    }
    #[test]
    fn mutation_and_alias_are_refused() {
        let f = Fixture::new();
        OBSERVED.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                fs::write(path, line("target", "rewritten")).unwrap();
            }))
        });
        assert!(matches!(
            f.read("target"),
            Err(ExcerptUnavailable::Source(
                SourceUnavailable::Replaced { .. }
            ))
        ));
        #[cfg(unix)]
        {
            let f = Fixture::new();
            let other = f.path.with_extension("saved");
            fs::rename(&f.path, &other).unwrap();
            std::os::unix::fs::symlink(&other, &f.path).unwrap();
            assert!(f.read("target").is_err());
        }
    }
    #[test]
    fn same_inode_change_after_settling_before_final_open_is_refused() {
        for append in [false, true] {
            let f = Fixture::new();
            SETTLED.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |path| {
                    if append {
                        fs::OpenOptions::new()
                            .append(true)
                            .open(path)
                            .unwrap()
                            .write_all(line("new", "appended").as_bytes())
                            .unwrap();
                    } else {
                        fs::write(path, line("target", "rewritten after settling")).unwrap();
                    }
                }))
            });
            let result = f.read("target");
            SETTLED.with(|hook| *hook.borrow_mut() = None);
            assert!(
                matches!(
                    result,
                    Err(ExcerptUnavailable::Source(
                        SourceUnavailable::Replaced { .. }
                    ))
                ),
                "append={append}: {result:?}"
            );
        }
    }
    #[test]
    fn partial_trailing_line_is_not_an_accepted_neighbor() {
        let f = Fixture::new();
        fs::write(
            &f.path,
            format!("{}{{\"uuid\":\"target\"", line("before", "before")),
        )
        .unwrap();
        assert_eq!(f.read("target"), Err(ExcerptUnavailable::TargetMissing));
        assert_eq!(ids(&f.read("before").unwrap()), ["before"]);
    }
    #[test]
    fn paged_read_refuses_mutation_at_its_final_reopen_without_returning_records() {
        let f = Fixture::new();
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        let observed = fired.clone();
        SETTLED.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                observed.set(true);
                let original = fs::read_to_string(path).unwrap();
                let rewritten = original.replace("trigger", "CHANGED");
                assert_eq!(original.len(), rewritten.len());
                fs::write(path, rewritten).unwrap();
            }))
        });
        let result = load_session_page_with(
            &f.request(),
            &PageSelection::Anchor {
                record_uuid: "target".into(),
                block: None,
                kind: PageTargetKind::Message,
                generation: None,
            },
            ExcerptLimits::default(),
        );
        SETTLED.with(|hook| *hook.borrow_mut() = None);
        assert!(
            fired.get(),
            "the page-specific final reopen seam must execute"
        );
        assert!(
            matches!(result, Err(PageUnavailable::Changed)),
            "{result:?}"
        );
    }
    fn page_anchor(id: &str) -> PageSelection {
        PageSelection::Anchor {
            record_uuid: id.into(),
            block: None,
            kind: PageTargetKind::Message,
            generation: None,
        }
    }
    #[test]
    fn claude_pages_keep_one_or_twelve_preceding_records_and_stable_cursors() {
        let f = Fixture::new();
        fs::write(
            &f.path,
            (0..230)
                .map(|n| line(&format!("r{n}"), "synthetic"))
                .collect::<String>(),
        )
        .unwrap();
        let read = |selection: &PageSelection| {
            load_session_page_with(&f.request(), selection, ExcerptLimits::default()).unwrap()
        };
        let page = read(&page_anchor("r20"));
        assert_eq!(page.records[0].ordinal, 19);
        assert_eq!(page.target, Some(20));
        assert_eq!(page.records.len(), 200);
        let context = load_session_context_with_reader(
            &f.request(),
            &page_anchor("r20"),
            None,
            ExcerptLimits::default(),
        )
        .unwrap();
        assert_eq!(context.records[0].ordinal, 8);
        assert_eq!(context.records.len(), 200);
        let first = read(&PageSelection::First);
        assert_eq!(first.records[0].ordinal, 0);
        let next = read(&PageSelection::Cursor(first.cursor(false, 200)));
        assert_eq!(next.records.len(), 30);
        assert_eq!(next.records[0].ordinal, 200);
        let before = read(&PageSelection::Cursor(next.cursor(true, 200)));
        assert_eq!(before.records[0].ordinal, 0);
        assert!(before.backwards);
        fs::OpenOptions::new()
            .append(true)
            .open(&f.path)
            .unwrap()
            .write_all(line("extra", "synthetic").as_bytes())
            .unwrap();
        assert!(matches!(
            load_session_page_with(
                &f.request(),
                &PageSelection::Cursor(first.cursor(false, 200)),
                ExcerptLimits::default()
            ),
            Err(PageUnavailable::Changed)
        ));
    }
    #[test]
    fn claude_pages_refuse_global_duplicates_and_wrong_blocks() {
        for sidechain in [false, true] {
            let f = Fixture::new();
            if sidechain {
                f.sidechain(&line("target", "duplicate"));
            } else {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&f.path)
                    .unwrap()
                    .write_all(line("target", "duplicate").as_bytes())
                    .unwrap();
            }
            assert!(matches!(
                load_session_page_with(
                    &f.request(),
                    &page_anchor("target"),
                    ExcerptLimits::default()
                ),
                Err(PageUnavailable::Read(ExcerptUnavailable::TargetAmbiguous))
            ));
        }
        let f = Fixture::new();
        let mut anchor = page_anchor("target");
        if let PageSelection::Anchor { kind, block, .. } = &mut anchor {
            *kind = PageTargetKind::ToolBlock;
            *block = Some(0);
        }
        assert!(matches!(
            load_session_page_with(&f.request(), &anchor, ExcerptLimits::default()),
            Err(PageUnavailable::Read(ExcerptUnavailable::BlockMissing))
        ));
        if let PageSelection::Anchor {
            kind,
            block,
            generation,
            ..
        } = &mut anchor
        {
            *kind = PageTargetKind::Message;
            *block = None;
            *generation = Some("A".repeat(64));
        }
        assert!(matches!(
            load_session_page_with(&f.request(), &anchor, ExcerptLimits::default()),
            Err(PageUnavailable::InvalidCursor)
        ));
    }
    #[test]
    fn claude_page_limits_and_cancellation_cover_the_whole_scan() {
        let f = Fixture::new();
        f.sidechain(&line("agent", "synthetic"));
        for limits in [
            ExcerptLimits {
                records: 3,
                ..ExcerptLimits::default()
            },
            ExcerptLimits {
                scan_bytes: fs::metadata(&f.path).unwrap().len(),
                ..ExcerptLimits::default()
            },
            ExcerptLimits {
                raw_lines: 3,
                ..ExcerptLimits::default()
            },
            ExcerptLimits {
                deadline: std::time::Duration::ZERO,
                ..ExcerptLimits::default()
            },
        ] {
            assert!(load_session_page_with(&f.request(), &PageSelection::First, limits).is_err());
        }
        let cancel = CancelToken::new();
        let copy = cancel.clone();
        let mut request = f.request();
        request.cancel = Some(&cancel);
        OBSERVED.with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| copy.cancel())));
        assert!(matches!(
            load_session_page_with(&request, &PageSelection::First, ExcerptLimits::default()),
            Err(PageUnavailable::Read(ExcerptUnavailable::Source(
                SourceUnavailable::Cancelled
            )))
        ));
    }
    #[test]
    fn claude_pages_refuse_changes_during_scan_or_rediscovery() {
        for duplicate in [false, true] {
            let f = Fixture::new();
            let directory = f.path.parent().unwrap().join(SESSION).join("subagents");
            OBSERVED.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |path| {
                    if duplicate {
                        fs::create_dir_all(&directory).unwrap();
                        fs::write(directory.join("added.jsonl"), line("target", "duplicate"))
                            .unwrap();
                    } else {
                        fs::write(path, line("target", "rewritten")).unwrap();
                    }
                }))
            });
            assert!(
                load_session_page_with(
                    &f.request(),
                    &page_anchor("target"),
                    ExcerptLimits::default()
                )
                .is_err()
            );
            OBSERVED.with(|hook| *hook.borrow_mut() = None);
        }
    }
    #[test]
    fn claude_page_windows_cross_files_with_explicit_source_ordinals() {
        let f = Fixture::new();
        f.sidechain(
            &[
                line("agent-first", "synthetic"),
                line("agent-last", "synthetic"),
            ]
            .concat(),
        );
        let read = load_session_page_with(
            &f.request(),
            &page_anchor("after"),
            ExcerptLimits::default(),
        )
        .unwrap();
        assert_eq!(
            read.records
                .iter()
                .map(|r| (r.ordinal, r.source))
                .collect::<Vec<_>>(),
            [(1, 0), (2, 0), (3, 1), (4, 1)]
        );
        assert_eq!(read.target, Some(2));
        let read = load_session_page_with(
            &f.request(),
            &page_anchor("agent-first"),
            ExcerptLimits::default(),
        )
        .unwrap();
        assert_eq!(
            read.records
                .iter()
                .map(|r| (r.ordinal, r.source))
                .collect::<Vec<_>>(),
            [(2, 0), (3, 1), (4, 1)]
        );
        let context = load_session_context_with_reader(
            &f.request(),
            &page_anchor("agent-first"),
            None,
            ExcerptLimits::default(),
        )
        .unwrap();
        assert_eq!(
            context
                .records
                .iter()
                .map(|r| (r.ordinal, r.source))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 0), (2, 0), (3, 1), (4, 1)]
        );
        assert_eq!(
            ids(&f.read("agent-first").unwrap()),
            ["agent-first", "agent-last"]
        );
    }
}
