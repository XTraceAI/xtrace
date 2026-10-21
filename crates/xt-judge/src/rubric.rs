//! A grading skill's rubric: `xtrace-rubric.md` in the skill's folder, next
//! to its `SKILL.md`. It says who the skill is and how a run's output is
//! turned into a score; the skill itself stays unchanged.
//!
//! ```markdown
//! ---
//! protocol: 1
//! id: claude-md-doctor
//! name: CLAUDE.md Doctor
//! version: 0.5.1+rubric.1
//! description: Checks a project's CLAUDE.md and AGENTS.md.
//! target: project
//! ---
//!
//! ## Health of the memory files (100 points)
//! Start at 100 and subtract 20 for each critical finding …
//!
//! ## Findings
//! One finding per diagnosis …
//! ```
//!
//! The format, read by one strict line parser ([`parse`]):
//!
//! * **Header.** The file starts with a `---` line; the header ends at the
//!   next `---` line. In between, one `key: value` per line (blank lines
//!   allowed), with exactly the keys `protocol` (`1`), `id` (a lowercase
//!   slug: letters, digits and `-`, starting and ending with a letter or
//!   digit, at most [`MAX_ID_CHARS`]), `name`, `version` (no whitespace),
//!   `description` (one line) and `target` (`project`), each once. A value
//!   is the plain text after the colon, trimmed; quotes are kept as written.
//! * **Criteria.** Every `## ` heading outside a fenced code block starts a
//!   section. A section headed `## <name> (<N> points)` (or `(1 point)`) is a
//!   criterion: `N` a whole number from 1 to 100, the name unique (ignoring
//!   case), and plain-language guidance below it. 1 to [`MAX_CRITERIA`]
//!   criteria whose points add up to exactly [`TOTAL_POINTS`].
//! * **Findings.** At most one section headed `## Findings` with guidance for
//!   the findings a run reports. It has no points: `## Findings (N points)`
//!   is refused.
//! * **Code fences.** A line indented at most three spaces that starts with
//!   at least three backticks or tildes opens a fenced code block; only a
//!   line of at least as many of the same character (indented at most three
//!   spaces, nothing after them) closes it. `## ` lines inside are guidance.
//! * Text between the header and the first section is free. The whole file
//!   is UTF-8 (a leading byte-order mark is ignored), at most
//!   [`MAX_RUBRIC_BYTES`], read as a regular file without following a link.
//!
//! Anything else is a typed [`RubricError`]; messages hold no path and no
//! text from the file. Each rubric carries a short hash of its exact bytes
//! ([`Rubric::digest`]), so an edited rubric is visible in the version a run
//! is stored with ([`Rubric::stored_version`]).

use sha2::{Digest, Sha256};
use std::path::Path;
use xt_probes::regular_file::{self, RegularFileError};

/// The rubric's file name in a skill folder.
pub const RUBRIC_FILE: &str = "xtrace-rubric.md";
/// The rubric format version this reader accepts.
pub const RUBRIC_PROTOCOL: &str = "1";
/// The only target this reader accepts: a run examines one project folder.
pub const TARGET_PROJECT: &str = "project";
/// Largest accepted rubric file.
pub const MAX_RUBRIC_BYTES: u64 = 64 * 1024;
/// Most criteria a rubric may have.
pub const MAX_CRITERIA: usize = 20;
/// What a rubric's criteria add up to: a score's whole range.
pub const TOTAL_POINTS: u32 = 100;
pub const MAX_ID_CHARS: usize = 64;
pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_VERSION_CHARS: usize = 64;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
/// Longest criterion name, in characters.
pub const MAX_CRITERION_CHARS: usize = 200;
/// Hexadecimal digits of the file hash kept in [`Rubric::digest`].
pub const DIGEST_CHARS: usize = 12;
/// The heading of the optional findings section.
pub const FINDINGS_HEADING: &str = "Findings";

const HEADER_FENCE: &str = "---";
const HEADER_KEYS: [&str; 6] = ["protocol", "id", "name", "version", "description", "target"];

