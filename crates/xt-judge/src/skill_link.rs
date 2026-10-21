//! Skills from a public GitHub link: the link's one reading ([`parse`]),
//! the downloaded archive unpacked within bounds ([`unpack`]), the skills it
//! holds ([`find`]), whether each can be kept ([`keep_states`]), keeping one
//! ([`keep`]) and where kept skills came from ([`LinkedSkills`]).
//!
//! Nothing here uses the network: the app resolves the link to one commit
//! and downloads that commit's archive (`crate` users pass its bytes in).
//! Nothing downloaded is run: a skill is only read and copied.
//!
//! * **Links** ([`parse`]): `https://github.com/<owner>/<repo>` (with or
//!   without `.git`), `…/tree/<ref>` and `…/tree/<ref>/<path>`, and
//!   `…/blob/<ref>/<path>/SKILL.md` (its folder), each with or without a
//!   trailing slash, with or without `https://`; a query or fragment
//!   (`?tab=…`, `#readme`) is dropped. Anything else is refused
//!   ([`LinkError`]). A `<ref>` is
//!   one path segment, so a branch whose name has a `/` cannot be named.
//! * **Unpacking** ([`unpack`]): a gzip-compressed tar archive whose entries
//!   share one top folder (GitHub's archive layout). Every entry counts
//!   toward [`ArchiveLimits::entries`] and every entry's data toward
//!   [`ArchiveLimits::bytes`] (the decompressed stream is capped too). An
//!   absolute path, a `..` part or a name that is not UTF-8 refuses the
//!   archive. Only entries at or under the link's folder are written, into
//!   a private folder: folders, regular files (mode `0644`, or `0755` when
//!   the archive marks it executable) and links, which are recorded rather
//!   than created ([`StagedLink`]). A link whose target is absolute or climbs
//!   out of the repository, a hard link, a device or a FIFO there refuses
//!   the archive ([`ArchiveError::Unsafe`]).
//! * **Finding** ([`find`]): the link's folder when it has a `SKILL.md`;
//!   otherwise each folder with a `SKILL.md` up to [`MAX_FIND_DEPTH`] levels
//!   below it (not inside a skill already found, never inside `.git`), at
//!   most [`MAX_FOUND`], by path.
//! * **Keeping** ([`keep_states`], one rule): a skill can be kept when it
//!   fits the copy limits grading uses for installed skills, every link in it
//!   names a regular file inside it, its `SKILL.md` reads as UTF-8 text, it
//!   has a usable folder name ([`folder_name`]), that name is free in both
//!   `~/.agents/skills` and `~/.claude/skills`, and no skill listed before it
//!   has the same name. [`keep`] copies it into
//!   `~/.agents/skills/.xtrace-keeping-…` and renames that copy to
//!   `~/.agents/skills/<name>`, never over an existing entry, then links
//!   `~/.claude/skills/<name>` to it when that name is free.
//! * **Records** ([`LinkedSkills`]): `<XTrace data>/linked-skills.json`
//!   holds where each kept skill came from (repository, path, the ref asked
//!   for, commit, when), for checking for updates later. Nothing is written
//!   inside the skill's folder.

use crate::{
    installed::{RubricStatus, front_matter},
    rubric::{self, RUBRIC_FILE, RubricError},
    skill::{INSTRUCTIONS_FILE, MAX_INSTRUCTIONS_BYTES},
    staging::{MAX_INSTALLED_SKILL_BYTES, MAX_INSTALLED_SKILL_FILES, MAX_SKILL_DEPTH},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use xt_probes::{process::CancelFlag, regular_file};

/// The only site links are accepted for.
pub const GITHUB_HOST: &str = "github.com";
/// Longest link read, in bytes.
pub const MAX_LINK_BYTES: usize = 2_048;
/// Most path segments a link's folder may have.
pub const MAX_LINK_SEGMENTS: usize = 32;
/// Largest archive downloaded.
pub const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Most entries an archive may have.
pub const MAX_ENTRIES: usize = 5_000;
/// Most bytes an archive's entries may hold together, unpacked.
pub const MAX_UNPACKED_BYTES: u64 = 100 * 1024 * 1024;
/// Most path parts an archive entry may have.
pub const MAX_ENTRY_PARTS: usize = 64;
/// Longest link target kept, in bytes.
pub const MAX_LINK_TARGET_BYTES: usize = 4_096;
/// How many folder levels below the link's folder skills are looked for.
pub const MAX_FIND_DEPTH: usize = 3;
/// Most skills one link lists.
pub const MAX_FOUND: usize = 100;
/// Hexadecimal digits of a commit shown.
pub const SHORT_COMMIT_CHARS: usize = 7;
/// Longest folder name a kept skill gets, in bytes.
pub const MAX_FOLDER_NAME_BYTES: usize = 100;
/// Longest skill name used as a folder name, in characters.
pub const MAX_SKILL_NAME_CHARS: usize = 64;
/// The start of a copy being kept, hidden in `~/.agents/skills`.
pub const KEEP_PREFIX: &str = ".xtrace-keeping-";
/// Where kept skills' sources are recorded, in XTrace's data folder.
pub const LINKED_SKILLS_FILE: &str = "linked-skills.json";
/// The look-ups' private staging folders' parent, in XTrace's data folder.
pub const STAGING_DIR: &str = "skill-link-staging";
/// Most kept skills recorded.
pub const MAX_RECORDS: usize = 2_000;
/// Largest records file read.
pub const MAX_RECORDS_BYTES: u64 = 4 * 1024 * 1024;
/// The records file's layout.
const RECORDS_LAYOUT: u32 = 1;
pub use crate::installed::{AGENTS_SKILLS, CLAUDE_SKILLS};

/// A public GitHub link, read by [`parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubLink {
    pub owner: String,
    pub repo: String,
    /// The branch, tag or commit named; none means the default branch.
    pub reference: Option<String>,
    /// The folder inside the repository, `/`-separated; empty for its top.
    pub path: String,
}

