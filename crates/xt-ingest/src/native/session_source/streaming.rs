//! Shared verified streaming walk for transient Claude excerpts and pages.
use super::excerpt::{ExcerptLimit, ExcerptLimits, ExcerptUnavailable};
use super::*;
use std::time::Instant;

pub(super) struct Scan<'a> {
    pub(super) cancel: Option<&'a CancelToken>,
    pub(super) deadline: Instant,
    pub(super) limits: ExcerptLimits,
    pub(super) bytes: u64,
    pub(super) lines: u64,
    pub(super) dropped_records: u64,
    pub(super) entries: usize,
    pub(super) files: usize,
    pub(super) budget: Budget,
}
impl Scan<'_> {
    pub(super) fn check(&self) -> Result<(), ExcerptUnavailable> {
        if cancelled(self.cancel) {
            return Err(ExcerptUnavailable::Source(SourceUnavailable::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(ExcerptUnavailable::Deadline);
        }
        Ok(())
    }
    fn discovery(&mut self, depth: usize, source: bool) -> Result<(), ExcerptUnavailable> {
        self.check()?;
        if depth > self.limits.discovery_depth {
            return Err(ExcerptUnavailable::Limit(ExcerptLimit::DiscoveryDepth));
        }
        let (count, max, limit) = if source {
            (
                &mut self.files,
                self.limits.source_files,
                ExcerptLimit::SourceFiles,
            )
        } else {
            (
                &mut self.entries,
                self.limits.discovery_entries,
                ExcerptLimit::DiscoveryEntries,
            )
        };
        *count += 1;
        if *count > max {
            return Err(ExcerptUnavailable::Limit(limit));
        }
        Ok(())
    }
}

pub(super) fn identities(files: &[ClaudeFile]) -> Vec<(PathBuf, FileIdentity)> {
    files
        .iter()
        .map(|file| (file.path.clone(), FileIdentity::of(file.observed())))
        .collect()
}
pub(super) fn discover(
    request: &SessionSourceRequest<'_>,
    scan: &mut Scan<'_>,
) -> Result<Vec<ClaudeFile>, ExcerptUnavailable> {
    let projects = request.home.join(".claude/projects");
    let found = discover_claude_checked(
        &projects,
        request.native_session_id,
        &mut |depth, source| scan.discovery(depth, source),
    )?;
    let (primary, found) = resolve_claude(request, found).map_err(ExcerptUnavailable::Source)?;
    if found.sidechain_uncertain {
        return Err(ExcerptUnavailable::Incomplete);
    }
    let observed: BTreeSet<&str> = found
        .sidechains
        .iter()
        .filter_map(|file| sidechain_key(&file.path, request.native_session_id))
        .collect();
    if request
        .indexed
        .iter()
        .filter_map(IndexedSource::claude_path)
        .filter_map(|path| sidechain_key(Path::new(path), request.native_session_id))
        .any(|key| !observed.contains(key))
    {
        return Err(ExcerptUnavailable::Incomplete);
    }
    Ok(std::iter::once(primary).chain(found.sidechains).collect())
}
pub(super) fn verify(
    file: &ClaudeFile,
    source: &fs::File,
    identity: &FileIdentity,
) -> Result<(), ExcerptUnavailable> {
    for current in [
        FileIdentity::of(&source.metadata()?),
        FileIdentity::of(&fs::symlink_metadata(&file.path)?),
    ] {
        if !unchanged(&current, identity) {
            return Err(ExcerptUnavailable::Source(SourceUnavailable::Replaced {
                basis: replacement_basis(&current, identity),
            }));
        }
    }
    Ok(())
}

pub(super) fn scan_file<F>(
    request: &SessionSourceRequest<'_>,
    file: &ClaudeFile,
    scan: &mut Scan<'_>,
    mut visit: F,
) -> Result<GenerationBasis, ExcerptUnavailable>
where
    F: FnMut(ParsedRecord, usize) -> Result<(), ExcerptUnavailable>,
{
    scan.check()?;
    let (mut source, identity) =
        claude_fs::open_source(&file.path, file.observed()).map_err(|error| {
            if replaced_since(&file.path, file.observed()) {
                ExcerptUnavailable::Source(SourceUnavailable::Replaced {
                    basis: ResumeBasis::Replaced,
                })
            } else {
                error.into()
            }
        })?;
    scan.bytes = scan.bytes.saturating_add(identity.len);
    if scan.bytes > scan.limits.scan_bytes {
        return Err(ExcerptUnavailable::Limit(ExcerptLimit::ScanBytes));
    }
    observed_for_test(&file.path);
    let resume = checkpoint::resume_point_checked(
        checkpoint_for(request, &file.path),
        &mut source,
        &identity,
        &mut || scan.check(),
    )?;
    let generation = match resume.basis {
        ResumeBasis::Fresh | ResumeBasis::Unidentified => GenerationBasis::Unrecorded,
        ResumeBasis::Unchanged => GenerationBasis::Indexed { appended: false },
        ResumeBasis::Appended => GenerationBasis::Indexed { appended: true },
        basis => {
            return Err(ExcerptUnavailable::Source(SourceUnavailable::Replaced {
                basis,
            }));
        }
    };
    source.seek(SeekFrom::Start(0))?;
    let mut remaining = identity.len;
    let mut buffer = [0; READ_CHUNK];
    let mut line = Vec::new();
    while remaining > 0 {
        scan.check()?;
        let wanted = READ_CHUNK.min(remaining as usize);
        let read = match source.read(&mut buffer[..wanted]) {
            Ok(0) => {
                return Err(ExcerptUnavailable::Source(SourceUnavailable::Replaced {
                    basis: ResumeBasis::Truncated,
                }));
            }
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        remaining -= read as u64;
        for part in buffer[..read].split_inclusive(|byte| *byte == b'\n') {
            scan.check()?;
            if line.len().saturating_add(part.len()) > scan.limits.line_bytes {
                return Err(ExcerptUnavailable::Limit(ExcerptLimit::LineBytes));
            }
            line.extend_from_slice(part);
            if !part.ends_with(b"\n") {
                continue;
            }
            scan.lines += 1;
            if scan.lines > scan.limits.raw_lines {
                return Err(ExcerptUnavailable::Limit(ExcerptLimit::RawLines));
            }
            // This is the full reader's classifier, identity rule and importer
            // stopping rule, applied once to one bounded complete line.
            let parsed = parse_verified(file, &line, generation, &mut scan.budget, scan.cancel)
                .map_err(|failure| match failure {
                    ReadFailure::Cancelled => {
                        ExcerptUnavailable::Source(SourceUnavailable::Cancelled)
                    }
                    ReadFailure::TooLarge(_) => ExcerptUnavailable::Limit(ExcerptLimit::Records),
                    _ => ExcerptUnavailable::Incomplete,
                })?;
            scan.check()?;
            scan.dropped_records += parsed.dropped_records;
            let line_bytes = line.len();
            line.clear();
            if parsed.stopped.is_some() {
                return Err(ExcerptUnavailable::Incomplete);
            }
            for record in parsed.records {
                visit(record, line_bytes)?;
            }
        }
    }
    verify(file, &source, &identity)?;
    scan.check()?;
    Ok(generation)
}