/// Why a rubric was refused. No path or file text in messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RubricError {
    #[error("the grading skill has no rubric file")]
    Missing,
    #[error("the rubric is not a regular file")]
    NotRegularFile,
    #[error("the rubric file is larger than allowed")]
    TooLarge,
    #[error("the rubric file could not be read")]
    Unreadable,
    #[error("the rubric is not UTF-8 text")]
    NotUtf8,
    #[error("the rubric does not start with a header between --- lines")]
    NoHeader,
    #[error("the rubric's header is not closed by a --- line")]
    HeaderUnterminated,
    #[error("a rubric header line is not a key: value line")]
    HeaderLine,
    #[error("the rubric header has a key the format does not define")]
    UnknownKey,
    #[error("the rubric header names a key twice")]
    DuplicateKey,
    #[error("the rubric header is missing a required key")]
    MissingKey,
    #[error("the rubric does not declare protocol 1")]
    Protocol,
    #[error("the rubric id is not a lowercase slug")]
    Id,
    #[error("a rubric name, version or description is empty, too long or not plain text")]
    Field,
    #[error("the rubric's target is not project")]
    Target,
    #[error("a rubric section heading is neither a criterion with points nor Findings")]
    Heading,
    #[error("a rubric criterion's points are not a whole number from 1 to 100")]
    Points,
    #[error("a rubric criterion name is empty, too long or not plain text")]
    CriterionName,
    #[error("the rubric names a criterion twice")]
    DuplicateCriterion,
    #[error("a rubric section has no guidance")]
    NoGuidance,
    #[error("the rubric has more than one Findings section")]
    DuplicateFindings,
    #[error("the rubric's Findings section has points; only criteria have points")]
    FindingsPoints,
    #[error("the rubric has no criteria")]
    NoCriteria,
    #[error("the rubric has more criteria than allowed")]
    TooManyCriteria,
    #[error("the rubric's points do not add up to 100")]
    Total,
}

/// One criterion: its name, its points and how to judge it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Criterion {
    pub name: String,
    pub points: u8,
    pub guidance: String,
}

/// A rubric that passed validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rubric {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// In the file's order.
    pub criteria: Vec<Criterion>,
    /// The `## Findings` section's guidance, if the rubric has one.
    pub findings: Option<String>,
    digest: String,
}

impl Rubric {
    /// The first [`DIGEST_CHARS`] hexadecimal digits of the SHA-256 of the
    /// rubric file's exact bytes.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The version a run of this rubric is stored with: the header's
    /// `version` and the file's [`Self::digest`], so editing the rubric
    /// without bumping its version still shows.
    pub fn stored_version(&self) -> String {
        format!("{} #{}", self.version, self.digest)
    }
}

/// Read and validate `<skill_dir>/xtrace-rubric.md`.
pub fn read(skill_dir: &Path) -> Result<Rubric, RubricError> {
    read_file(&skill_dir.join(RUBRIC_FILE))
}

/// Read and validate the rubric file at `path` (a skill folder's, or one
/// XTrace keeps for a skill: [`crate::rubric_store`]).
pub fn read_file(path: &Path) -> Result<Rubric, RubricError> {
    read_file_bytes(path).map(|(_, rubric)| rubric)
}

/// [`read_file`], with the file's exact bytes.
pub fn read_file_bytes(path: &Path) -> Result<(Vec<u8>, Rubric), RubricError> {
    let bytes = regular_file::read_capped(path, MAX_RUBRIC_BYTES).map_err(|error| match error {
        RegularFileError::Missing => RubricError::Missing,
        RegularFileError::NotRegular => RubricError::NotRegularFile,
        RegularFileError::TooLarge => RubricError::TooLarge,
        RegularFileError::Unreadable => RubricError::Unreadable,
    })?;
    let rubric = parse(&bytes)?;
    Ok((bytes, rubric))
}

/// The rubric `bytes` with its header's `id` line set to `id`, validated
/// again; every other byte stays. The rubric must already be valid.
pub fn with_id(bytes: &[u8], id: &str) -> Result<(Vec<u8>, Rubric), RubricError> {
    with_header(bytes, "id", id)
}