impl GithubLink {
    /// `github.com/<owner>/<repo>`.
    pub fn repository(&self) -> String {
        format!("{GITHUB_HOST}/{}/{}", self.owner, self.repo)
    }
}

/// Why a link is not one XTrace reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// Not an `https://github.com/` link.
    #[error("not an https://github.com link")]
    NotGithub,
    /// A GitHub link of another shape (an issue, a file other than
    /// `SKILL.md`, a query, …).
    #[error("not a link to a repository or a folder in one")]
    Shape,
    /// An owner, repository, ref or folder name GitHub does not allow, or
    /// one XTrace does not read.
    #[error("a name in the link is not allowed")]
    Name,
}

/// The one reading of a link (see the module docs).
pub fn parse(text: &str) -> Result<GithubLink, LinkError> {
    let text = text.trim();
    if text.len() > MAX_LINK_BYTES {
        return Err(LinkError::Shape);
    }
    // A query or fragment GitHub's pages add (`?tab=readme-ov-file`,
    // `#readme`) is dropped.
    let text = text.split(['?', '#']).next().unwrap_or_default();
    let rest = match text.get(..8) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https://") => &text[8..],
        // Another scheme; a link without one is read as `https://`.
        _ if text.contains("://") => return Err(LinkError::NotGithub),
        _ => text,
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !host.eq_ignore_ascii_case(GITHUB_HOST) && !host.eq_ignore_ascii_case("www.github.com") {
        return Err(LinkError::NotGithub);
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(LinkError::Shape);
    }
    let (owner, repo, reference, folder): (&str, &str, Option<&str>, &[&str]) =
        match parts.as_slice() {
            [owner, repo] => (owner, repo.strip_suffix(".git").unwrap_or(repo), None, &[]),
            [owner, repo, "tree", reference, folder @ ..] => (owner, repo, Some(reference), folder),
            [owner, repo, "blob", reference, folder @ .., file] if *file == INSTRUCTIONS_FILE => {
                (owner, repo, Some(reference), folder)
            }
            _ => return Err(LinkError::Shape),
        };
    if !owner_name(owner) || !repo_name(repo) || reference.is_some_and(|name| !ref_name(name)) {
        return Err(LinkError::Name);
    }
    if folder.len() > MAX_LINK_SEGMENTS {
        return Err(LinkError::Name);
    }
    let folder = folder
        .iter()
        .map(|part| decode(part).filter(|part| entry_part(part)))
        .collect::<Option<Vec<String>>>()
        .ok_or(LinkError::Name)?;
    Ok(GithubLink {
        owner: (*owner).to_owned(),
        repo: repo.to_owned(),
        reference: reference.map(str::to_owned),
        path: folder.join("/"),
    })
}

/// A GitHub account name: 1–39 letters, digits and `-`, not starting with `-`.
fn owner_name(name: &str) -> bool {
    (1..=39).contains(&name.len())
        && !name.starts_with('-')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A repository name: 1–100 letters, digits, `.`, `_` and `-`, not `.` or `..`.
fn repo_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A branch, tag or commit XTrace names in a request: 1–255 letters,
/// digits, `.`, `_`, `+` and `-`, not starting with `.` or `-`, without
/// `..`, not ending in `.lock`.
fn ref_name(name: &str) -> bool {
    (1..=255).contains(&name.len())
        && !name.starts_with(['.', '-'])
        && !name.contains("..")
        && !name.ends_with(".lock")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// A path part `%`-decoded as UTF-8; none when it is not.
fn decode(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A folder or file name inside a repository that XTrace writes: not
/// empty, not `.` or `..`, without `/`, `\` or control characters, at most
/// 255 bytes.
fn entry_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 255
        && part != "."
        && part != ".."
        && !part.contains(['/', '\\'])
        && !part.chars().any(char::is_control)
}

/// Whether `text` is a full commit ID: 40 (or 64) lowercase hexadecimal
/// digits.
pub fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The first [`SHORT_COMMIT_CHARS`] digits of a commit.
pub fn short_commit(commit: &str) -> &str {
    commit.get(..SHORT_COMMIT_CHARS).unwrap_or(commit)
}

/// The bounds of one archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    pub entries: usize,
    pub bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            entries: MAX_ENTRIES,
            bytes: MAX_UNPACKED_BYTES,
        }
    }
}

