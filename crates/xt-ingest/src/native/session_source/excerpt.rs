//! Transient, bounded context for one saved message. Every relevant source is
//! checked before a unique target is returned; only three records are retained.
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
                excerpt_from_loaded(*loaded, record_uuid, block)
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
    let mut generation = GenerationBasis::Indexed { appended: false };
    for file in &files {
        let (basis, window, offset, count) = scan_file(request, file, record_uuid, &mut scan)?;
        generation = generation.and(basis);
        matches += count;
        if matches > 1 {
            return Err(ExcerptUnavailable::TargetAmbiguous);
        }
        if count == 1 {
            selected = window;
            target = offset;
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
    Ok(LoadedExcerpt {
        generation,
        records: selected,
        target,
    })
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

fn scan_file(
    request: &SessionSourceRequest<'_>,
    file: &ClaudeFile,
    uuid: &str,
    scan: &mut Scan<'_>,
) -> Result<(GenerationBasis, Vec<ParsedRecord>, usize, usize), ExcerptUnavailable> {
    let mut previous = None;
    let mut window = Vec::new();
    let mut offset = 0;
    let mut matches = 0;
    let mut want_next = false;
    let generation = super::streaming::scan_file(request, file, scan, |record, _| {
        if record.canonical.uuid.as_deref() == Some(uuid) {
            matches += 1;
            if matches > 1 {
                return Err(ExcerptUnavailable::TargetAmbiguous);
            }
            if let Some(before) = previous.take() {
                window.push(before);
            }
            offset = window.len();
            window.push(record.clone());
            want_next = true;
        } else if want_next {
            window.push(record.clone());
            want_next = false;
        }
        previous = Some(record);
        Ok(())
    })?;
    Ok((generation, window, offset, matches))
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
pub fn excerpt_from_loaded(
    loaded: LoadedSession,
    uuid: &str,
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
            return Ok(LoadedExcerpt {
                generation: loaded.generation,
                records: loaded.records[start..end].to_vec(),
                target: target - start,
            });
        }
        offset = end;
    }
    Err(ExcerptUnavailable::Incomplete)
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
}