/// The rubric `bytes` with its header's `version` line set to `version`,
/// validated again; every other byte stays. The rubric must already be
/// valid.
pub fn with_version(bytes: &[u8], version: &str) -> Result<(Vec<u8>, Rubric), RubricError> {
    with_header(bytes, "version", version)
}

/// The header lines of rubric text: each line after the opening `---` up to
/// the closing one, as written (with its line ending).
fn header_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut fences = 0;
    text.split_inclusive('\n')
        .enumerate()
        .filter_map(move |(index, line)| {
            let bare = line.trim_end_matches(['\n', '\r']);
            let bare = if index == 0 {
                bare.strip_prefix('\u{feff}').unwrap_or(bare)
            } else {
                bare
            };
            if fences < 2 && bare.trim_end() == HEADER_FENCE && (index == 0 || fences == 1) {
                fences += 1;
                return None;
            }
            (fences == 1).then_some((index, line))
        })
}

/// A valid rubric's `bytes` with the header line of `key` set to `value`,
/// validated again; every other byte stays.
fn with_header(bytes: &[u8], key: &str, value: &str) -> Result<(Vec<u8>, Rubric), RubricError> {
    parse(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(|_| RubricError::NotUtf8)?;
    let at = header_lines(text)
        .find(|(_, line)| line.split_once(':').is_some_and(|(seen, _)| seen == key))
        .map(|(index, _)| index)
        .ok_or(RubricError::MissingKey)?;
    let mut out = String::with_capacity(text.len() + value.len());
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if index == at {
            let ending = &line[line.trim_end_matches(['\n', '\r']).len()..];
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push_str(ending);
        } else {
            out.push_str(line);
        }
    }
    let rubric = parse(out.as_bytes())?;
    Ok((out.into_bytes(), rubric))
}

/// The trimmed value of the first `key:` line in the header of rubric
/// `bytes`, read loosely so a rubric that is refused for another reason
/// still says it; none when the text has no header or no such line.
pub fn header_value(bytes: &[u8], key: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    header_lines(text).find_map(|(_, line)| {
        let (seen, value) = line.split_once(':')?;
        (seen == key).then(|| value.trim().to_owned())
    })
}

/// Validate rubric bytes against the format.
pub fn parse(bytes: &[u8]) -> Result<Rubric, RubricError> {
    if bytes.len() as u64 > MAX_RUBRIC_BYTES {
        return Err(RubricError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| RubricError::NotUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    if lines.next().map(str::trim_end) != Some(HEADER_FENCE) {
        return Err(RubricError::NoHeader);
    }
    let mut header: Vec<(&str, &str)> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == HEADER_FENCE {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = header_line(line)?;
        if !HEADER_KEYS.contains(&key) {
            return Err(RubricError::UnknownKey);
        }
        if header.iter().any(|(seen, _)| *seen == key) {
            return Err(RubricError::DuplicateKey);
        }
        header.push((key, value));
    }
    if !closed {
        return Err(RubricError::HeaderUnterminated);
    }
    let get = |key: &str| {
        header
            .iter()
            .find(|(seen, _)| *seen == key)
            .map(|(_, value)| *value)
            .ok_or(RubricError::MissingKey)
    };
    let (protocol, id, name, version, description, target) = (
        get("protocol")?,
        get("id")?,
        get("name")?,
        get("version")?,
        get("description")?,
        get("target")?,
    );
    if protocol != RUBRIC_PROTOCOL {
        return Err(RubricError::Protocol);
    }
    if !is_slug(id) {
        return Err(RubricError::Id);
    }
    if target != TARGET_PROJECT {
        return Err(RubricError::Target);
    }
    let name = field(name, MAX_NAME_CHARS)?;
    let version = field(version, MAX_VERSION_CHARS)?;
    if version.chars().any(char::is_whitespace) {
        return Err(RubricError::Field);
    }
    let description = field(description, MAX_DESCRIPTION_CHARS)?;
    let (criteria, findings) = body(lines)?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..DIGEST_CHARS]
        .to_owned();
    Ok(Rubric {
        id: id.to_owned(),
        name,
        version,
        description,
        criteria,
        findings,
        digest,
    })
}

/// One `key: value` header line: the key starts the line and is lowercase
/// letters and `_`; the value is the trimmed rest.
fn header_line(line: &str) -> Result<(&str, &str), RubricError> {
    let (key, value) = line.split_once(':').ok_or(RubricError::HeaderLine)?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(RubricError::HeaderLine);
    }
    Ok((key, value.trim()))
}