/// Why an archive was not unpacked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    #[error("the archive has more entries than XTrace reads")]
    TooManyEntries,
    #[error("the archive holds more data than XTrace unpacks")]
    TooLarge,
    /// An absolute path, a `..` part, a name that is not UTF-8, a link out
    /// of the repository, a hard link, a device or a FIFO.
    #[error("the archive has an entry XTrace does not unpack")]
    Unsafe,
    /// Not a gzip-compressed tar archive of one top folder, or a duplicate
    /// or conflicting entry.
    #[error("the archive cannot be read")]
    Malformed,
    /// Nothing in the archive is at or under the link's folder.
    #[error("the link's folder is not in the archive")]
    PathNotFound,
    #[error("cancelled")]
    Cancelled,
    /// The staging folder could not be written.
    #[error("the archive could not be written")]
    Io,
}

/// A link inside the unpacked folder, recorded instead of created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedLink {
    /// Its path from the repository's top, `/`-separated.
    pub path: String,
    /// What it names, as written in the archive (relative).
    pub target: String,
}

/// An archive unpacked into a staging folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unpacked {
    /// The staging folder: the repository's top.
    pub root: PathBuf,
    /// The links at or under the link's folder.
    pub links: Vec<StagedLink>,
}

/// Unpack the gzip-compressed tar `archive` into the existing, empty,
/// private folder `into`, within `limits`, writing only what is at or
/// under `prefix` (a [`GithubLink::path`]). `cancel` is checked before each
/// entry and each read.
pub fn unpack(
    archive: impl Read,
    into: &Path,
    prefix: &str,
    limits: ArchiveLimits,
    cancel: &CancelFlag,
) -> Result<Unpacked, ArchiveError> {
    // Headers and padding are read too: allow one block per entry and a
    // margin over the data the entries may hold.
    let cap = limits
        .bytes
        .saturating_add((limits.entries as u64).saturating_mul(2 * 512))
        .saturating_add(1024 * 1024);
    let mut reader = CappedReader {
        inner: flate2::read::GzDecoder::new(archive),
        left: cap,
        over: false,
        cancel,
        cancelled: false,
    };
    let unpacked = unpack_from(&mut reader, into, prefix, limits, cancel);
    if reader.cancelled {
        return Err(ArchiveError::Cancelled);
    }
    if reader.over {
        return Err(ArchiveError::TooLarge);
    }
    unpacked
}

/// A decompressed stream that ends in an error past `left` bytes or once
/// cancelled, and says which.
struct CappedReader<'a, R> {
    inner: R,
    left: u64,
    over: bool,
    cancel: &'a CancelFlag,
    cancelled: bool,
}

impl<R: Read> Read for CappedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            self.cancelled = true;
            return Err(std::io::Error::other("cancelled"));
        }
        let read = self.inner.read(buf)?;
        if read as u64 > self.left {
            self.over = true;
            return Err(std::io::Error::other("too large"));
        }
        self.left -= read as u64;
        Ok(read)
    }
}

fn unpack_from(
    reader: &mut impl Read,
    into: &Path,
    prefix: &str,
    limits: ArchiveLimits,
    cancel: &CancelFlag,
) -> Result<Unpacked, ArchiveError> {
    let prefix: Vec<&str> = if prefix.is_empty() {
        Vec::new()
    } else {
        prefix.split('/').collect()
    };
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|_| ArchiveError::Malformed)?;
    let mut top: Option<String> = None;
    let mut count = 0_usize;
    let mut bytes = 0_u64;
    let mut found = false;
    let mut links: Vec<StagedLink> = Vec::new();
    for entry in entries {
        if cancel.is_cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        let mut entry = entry.map_err(|_| ArchiveError::Malformed)?;
        count += 1;
        if count > limits.entries {
            return Err(ArchiveError::TooManyEntries);
        }
        let kind = entry.header().entry_type();
        let size = entry.size();
        bytes = bytes.saturating_add(size);
        if bytes > limits.bytes {
            return Err(ArchiveError::TooLarge);
        }
        // GitHub's archives start with the commit as a global header.
        if kind.is_pax_global_extensions() {
            continue;
        }
        let parts = entry_parts(&entry.path_bytes())?;
        let Some((first, rest)) = parts.split_first() else {
            return Err(ArchiveError::Malformed);
        };
        match &top {
            None => top = Some(first.clone()),
            Some(top) if top != first => return Err(ArchiveError::Malformed),
            Some(_) => {}
        }
        if rest.is_empty() {
            if kind.is_dir() {
                continue;
            }
            return Err(ArchiveError::Malformed);
        }
        if rest.len() < prefix.len() || rest[..prefix.len()] != prefix[..] {
            continue;
        }
        found = true;
        let relative = rest.join("/");
        let target = into.join(&relative);
        if kind.is_dir() {
            make_folders(&target)?;
        } else if kind.is_file() || kind.is_contiguous() {
            if let Some(parent) = target.parent() {
                make_folders(parent)?;
            }
            let executable = entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0);
            write_entry(&target, &mut entry, size, executable)?;
        } else if kind.is_symlink() {
            let link = entry
                .link_name_bytes()
                .ok_or(ArchiveError::Malformed)?
                .into_owned();
            let link = String::from_utf8(link).map_err(|_| ArchiveError::Unsafe)?;
            let base: Vec<&str> = rest[..rest.len() - 1].iter().map(String::as_str).collect();
            if link.len() > MAX_LINK_TARGET_BYTES || resolve(&base, &link).is_none() {
                return Err(ArchiveError::Unsafe);
            }
            if let Some(parent) = target.parent() {
                make_folders(parent)?;
            }
            links.push(StagedLink {
                path: relative,
                target: link,
            });
        } else if kind.is_hard_link()
            || kind.is_character_special()
            || kind.is_block_special()
            || kind.is_fifo()
        {
            return Err(ArchiveError::Unsafe);
        } else {
            return Err(ArchiveError::Malformed);
        }
    }
    if top.is_none() {
        return Err(ArchiveError::Malformed);
    }
    if !found {
        return Err(ArchiveError::PathNotFound);
    }
    // A link may not share its path with another entry.
    for (at, link) in links.iter().enumerate() {
        if std::fs::symlink_metadata(into.join(&link.path)).is_ok()
            || links[..at].iter().any(|other| other.path == link.path)
        {
            return Err(ArchiveError::Malformed);
        }
    }
    Ok(Unpacked {
        root: into.to_path_buf(),
        links,
    })
}

/// An entry's path as its parts: UTF-8, relative, without `..`; empty and
/// `.` parts dropped.
fn entry_parts(raw: &[u8]) -> Result<Vec<String>, ArchiveError> {
    let text = std::str::from_utf8(raw).map_err(|_| ArchiveError::Unsafe)?;
    if text.starts_with('/') {
        return Err(ArchiveError::Unsafe);
    }
    let parts: Vec<String> = text
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_owned)
        .collect();
    if parts.len() > MAX_ENTRY_PARTS || parts.iter().any(|part| !entry_part(part)) {
        return Err(ArchiveError::Unsafe);
    }
    Ok(parts)
}

/// `target` read from the folder `base` (parts from a top folder: the
/// repository's, or a skill's): the parts it names, `.` and `..` applied.
/// None when it is absolute or climbs above that top.
fn resolve(base: &[&str], target: &str) -> Option<Vec<String>> {
    if target.starts_with('/') || target.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = base.iter().map(|part| (*part).to_owned()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other.to_owned()),
        }
    }
    Some(parts)
}

#[cfg(unix)]
fn make_folders(path: &Path) -> Result<(), ArchiveError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(path)
        .map_err(|_| ArchiveError::Malformed)?;
    std::fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.is_dir())
        .then_some(())
        .ok_or(ArchiveError::Malformed)
}

#[cfg(not(unix))]
fn make_folders(_: &Path) -> Result<(), ArchiveError> {
    Err(ArchiveError::Io)
}

/// The mode a file gets: `0755` when executable, otherwise `0644`.
fn file_mode(executable: bool) -> u32 {
    if executable { 0o755 } else { 0o644 }
}

/// Write exactly `size` bytes of `entry` to the new file `path`.
#[cfg(unix)]
fn write_entry(
    path: &Path,
    entry: &mut impl Read,
    size: u64,
    executable: bool,
) -> Result<(), ArchiveError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(file_mode(executable))
        .open(path)
        .map_err(|error| match error.kind() {
            // A duplicate entry, or a file where a folder was.
            std::io::ErrorKind::AlreadyExists => ArchiveError::Malformed,
            _ => ArchiveError::Io,
        })?;
    let copied =
        std::io::copy(&mut entry.take(size), &mut file).map_err(|_| ArchiveError::Malformed)?;
    if copied != size {
        return Err(ArchiveError::Malformed);
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_entry(_: &Path, _: &mut impl Read, _: u64, _: bool) -> Result<(), ArchiveError> {
    Err(ArchiveError::Io)
}

/// Why a found skill cannot be kept, whatever its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillProblem {
    /// More files, folders, bytes or folder levels than grading copies of
    /// an installed skill.
    TooLarge,
    /// A link in it names a folder, nothing, or something outside it.
    LinkLeavesFolder,
    /// Its `SKILL.md` is a link, too large, or not UTF-8 text.
    InstructionsUnreadable,
}

/// A skill found in an unpacked archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundSkill {
    /// Its folder from the repository's top, `/`-separated; empty for the
    /// top itself.
    pub path: String,
    /// `SKILL.md`'s `name`, or its folder's name.
    pub name: String,
    /// The first line of `SKILL.md`'s `description`; empty when none.
    pub description: String,
    /// Regular files and links in it.
    pub files: u32,
    /// Bytes of its regular files.
    pub bytes: u64,
    pub rubric: RubricStatus,
    /// The folder name it would get ([`folder_name`]).
    pub folder_name: Option<String>,
    pub problem: Option<SkillProblem>,
}

/// What one link holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// By path.
    pub skills: Vec<FoundSkill>,
    /// More than [`MAX_FOUND`] skills were found; the first are listed.
    pub incomplete: bool,
}

/// Why a link holds no skill to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    /// The link's folder is not a folder in the archive.
    #[error("the link's folder is not in the repository")]
    PathNotFound,
    #[error("no skill found at this link")]
    NoSkill,
}