/// Whether `id` is a rubric id: a lowercase slug of at most [`MAX_ID_CHARS`].
pub fn is_slug(id: &str) -> bool {
    let alnum = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_ID_CHARS
        && bytes.iter().all(|byte| alnum(*byte) || *byte == b'-')
        && alnum(bytes[0])
        && alnum(bytes[bytes.len() - 1])
}

/// Plain text: not empty, at most `cap` characters, no control characters.
fn plain(text: &str, cap: usize) -> bool {
    !text.is_empty() && text.chars().count() <= cap && !text.chars().any(char::is_control)
}

fn field(value: &str, cap: usize) -> Result<String, RubricError> {
    plain(value, cap)
        .then(|| value.to_owned())
        .ok_or(RubricError::Field)
}

/// What a `## ` heading opens.
enum Section {
    Criterion { name: String, points: u8 },
    Findings,
}

/// The sections after the header: criteria in order and the optional
/// findings guidance.
fn body<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<(Vec<Criterion>, Option<String>), RubricError> {
    let mut criteria: Vec<Criterion> = Vec::new();
    let mut findings: Option<String> = None;
    let mut current: Option<(Section, Vec<&str>)> = None;
    let mut fence: Option<Fence> = None;
    let mut close = |section: Option<(Section, Vec<&str>)>| -> Result<(), RubricError> {
        let Some((section, text)) = section else {
            return Ok(());
        };
        let guidance = text.join("\n").trim().to_owned();
        if guidance.is_empty() {
            return Err(RubricError::NoGuidance);
        }
        match section {
            Section::Findings => {
                if findings.replace(guidance).is_some() {
                    return Err(RubricError::DuplicateFindings);
                }
            }
            Section::Criterion { name, points } => {
                let folded = name.to_lowercase();
                if criteria
                    .iter()
                    .any(|criterion| criterion.name.to_lowercase() == folded)
                {
                    return Err(RubricError::DuplicateCriterion);
                }
                criteria.push(Criterion {
                    name,
                    points,
                    guidance,
                });
            }
        }
        Ok(())
    };
    for line in lines {
        if let Some(open) = fence {
            if open.closed_by(line) {
                fence = None;
            }
        } else if let Some(open) = Fence::opened_by(line) {
            fence = Some(open);
        } else if let Some(heading) = line.strip_prefix("## ") {
            close(current.take())?;
            current = Some((section(heading.trim())?, Vec::new()));
            continue;
        }
        if let Some((_, text)) = current.as_mut() {
            text.push(line);
        }
    }
    close(current.take())?;
    if criteria.is_empty() {
        return Err(RubricError::NoCriteria);
    }
    if criteria.len() > MAX_CRITERIA {
        return Err(RubricError::TooManyCriteria);
    }
    let total: u32 = criteria
        .iter()
        .map(|criterion| u32::from(criterion.points))
        .sum();
    if total != TOTAL_POINTS {
        return Err(RubricError::Total);
    }
    Ok((criteria, findings))
}

/// An open fenced code block, CommonMark style: a line indented at most
/// three spaces that starts with at least three backticks or tildes opens
/// one (a backtick fence's info string has no backtick); only a line indented
/// at most three spaces holding at least as many of the same character and
/// nothing else but spaces closes it.
#[derive(Clone, Copy)]
struct Fence {
    marker: u8,
    length: usize,
}