/// The skills at `prefix` in `unpacked` (see the module docs). `repo` names
/// the top folder when the link's folder is the repository's top.
pub fn find(unpacked: &Unpacked, prefix: &str, repo: &str) -> Result<Found, FindError> {
    let base = folder_at(&unpacked.root, prefix);
    if !std::fs::symlink_metadata(&base).is_ok_and(|meta| meta.is_dir()) {
        return Err(FindError::PathNotFound);
    }
    let mut paths: Vec<String> = Vec::new();
    let mut incomplete = false;
    if is_skill_folder(unpacked, prefix) {
        paths.push(prefix.to_owned());
    } else {
        let mut level = vec![prefix.to_owned()];
        for depth in 1..=MAX_FIND_DEPTH {
            let mut next = Vec::new();
            for folder in &level {
                for name in sorted_names(&folder_at(&unpacked.root, folder)) {
                    if name == ".git" {
                        continue;
                    }
                    let path = join(folder, &name);
                    let real_folder = std::fs::symlink_metadata(folder_at(&unpacked.root, &path))
                        .is_ok_and(|meta| meta.is_dir());
                    if !real_folder {
                        continue;
                    }
                    if is_skill_folder(unpacked, &path) {
                        paths.push(path);
                    } else if depth < MAX_FIND_DEPTH {
                        next.push(path);
                    }
                }
            }
            level = next;
        }
    }
    if paths.is_empty() {
        return Err(FindError::NoSkill);
    }
    paths.sort();
    if paths.len() > MAX_FOUND {
        paths.truncate(MAX_FOUND);
        incomplete = true;
    }
    let skills = paths
        .iter()
        .map(|path| summary(unpacked, path, repo))
        .collect();
    Ok(Found { skills, incomplete })
}

fn folder_at(root: &Path, path: &str) -> PathBuf {
    if path.is_empty() {
        root.to_path_buf()
    } else {
        root.join(path)
    }
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The UTF-8 names in a folder, sorted; none when it cannot be read.
fn sorted_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Whether the folder at `path` has a `SKILL.md`: a regular file, or a link.
fn is_skill_folder(unpacked: &Unpacked, path: &str) -> bool {
    let instructions = join(path, INSTRUCTIONS_FILE);
    std::fs::symlink_metadata(folder_at(&unpacked.root, &instructions))
        .is_ok_and(|meta| meta.is_file())
        || unpacked.links.iter().any(|link| link.path == instructions)
}

/// One found skill, read and measured.
fn summary(unpacked: &Unpacked, path: &str, repo: &str) -> FoundSkill {
    let folder = folder_at(&unpacked.root, path);
    let own_name = path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(repo);
    let inside: Vec<&StagedLink> = unpacked
        .links
        .iter()
        .filter(|link| path.is_empty() || link.path.starts_with(&format!("{path}/")))
        .collect();
    let measured = measure(&folder);
    let files = measured.files + inside.len() as u64;
    let too_large = measured.unreadable
        || files + measured.folders > MAX_INSTALLED_SKILL_FILES as u64
        || measured.bytes > MAX_INSTALLED_SKILL_BYTES
        || measured.depth > MAX_SKILL_DEPTH;
    // Each link is read from its own folder inside the skill, as it will be
    // once the skill is kept under another name: a `..` that rises above
    // the skill's folder refuses it, and it must name a regular file there
    // (not a folder, not another link).
    let links_inside = inside.iter().all(|link| {
        let within = if path.is_empty() {
            link.path.as_str()
        } else {
            &link.path[path.len() + 1..]
        };
        let parts: Vec<&str> = within.split('/').collect();
        resolve(&parts[..parts.len() - 1], &link.target).is_some_and(|named| {
            !named.is_empty()
                && std::fs::symlink_metadata(folder.join(named.join("/")))
                    .is_ok_and(|meta| meta.is_file())
        })
    });
    let instructions =
        regular_file::read_capped(&folder.join(INSTRUCTIONS_FILE), MAX_INSTRUCTIONS_BYTES)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
    let (name, description) = instructions
        .as_deref()
        .map(front_matter)
        .unwrap_or((None, None));
    let rubric_link = inside
        .iter()
        .any(|link| link.path == join(path, RUBRIC_FILE));
    let rubric = if rubric_link {
        RubricStatus::Problem(RubricError::NotRegularFile)
    } else {
        match rubric::read(&folder) {
            Ok(rubric) => RubricStatus::Ready(rubric),
            Err(RubricError::Missing) => RubricStatus::Missing,
            Err(problem) => RubricStatus::Problem(problem),
        }
    };
    let problem = if too_large {
        Some(SkillProblem::TooLarge)
    } else if !links_inside {
        Some(SkillProblem::LinkLeavesFolder)
    } else if instructions.is_none() {
        Some(SkillProblem::InstructionsUnreadable)
    } else {
        None
    };
    FoundSkill {
        path: path.to_owned(),
        folder_name: folder_name(name.as_deref(), own_name),
        name: name.unwrap_or_else(|| clip(own_name, crate::installed::MAX_NAME_CHARS)),
        description: description.unwrap_or_default(),
        files: u32::try_from(files).unwrap_or(u32::MAX),
        bytes: measured.bytes,
        rubric,
        problem,
    }
}

fn clip(text: &str, cap: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(cap)
        .collect()
}

/// What a folder holds.
#[derive(Default)]
struct Measured {
    files: u64,
    folders: u64,
    bytes: u64,
    depth: usize,
    /// Something in it could not be read, or is neither a folder nor a
    /// regular file.
    unreadable: bool,
}

fn measure(folder: &Path) -> Measured {
    let mut out = Measured::default();
    let mut pending = vec![(folder.to_path_buf(), 0_usize)];
    while let Some((at, depth)) = pending.pop() {
        out.depth = out.depth.max(depth);
        let Ok(entries) = std::fs::read_dir(&at) else {
            out.unreadable = true;
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                out.unreadable = true;
                continue;
            };
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => {
                    out.folders += 1;
                    pending.push((entry.path(), depth + 1));
                }
                Ok(meta) if meta.is_file() => {
                    out.files += 1;
                    out.bytes = out.bytes.saturating_add(meta.len());
                }
                _ => out.unreadable = true,
            }
        }
    }
    out
}

/// The folder name a kept skill gets: its `SKILL.md` name when that is a
/// skill name (lowercase letters, digits and single `-` between them, at
/// most [`MAX_SKILL_NAME_CHARS`]); otherwise its own folder's name (the
/// repository's for its top) when that is a plain name (not hidden, no
/// `/`, `\` or control character, at most [`MAX_FOLDER_NAME_BYTES`]
/// bytes); otherwise none.
pub fn folder_name(skill_name: Option<&str>, own_name: &str) -> Option<String> {
    let skill_like = |name: &str| {
        (1..=MAX_SKILL_NAME_CHARS).contains(&name.len())
            && name.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
    };
    if let Some(name) = skill_name.filter(|name| skill_like(name)) {
        return Some(name.to_owned());
    }
    (own_name.len() <= MAX_FOLDER_NAME_BYTES && entry_part(own_name) && !own_name.starts_with('.'))
        .then(|| own_name.to_owned())
}

/// Whether one found skill can be kept, or why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepState {
    Keepable,
    /// `~/.agents/skills/<name>` is taken (this entry).
    NameInAgents(PathBuf),
    /// `~/.claude/skills/<name>` is taken (this entry).
    NameInClaude(PathBuf),
    /// A skill listed before it (this path) would get the same name.
    SameNameAbove(String),
    /// It has no usable folder name ([`folder_name`]).
    UnusableName,
    Problem(SkillProblem),
}

/// The one rule for whether each found skill can be kept now (see the
/// module docs), in `skills`' order. Names are compared ignoring case, as
/// macOS's file system does.
pub fn keep_states(skills: &[FoundSkill], home: &Path) -> Vec<KeepState> {
    let agents = home.join(AGENTS_SKILLS);
    let claude = home.join(CLAUDE_SKILLS);
    let mut taken: Vec<(String, String)> = Vec::new();
    skills
        .iter()
        .map(|skill| {
            if let Some(problem) = skill.problem {
                return KeepState::Problem(problem);
            }
            let Some(name) = &skill.folder_name else {
                return KeepState::UnusableName;
            };
            let state = if exists(&agents.join(name)) {
                KeepState::NameInAgents(agents.join(name))
            } else if exists(&claude.join(name)) {
                KeepState::NameInClaude(claude.join(name))
            } else if let Some((_, path)) = taken
                .iter()
                .find(|(other, _)| other.to_lowercase() == name.to_lowercase())
            {
                KeepState::SameNameAbove(path.clone())
            } else {
                KeepState::Keepable
            };
            taken.push((name.clone(), skill.path.clone()));
            state
        })
        .collect()
}

/// Whether anything (even a broken link) is at `path`.
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// What linking `~/.claude/skills/<name>` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeLink {
    /// This link now names the kept folder.
    Linked(PathBuf),
    /// Something already has that name there; nothing was changed.
    NameTaken,
    /// There is no `~/.claude` folder (Claude Code is not set up).
    NoClaudeFolder,
    /// `~/.claude/skills` is not a folder, or the link could not be made.
    Failed,
}

/// What keeping one skill did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// It is at `folder` now.
    Kept { folder: PathBuf, claude: ClaudeLink },
    /// `~/.agents/skills/<name>` was taken meanwhile; nothing was kept.
    NameTaken,
    /// It could not be copied or put in place, or [`keep_unless`] was told
    /// to stop before the name was claimed; nothing was kept.
    Failed,
}

/// Why nothing could be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeepError {
    /// `~/.agents/skills` is not a real folder and cannot be made one.
    #[error("~/.agents/skills is not a folder XTrace can use")]
    AgentsFolder,
}

/// `~/.agents/skills`, made when missing: a real folder (not a link).
#[cfg(unix)]
pub fn agents_folder(home: &Path) -> Result<PathBuf, KeepError> {
    use std::os::unix::fs::DirBuilderExt;
    let folder = home.join(AGENTS_SKILLS);
    if std::fs::symlink_metadata(&folder).is_err() {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&folder);
    }
    std::fs::symlink_metadata(&folder)
        .is_ok_and(|meta| meta.is_dir())
        .then_some(folder)
        .ok_or(KeepError::AgentsFolder)
}

#[cfg(not(unix))]
pub fn agents_folder(_: &Path) -> Result<PathBuf, KeepError> {
    Err(KeepError::AgentsFolder)
}