/// A line's text after at most three leading spaces; `None` when it is
/// indented further (an indented code line, never a fence).
fn fence_indent(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() <= 3).then_some(rest)
}

/// How many `marker` bytes `text` starts with.
fn run_of(text: &str, marker: u8) -> usize {
    text.bytes().take_while(|byte| *byte == marker).count()
}

impl Fence {
    fn opened_by(line: &str) -> Option<Self> {
        let rest = fence_indent(line)?;
        let marker = *rest
            .as_bytes()
            .first()
            .filter(|byte| matches!(byte, b'`' | b'~'))?;
        let length = run_of(rest, marker);
        if length < 3 || (marker == b'`' && rest[length..].contains('`')) {
            return None;
        }
        Some(Self { marker, length })
    }

    fn closed_by(self, line: &str) -> bool {
        let Some(rest) = fence_indent(line) else {
            return false;
        };
        let length = run_of(rest, self.marker);
        length >= self.length
            && rest[length..]
                .trim_matches(|c| c == ' ' || c == '\t')
                .is_empty()
    }
}

/// A heading's text: `Findings`, or `<name> (<N> points)` / `(1 point)`.
fn section(heading: &str) -> Result<Section, RubricError> {
    if heading == FINDINGS_HEADING {
        return Ok(Section::Findings);
    }
    let inner = heading.strip_suffix(')').ok_or(RubricError::Heading)?;
    let (name, points) = inner.rsplit_once(" (").ok_or(RubricError::Heading)?;
    if name.trim().eq_ignore_ascii_case(FINDINGS_HEADING) {
        return Err(RubricError::FindingsPoints);
    }
    let (number, unit) = points.split_once(' ').ok_or(RubricError::Heading)?;
    if unit != "points" && !(unit == "point" && number == "1") {
        return Err(RubricError::Heading);
    }
    let points = (!number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && !number.starts_with('0'))
    .then(|| number.parse::<u32>().ok())
    .flatten()
    .filter(|points| (1..=TOTAL_POINTS).contains(points))
    .and_then(|points| u8::try_from(points).ok())
    .ok_or(RubricError::Points)?;
    let name = name.trim();
    if !plain(name, MAX_CRITERION_CHARS) {
        return Err(RubricError::CriterionName);
    }
    Ok(Section::Criterion {
        name: name.to_owned(),
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "---\nprotocol: 1\nid: test-skill\nname: Test skill\nversion: 1.2\n\
         description: Checks things.\ntarget: project\n---\n\nFree text.\n\n\
         ## Accuracy (60 points)\nJudge accuracy.\n\n## Style (40 points)\nJudge style.\n\n\
         ## Findings\nOne per problem.\n";

    fn with(from: &str, to: &str) -> Result<Rubric, RubricError> {
        assert!(GOOD.contains(from), "{from}");
        parse(GOOD.replacen(from, to, 1).as_bytes())
    }

    #[test]
    fn reads_the_header_criteria_and_findings() {
        let rubric = parse(GOOD.as_bytes()).unwrap();
        assert_eq!(rubric.id, "test-skill");
        assert_eq!(rubric.name, "Test skill");
        assert_eq!(rubric.version, "1.2");
        assert_eq!(rubric.description, "Checks things.");
        assert_eq!(
            rubric.criteria,
            [
                Criterion {
                    name: "Accuracy".into(),
                    points: 60,
                    guidance: "Judge accuracy.".into()
                },
                Criterion {
                    name: "Style".into(),
                    points: 40,
                    guidance: "Judge style.".into()
                },
            ]
        );
        assert_eq!(rubric.findings.as_deref(), Some("One per problem."));
        assert_eq!(rubric.digest().len(), DIGEST_CHARS);
        assert!(rubric.digest().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(rubric.stored_version(), format!("1.2 #{}", rubric.digest()));
    }

    #[test]
    fn the_digest_follows_every_byte() {
        let a = parse(GOOD.as_bytes()).unwrap();
        let b = with("Judge style.", "Judge style!").unwrap();
        assert_eq!(a.version, b.version);
        assert_ne!(a.digest(), b.digest());
        assert_ne!(a.stored_version(), b.stored_version());
    }

    #[test]
    fn accepts_crlf_a_bom_one_point_and_no_findings() {
        let crlf = GOOD.replace('\n', "\r\n");
        assert_eq!(parse(crlf.as_bytes()).unwrap().criteria.len(), 2);
        let bom = format!("\u{feff}{GOOD}");
        assert!(parse(bom.as_bytes()).is_ok());
        let one = with(
            "## Style (40 points)\nJudge style.",
            "## Style (39 points)\nJudge.\n\n## Tidy (1 point)\nTidy.",
        )
        .unwrap();
        assert_eq!(one.criteria[2].points, 1);
        let none = with("## Findings\nOne per problem.\n", "").unwrap();
        assert_eq!(none.findings, None);
        let whole = with(
            "## Accuracy (60 points)\nJudge accuracy.\n\n## Style (40 points)\nJudge style.",
            "## Everything (100 points)\nAll of it.",
        )
        .unwrap();
        assert_eq!(whole.criteria.len(), 1);
    }

    #[test]
    fn a_heading_inside_a_code_fence_is_guidance() {
        let rubric = with(
            "Judge style.",
            "Judge style.\n```markdown\n## Not a section (5 points)\n```\n~~~\n## Nor this\n~~~",
        )
        .unwrap();
        assert_eq!(rubric.criteria.len(), 2);
        assert!(rubric.criteria[1].guidance.contains("## Not a section"));
    }

    #[test]
    fn fences_follow_commonmark() {
        // A language tag inside an open fence does not close it.
        let tagged = with(
            "Judge style.",
            "Judge style.\n```markdown\n```rust\n## Inside (5 points)\n```\nAfter.",
        )
        .unwrap();
        assert_eq!(tagged.criteria.len(), 2);
        assert!(tagged.criteria[1].guidance.contains("## Inside"));
        assert!(tagged.criteria[1].guidance.ends_with("After."));
        // Four backticks are closed only by four or more.
        let long = with(
            "Judge style.",
            "Judge style.\n````\n```\n## Inside (5 points)\n`````\n",
        )
        .unwrap();
        assert_eq!(long.criteria.len(), 2);
        assert!(long.criteria[1].guidance.contains("## Inside"));
        // A tilde line does not close a backtick fence.
        let mixed = with(
            "Judge style.",
            "Judge style.\n```\n~~~\n## Inside (5 points)\n```",
        )
        .unwrap();
        assert_eq!(mixed.criteria.len(), 2);
        // Indented four spaces, ``` is code, not a fence: the heading after
        // it is a real section (and here breaks the total).
        assert_eq!(
            with(
                "Judge style.",
                "Judge style.\n    ```\n## Extra (5 points)\nMore.\n    ```",
            ),
            Err(RubricError::Total)
        );
        // Up to three spaces still opens one.
        let three = with(
            "Judge style.",
            "Judge style.\n   ```\n## Inside (5 points)\n   ```",
        )
        .unwrap();
        assert_eq!(three.criteria.len(), 2);
        // A backtick fence's info string has no backtick.
        assert_eq!(
            with(
                "Judge style.",
                "Judge style.\n``` a`b\n## Extra (5 points)\nMore.",
            ),
            Err(RubricError::Total)
        );
    }

    #[test]
    fn findings_has_no_points() {
        for heading in ["## Findings (10 points)", "## findings (1 point)"] {
            assert_eq!(
                with("## Findings", heading),
                Err(RubricError::FindingsPoints),
                "{heading}"
            );
        }
    }

    #[test]
    fn points_must_sum_to_exactly_100() {
        assert_eq!(
            with("Style (40 points)", "Style (41 points)"),
            Err(RubricError::Total)
        );
        assert_eq!(
            with("Style (40 points)", "Style (39 points)"),
            Err(RubricError::Total)
        );
    }

    #[test]
    fn header_keys_are_exact() {
        for key in HEADER_KEYS {
            let line = GOOD
                .lines()
                .find(|line| line.starts_with(&format!("{key}:")))
                .unwrap();
            assert_eq!(
                with(&format!("{line}\n"), ""),
                Err(RubricError::MissingKey),
                "{key}"
            );
        }
        assert_eq!(
            with("target: project\n", "target: project\nextra: 1\n"),
            Err(RubricError::UnknownKey)
        );
        assert_eq!(
            with("name: Test skill\n", "name: Test skill\nname: Again\n"),
            Err(RubricError::DuplicateKey)
        );
        assert_eq!(
            with("name: Test skill\n", "  name: Test skill\n"),
            Err(RubricError::HeaderLine)
        );
        assert_eq!(
            with("name: Test skill\n", "# a comment\n"),
            Err(RubricError::HeaderLine)
        );
        assert_eq!(
            with("protocol: 1", "protocol: 2"),
            Err(RubricError::Protocol)
        );
        assert_eq!(
            with("target: project", "target: session"),
            Err(RubricError::Target)
        );
        assert_eq!(with("name: Test skill", "name:"), Err(RubricError::Field));
        assert_eq!(
            with("version: 1.2", "version: 1 2"),
            Err(RubricError::Field)
        );
        let long = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert_eq!(
            with(
                "description: Checks things.",
                &format!("description: {long}")
            ),
            Err(RubricError::Field)
        );
        assert_eq!(
            with("name: Test skill", "name: Tab\there"),
            Err(RubricError::Field)
        );
    }

    #[test]
    fn the_id_is_a_lowercase_slug() {
        for bad in [
            "Test-skill",
            "test skill",
            "-test",
            "test-",
            "",
            "test_skill",
            "tést",
        ] {
            assert_eq!(
                with("id: test-skill", &format!("id: {bad}")),
                Err(RubricError::Id),
                "{bad}"
            );
        }
        let long = "a".repeat(MAX_ID_CHARS + 1);
        assert_eq!(
            with("id: test-skill", &format!("id: {long}")),
            Err(RubricError::Id)
        );
        for good in ["a", "a-1", "claude-md-doctor", &"a".repeat(MAX_ID_CHARS)] {
            assert!(
                with("id: test-skill", &format!("id: {good}")).is_ok(),
                "{good}"
            );
        }
    }

    #[test]
    fn the_header_must_open_and_close() {
        assert_eq!(parse(b"protocol: 1\n"), Err(RubricError::NoHeader));
        assert_eq!(parse(b""), Err(RubricError::NoHeader));
        assert_eq!(
            parse(b"---\nprotocol: 1\nid: x\n"),
            Err(RubricError::HeaderUnterminated)
        );
        // Without its closing line, the body is read as header lines.
        assert_eq!(
            parse(GOOD.replacen("---\n\nFree", "\nFree", 1).as_bytes()),
            Err(RubricError::HeaderLine)
        );
    }

    #[test]
    fn criteria_are_checked() {
        assert_eq!(
            with("## Style (40 points)", "## Accuracy (40 points)"),
            Err(RubricError::DuplicateCriterion)
        );
        assert_eq!(
            with("## Style (40 points)", "## ACCURACY (40 points)"),
            Err(RubricError::DuplicateCriterion)
        );
        for heading in [
            "## Style",
            "## Style (40)",
            "## Style 40 points",
            "## Style (40 pts)",
            "## (40 points)",
            "## Findings (40 points) extra",
        ] {
            assert_eq!(
                with("## Style (40 points)", heading),
                Err(RubricError::Heading),
                "{heading}"
            );
        }
        for points in ["0", "101", "040", "-4", "4.0", "x"] {
            assert_eq!(
                with("Style (40 points)", &format!("Style ({points} points)")),
                Err(RubricError::Points),
                "{points}"
            );
        }
        assert_eq!(
            with("Style (40 points)", "Style (40 point)"),
            Err(RubricError::Heading)
        );
        let long = "n".repeat(MAX_CRITERION_CHARS + 1);
        assert_eq!(
            with("## Style (40 points)", &format!("## {long} (40 points)")),
            Err(RubricError::CriterionName)
        );
        assert_eq!(
            with("## Style (40 points)", "## Sty\u{7}le (40 points)"),
            Err(RubricError::CriterionName)
        );
        assert_eq!(with("Judge style.", ""), Err(RubricError::NoGuidance));
        assert_eq!(with("One per problem.", "  "), Err(RubricError::NoGuidance));
        assert_eq!(
            with(
                "## Findings\nOne per problem.\n",
                "## Findings\nA.\n\n## Findings\nB.\n"
            ),
            Err(RubricError::DuplicateFindings)
        );
    }

    #[test]
    fn criteria_count_is_bounded() {
        let header = &GOOD[..GOOD.find("Free text").unwrap()];
        assert_eq!(parse(header.as_bytes()), Err(RubricError::NoCriteria));
        let findings_only = format!("{header}## Findings\nOnly this.\n");
        assert_eq!(
            parse(findings_only.as_bytes()),
            Err(RubricError::NoCriteria)
        );
        let mut many = header.to_owned();
        for index in 0..=MAX_CRITERIA {
            let points = if index == 0 { 80 } else { 1 };
            many.push_str(&format!("## C{index} ({points} points)\nG.\n"));
        }
        assert_eq!(parse(many.as_bytes()), Err(RubricError::TooManyCriteria));
        let mut most = header.to_owned();
        for index in 0..MAX_CRITERIA {
            most.push_str(&format!("## C{index} (5 points)\nG.\n"));
        }
        assert_eq!(parse(most.as_bytes()).unwrap().criteria.len(), MAX_CRITERIA);
    }

    #[test]
    fn size_and_encoding_are_checked() {
        let mut big = GOOD.to_owned();
        big.push_str(&"x".repeat(MAX_RUBRIC_BYTES as usize));
        assert_eq!(parse(big.as_bytes()), Err(RubricError::TooLarge));
        let mut bad = GOOD.as_bytes().to_vec();
        bad.push(0xff);
        assert_eq!(parse(&bad), Err(RubricError::NotUtf8));
    }

    #[cfg(unix)]
    #[test]
    fn read_takes_only_a_regular_file() {
        let dir = tempfile::Builder::new()
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
            .unwrap();
        assert_eq!(read(dir.path()), Err(RubricError::Missing));
        std::fs::write(dir.path().join("elsewhere.md"), GOOD).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("elsewhere.md"),
            dir.path().join(RUBRIC_FILE),
        )
        .unwrap();
        assert_eq!(read(dir.path()), Err(RubricError::NotRegularFile));
        std::fs::remove_file(dir.path().join(RUBRIC_FILE)).unwrap();
        std::fs::write(
            dir.path().join(RUBRIC_FILE),
            vec![b'x'; MAX_RUBRIC_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(read(dir.path()), Err(RubricError::TooLarge));
        std::fs::write(dir.path().join(RUBRIC_FILE), GOOD).unwrap();
        assert_eq!(read(dir.path()).unwrap().id, "test-skill");
    }

    /// The format page's example is a valid rubric.
    #[test]
    fn the_documented_example_parses() {
        let page = include_str!("../../../docs/GRADING-RUBRIC.md");
        let (_, rest) = page.split_once("```markdown\n").unwrap();
        let (example, _) = rest.split_once("\n```\n").unwrap();
        let rubric = parse(format!("{example}\n").as_bytes()).unwrap();
        assert_eq!(rubric.id, "readme-check");
        assert_eq!(rubric.criteria.len(), 2);
        assert!(rubric.findings.is_some());
        assert!(page.contains(&format!("`1.0 #{}`", "3fa2b9c1d0e4")));
        assert_eq!("3fa2b9c1d0e4".len(), DIGEST_CHARS);
    }

    #[test]
    fn errors_name_no_path_or_content() {
        for error in [
            RubricError::Missing,
            RubricError::Heading,
            RubricError::Total,
        ] {
            assert!(!error.to_string().contains('/'));
        }
    }
}