/// Keep `skill` (found in `unpacked`, keepable a moment ago by
/// [`keep_states`]) in `agents` (from [`agents_folder`]) under its folder
/// name, then link it in `~/.claude/skills` under `home` when that name is
/// free. `token` makes the hidden copy's name unique. Never replaces
/// anything: a name taken meanwhile keeps nothing.
pub fn keep(
    unpacked: &Unpacked,
    skill: &FoundSkill,
    home: &Path,
    agents: &Path,
    token: &str,
) -> Kept {
    keep_unless(unpacked, skill, home, agents, token, || false)
}

/// [`keep`], asking `stop` before copying and again just before claiming
/// the name: once it says yes, the hidden copy is removed and nothing is
/// claimed ([`Kept::Failed`]). After the claim the keep finishes, so
/// quitting never leaves an empty claimed folder behind.
#[cfg(unix)]
pub fn keep_unless(
    unpacked: &Unpacked,
    skill: &FoundSkill,
    home: &Path,
    agents: &Path,
    token: &str,
    stop: impl Fn() -> bool,
) -> Kept {
    let Some(name) = &skill.folder_name else {
        return Kept::Failed;
    };
    if stop() {
        return Kept::Failed;
    }
    let copy = agents.join(format!("{KEEP_PREFIX}{token}"));
    let source = folder_at(&unpacked.root, &skill.path);
    if copy_staged(&source, &copy, &skill.path, &unpacked.links).is_err() {
        remove_all(&copy);
        return Kept::Failed;
    }
    if stop() {
        remove_all(&copy);
        return Kept::Failed;
    }
    let destination = agents.join(name);
    // Claim the name with an empty folder, then rename the copy onto it:
    // `rename` replaces only an empty folder, so a name taken by anything
    // else, even meanwhile, is never overwritten.
    match std::fs::create_dir(&destination) {
        Ok(()) => {}
        Err(error) => {
            remove_all(&copy);
            return if error.kind() == std::io::ErrorKind::AlreadyExists {
                Kept::NameTaken
            } else {
                Kept::Failed
            };
        }
    }
    if std::fs::rename(&copy, &destination).is_err() {
        let _ = std::fs::remove_dir(&destination);
        remove_all(&copy);
        return Kept::Failed;
    }
    let claude = link_in_claude(home, name, &destination);
    Kept::Kept {
        folder: destination,
        claude,
    }
}

#[cfg(not(unix))]
pub fn keep_unless(
    _: &Unpacked,
    _: &FoundSkill,
    _: &Path,
    _: &Path,
    _: &str,
    _: impl Fn() -> bool,
) -> Kept {
    Kept::Failed
}

/// Whether `path` is a folder XTrace may put a link in: a real folder, or a
/// link that resolves to a real folder this process's user owns.
#[cfg(unix)]
fn usable_folder(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => true,
        Ok(meta) if meta.file_type().is_symlink() => std::fs::metadata(path).is_ok_and(|target| {
            target.is_dir() && target.uid() == regular_file::effective_user_id()
        }),
        _ => false,
    }
}

/// Link `~/.claude/skills/<name>` to `folder` when `~/.claude` is a folder
/// (or a link to a folder the user owns) and the name is free there;
/// `~/.claude/skills` is made when missing, and may be such a link too.
#[cfg(unix)]
fn link_in_claude(home: &Path, name: &str, folder: &Path) -> ClaudeLink {
    use std::os::unix::fs::DirBuilderExt;
    let claude = home.join(".claude");
    if !usable_folder(&claude) {
        return ClaudeLink::NoClaudeFolder;
    }
    let skills = home.join(CLAUDE_SKILLS);
    if std::fs::symlink_metadata(&skills).is_err() {
        let _ = std::fs::DirBuilder::new().mode(0o755).create(&skills);
    }
    if !usable_folder(&skills) {
        return ClaudeLink::Failed;
    }
    let link = skills.join(name);
    match std::os::unix::fs::symlink(folder, &link) {
        Ok(()) => ClaudeLink::Linked(link),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => ClaudeLink::NameTaken,
        Err(_) => ClaudeLink::Failed,
    }
}

/// Copy the staged skill folder `source` (at `path` in the repository) to
/// the new folder `destination`: folders `0755`, files with their staged
/// mode, and the staged links under `path` made as links again.
#[cfg(unix)]
fn copy_staged(
    source: &Path,
    destination: &Path,
    path: &str,
    links: &[StagedLink],
) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    std::fs::DirBuilder::new().mode(0o700).create(destination)?;
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(source.join(&relative))? {
            let entry = entry?;
            let inner = relative.join(entry.file_name());
            let meta = entry.metadata()?;
            if meta.is_dir() {
                std::fs::DirBuilder::new()
                    .mode(0o755)
                    .create(destination.join(&inner))?;
                pending.push(inner);
            } else if meta.is_file() {
                let mut from = std::fs::File::open(entry.path())?;
                let mut to = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(file_mode(meta.permissions().mode() & 0o111 != 0))
                    .open(destination.join(&inner))?;
                std::io::copy(&mut from, &mut to)?;
            } else {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        }
    }
    for link in links {
        let relative = if path.is_empty() {
            Some(link.path.as_str())
        } else {
            link.path.strip_prefix(&format!("{path}/"))
        };
        if let Some(relative) = relative {
            std::os::unix::fs::symlink(&link.target, destination.join(relative))?;
        }
    }
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755))
}

/// Remove a folder XTrace made, with everything in it; links are removed,
/// never followed.
pub fn remove_all(folder: &Path) {
    let _ = std::fs::remove_dir_all(folder);
}

/// Remove what a crash between copying and renaming left in `agents`
/// (`~/.agents/skills`): real folders whose name starts with
/// [`KEEP_PREFIX`]. Nothing else is touched. A crash between claiming a
/// name (an empty `~/.agents/skills/<name>` folder) and renaming the copy
/// onto it leaves that empty folder, which then blocks the name until it is
/// removed by hand; it is not removed here, since an empty folder there may
/// be the user's. Returns how many were removed.
pub fn remove_keeping_leftovers(home: &Path) -> usize {
    let agents = home.join(AGENTS_SKILLS);
    if !std::fs::symlink_metadata(&agents).is_ok_and(|meta| meta.is_dir()) {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(&agents) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(KEEP_PREFIX));
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if ours && folder && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Where one kept skill came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedSkill {
    /// `~/.agents/skills/<name>`, absolute.
    pub folder: String,
    /// `~/.claude/skills/<name>` when XTrace made that link.
    pub claude_link: Option<String>,
    /// `<owner>/<repo>` on GitHub.
    pub repository: String,
    /// The skill's folder in the repository; empty for its top.
    pub path: String,
    /// The branch, tag or commit the link named; none for the default
    /// branch.
    pub reference: Option<String>,
    /// The commit it was copied from.
    pub commit: String,
    /// When that commit was downloaded, in Unix milliseconds.
    pub fetched_at_ms: i64,
    /// When it was kept, in Unix milliseconds.
    pub kept_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct RecordsFile {
    layout: u32,
    skills: Vec<LinkedSkill>,
}

/// `<XTrace data>/linked-skills.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSkills {
    path: PathBuf,
}

impl LinkedSkills {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The records, oldest first; a missing, unreadable or unknown file
    /// reads as none.
    pub fn read(&self) -> Vec<LinkedSkill> {
        regular_file::read_capped(&self.path, MAX_RECORDS_BYTES)
            .ok()
            .and_then(|bytes| parse_records(&bytes))
            .unwrap_or_default()
    }

    /// Change the records under one lock: read them, apply `change`, and
    /// write them back through a temporary file of a unique name beside the
    /// file. A file that cannot be used (not this layout, not JSON, too
    /// large, not a regular file) is first moved aside to
    /// `linked-skills.json.bad` (replacing an older one), never overwritten;
    /// when it cannot be moved, nothing is written. A missing file is none.
    pub fn update<T>(&self, change: impl FnOnce(&mut Vec<LinkedSkill>) -> T) -> std::io::Result<T> {
        let _held = RECORDS_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut skills = match regular_file::read_capped(&self.path, MAX_RECORDS_BYTES) {
            Err(regular_file::RegularFileError::Missing) => Vec::new(),
            Ok(bytes) => match parse_records(&bytes) {
                Some(skills) => skills,
                None => {
                    self.set_aside()?;
                    Vec::new()
                }
            },
            Err(_) => {
                self.set_aside()?;
                Vec::new()
            }
        };
        let answer = change(&mut skills);
        self.write(&skills)?;
        Ok(answer)
    }

    /// `linked-skills.json.bad`, where an unusable file is moved.
    pub fn set_aside_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".bad");
        PathBuf::from(name)
    }

    fn set_aside(&self) -> std::io::Result<()> {
        std::fs::rename(&self.path, self.set_aside_path())
    }

    /// Replace the file with `skills`, through a private temporary file of
    /// a unique name beside it. Only [`Self::update`] calls this, under its
    /// lock.
    #[cfg(unix)]
    fn write(&self, skills: &[LinkedSkill]) -> std::io::Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let folder = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(folder)?;
        let bytes = serde_json::to_vec_pretty(&RecordsFile {
            layout: RECORDS_LAYOUT,
            skills: skills.to_vec(),
        })
        .map_err(std::io::Error::other)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let temp = folder.join(format!(
            ".{LINKED_SKILLS_FILE}.{}.{}.{nanos}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written
    }

    #[cfg(not(unix))]
    fn write(&self, _: &[LinkedSkill]) -> std::io::Result<()> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// One lock for every change to a records file in this process (XTrace runs
/// one instance at a time).
static RECORDS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The records in a file's bytes; none when they are not this layout.
fn parse_records(bytes: &[u8]) -> Option<Vec<LinkedSkill>> {
    serde_json::from_slice::<RecordsFile>(bytes)
        .ok()
        .filter(|file| file.layout == RECORDS_LAYOUT)
        .map(|file| file.skills)
}

/// Add `skill` to `skills`, replacing a record of the same folder. False
/// (and nothing changed) when [`MAX_RECORDS`] other folders are recorded.
pub fn recorded(skills: &mut Vec<LinkedSkill>, skill: LinkedSkill) -> bool {
    skills.retain(|kept| kept.folder != skill.folder);
    if skills.len() >= MAX_RECORDS {
        return false;
    }
    skills.push(skill);
    true
}

#[cfg(test)]
#[path = "skill_link_tests.rs"]
mod tests;
