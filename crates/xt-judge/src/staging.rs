//! Where a run happens and what is kept of it.
//!
//! Claude Code stops matching its `Bash(...)` and `Edit(...)` allow rules for
//! a path that contains whitespace, and both the app bundle and macOS's
//! `Application Support` folder do. So a run never happens in either:
//!
//! * **Staging.** Each run gets a private (`0700`) folder of its own under
//!   the system temporary folder, `<tmp>/xtrace-grading/<run id>/`
//!   ([`Staging::create`]). The skill is copied into its `skill/` folder
//!   within [`CopyLimits`] (a built-in skill's or an installed skill's) and
//!   its `grade/` folder is the run's grade folder. Folders and regular
//!   files are copied; a link is copied as the file it names only when that
//!   is a regular file inside the skill's own folder, and any other link
//!   refuses the run ([`GradeError::SkillLinkRefused`]), as does a skill
//!   past its limits ([`GradeError::SkillTooLarge`]). A resolved staging path that contains
//!   whitespace refuses the run with the runner's own typed reason
//!   ([`GradeError::GradeDirPathHasWhitespace`]). The `xtrace-grading` folder
//!   itself must be a real folder (not a link) that this process's user owns
//!   and only its owner can open ([`private_parent`]).
//! * **Keeping.** After a valid result, only `result.json`, the validated
//!   `report` HTML file and the `.svg` files beside it (the skill's share
//!   card and badge) are copied into the permanent grade folder
//!   ([`Staging::archive`]). The `work/` folder, which holds the exam's
//!   working files including excerpts of conversations, is never copied. The
//!   permanent folder may contain whitespace: nothing runs there.
//! * **Leftovers.** The run's Claude Code process is recorded in the
//!   staging folder ([`Staging::leader_record`]); after a crash of the app,
//!   the next start stops that process group, only if the recorded process
//!   is still exactly the one that was launched
//!   ([`stop_recorded_leader`]), before it removes the folder.
//! * **Removal.** The staging folder is removed when the run is over
//!   ([`Staging::remove`], or on drop). Folders named like a run ID can be
//!   removed in bulk under either parent ([`remove_run_folders`]): leftover
//!   staging after a crash (only under a [`private_parent`]), permanent
//!   folders past their retention.
//!
//! Only Unix hosts are supported.

use crate::{
    protocol::{GradeResult, MAX_RESULT_BYTES, RESULT_FILE},
    rubric::{MAX_RUBRIC_BYTES, RUBRIC_FILE},
    runner::{GradeError, WORK_DIR},
};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use xt_probes::{
    leader::{GroupLeader, GroupStop, MAX_RECORD_BYTES},
    regular_file,
};

/// The folder under the system temporary folder that holds every run's
/// staging folder.
pub const STAGING_DIR: &str = "xtrace-grading";
/// The staged skill's folder inside a staging folder.
pub const STAGED_SKILL_DIR: &str = "skill";
/// The run's grade folder inside a staging folder.
pub const STAGED_GRADE_DIR: &str = "grade";
/// The file in a staging folder that names the run's Claude Code process
/// ([`xt_probes::leader::GroupLeader::record`]).
pub const LEADER_RECORD: &str = "leader";
/// Most files and folders a staged built-in skill may have
/// ([`CopyLimits::BUILT_IN`]).
pub const MAX_SKILL_FILES: usize = 200;
/// Most bytes a staged built-in skill may have in all.
pub const MAX_SKILL_BYTES: u64 = 16 * 1024 * 1024;
/// Most files and folders a staged installed skill may have
/// ([`CopyLimits::INSTALLED`]).
pub const MAX_INSTALLED_SKILL_FILES: usize = 2_000;
/// Most bytes a staged installed skill may have in all.
pub const MAX_INSTALLED_SKILL_BYTES: u64 = 50 * 1024 * 1024;
/// Deepest folder below a skill's own folder that is copied.
pub const MAX_SKILL_DEPTH: usize = 32;

/// How much of a skill folder a run copies before it refuses the skill
/// ([`GradeError::SkillTooLarge`]). Hidden entries and `__pycache__` are
/// never copied and do not count toward `files`; every entry a folder holds
/// counts toward that folder's read bound, which is `files` too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyLimits {
    /// Files and folders copied in all (the skill's own folder not counted),
    /// and entries read from any one folder.
    pub files: usize,
    pub bytes: u64,
    /// Folder levels below the skill's own folder.
    pub depth: usize,
}

impl CopyLimits {
    /// A skill bundled with the app.
    pub const BUILT_IN: Self = Self {
        files: MAX_SKILL_FILES,
        bytes: MAX_SKILL_BYTES,
        depth: MAX_SKILL_DEPTH,
    };
    /// A skill the user installed and added to Scores.
    pub const INSTALLED: Self = Self {
        files: MAX_INSTALLED_SKILL_FILES,
        bytes: MAX_INSTALLED_SKILL_BYTES,
        depth: MAX_SKILL_DEPTH,
    };
}
/// Largest report file kept.
pub const MAX_REPORT_BYTES: u64 = 16 * 1024 * 1024;
/// The extension of the files beside the report that are kept with it.
pub const CARD_EXTENSION: &str = "svg";
/// Most files beside the report that are kept with it.
pub const MAX_CARDS: usize = 8;
/// Largest file beside the report that is kept with it.
pub const MAX_CARD_BYTES: u64 = 1024 * 1024;

/// One run's staging folder. Removed when dropped, unless [`Self::remove`]
/// already did.
#[derive(Debug)]
pub struct Staging {
    root: PathBuf,
    removed: bool,
}

/// Whether `name` is a run ID: a lowercase UUID (`8-4-4-4-12` hexadecimal
/// digits). Only folders named this way are ever created or removed here.
pub fn is_run_id(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
}

/// The folder that holds every run's staging folder.
pub fn staging_parent(temp_dir: &Path) -> PathBuf {
    temp_dir.join(STAGING_DIR)
}

/// [`staging_parent`], resolved, when it may be trusted: a real folder (not
/// a link) that this process's user owns and only its owner can open.
/// `None` otherwise, or when it is missing. Both creating a staging folder
/// and removing leftover ones check this first.
#[cfg(unix)]
pub fn private_parent(temp_dir: &Path) -> Option<PathBuf> {
    let parent = staging_parent(temp_dir);
    is_private_folder(&parent)
        .then(|| std::fs::canonicalize(&parent).ok())
        .flatten()
}

#[cfg(not(unix))]
pub fn private_parent(_: &Path) -> Option<PathBuf> {
    None
}

/// Whether `path` is a real folder (not a link) that this process's user
/// owns and only its owner can open.
#[cfg(unix)]
pub(crate) fn is_private_folder(path: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.is_dir()
            && meta.permissions().mode() & 0o077 == 0
            && meta.uid() == regular_file::effective_user_id()
    })
}

#[cfg(not(unix))]
pub(crate) fn is_private_folder(_: &Path) -> bool {
    false
}

/// Make `folder` owner-only if it is missing, then return it resolved when
/// it is a [`is_private_folder`] whose path has no whitespace.
#[cfg(unix)]
pub(crate) fn make_private_folder(folder: &Path) -> Result<PathBuf, GradeError> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(GradeError::StagingFailed),
    }
    if !is_private_folder(folder) {
        return Err(GradeError::StagingFailed);
    }
    let resolved = std::fs::canonicalize(folder).map_err(|_| GradeError::StagingFailed)?;
    let text = resolved.to_str().ok_or(GradeError::PathNotUtf8)?;
    if text.chars().any(char::is_whitespace) {
        return Err(GradeError::GradeDirPathHasWhitespace);
    }
    Ok(resolved)
}

#[cfg(not(unix))]
pub(crate) fn make_private_folder(_: &Path) -> Result<PathBuf, GradeError> {
    Err(GradeError::UnsupportedPlatform)
}

/// [`staging_parent`], made if missing, when it is a [`private_parent`]
/// whose path has no whitespace.
pub(crate) fn make_private_parent(temp_dir: &Path) -> Result<PathBuf, GradeError> {
    make_private_folder(&staging_parent(temp_dir))
}

impl Staging {
    /// [`Self::create_with`] a built-in skill's [`CopyLimits::BUILT_IN`].
    pub fn create(temp_dir: &Path, run_id: &str, skill_dir: &Path) -> Result<Self, GradeError> {
        Self::create_with(temp_dir, run_id, skill_dir, CopyLimits::BUILT_IN)
    }

    /// Create `<temp_dir>/xtrace-grading/<run_id>/` with a copy of the skill
    /// at `skill_dir` in `skill/`, within `limits`, and an empty `grade/`.
    /// The run folder must not exist yet.
    #[cfg(unix)]
    pub fn create_with(
        temp_dir: &Path,
        run_id: &str,
        skill_dir: &Path,
        limits: CopyLimits,
    ) -> Result<Self, GradeError> {
        use std::os::unix::fs::DirBuilderExt;
        if !is_run_id(run_id) {
            return Err(GradeError::StagingFailed);
        }
        let parent = make_private_parent(temp_dir)?;
        let root = parent.join(run_id);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|_| GradeError::StagingFailed)?;
        // From here on, dropping removes what was made.
        let staging = Self {
            root,
            removed: false,
        };
        let source = std::fs::canonicalize(skill_dir).map_err(|_| GradeError::StagingFailed)?;
        copy_skill(&source, &staging.skill_dir(), limits)?;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(staging.grade_dir())
            .map_err(|_| GradeError::StagingFailed)?;
        Ok(staging)
    }

    #[cfg(not(unix))]
    pub fn create_with(_: &Path, _: &str, _: &Path, _: CopyLimits) -> Result<Self, GradeError> {
        Err(GradeError::UnsupportedPlatform)
    }

    /// Place the rubric XTrace keeps for the staged skill
    /// ([`crate::rubric_store`]) in the staged copy as its
    /// `xtrace-rubric.md`, when the copy has no entry by that name. A copy
    /// that has one (the skill's folder gained its own rubric meanwhile)
    /// keeps it, and a kept file that cannot be read as a regular file of at
    /// most [`MAX_RUBRIC_BYTES`] is not placed: either way the staged copy
    /// then opens with another rubric, or none, and the run's own check of it
    /// reports that.
    pub fn place_rubric(&self, kept: &Path) -> Result<(), GradeError> {
        let staged = self.skill_dir().join(RUBRIC_FILE);
        if std::fs::symlink_metadata(&staged).is_ok() {
            return Ok(());
        }
        let Ok(bytes) = regular_file::read_capped(kept, MAX_RUBRIC_BYTES) else {
            return Ok(());
        };
        write_new(&staged, &bytes).map_err(|_| GradeError::StagingFailed)
    }

    /// The resolved staging folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The staged copy of the skill.
    pub fn skill_dir(&self) -> PathBuf {
        self.root.join(STAGED_SKILL_DIR)
    }

    /// The run's grade folder.
    pub fn grade_dir(&self) -> PathBuf {
        self.root.join(STAGED_GRADE_DIR)
    }

    /// Where the run's Claude Code process is recorded
    /// ([`crate::runner::GradeRun::leader_record`]): beside the grade folder,
    /// so nothing the run may write can change it.
    pub fn leader_record(&self) -> PathBuf {
        self.root.join(LEADER_RECORD)
    }

    /// Copy what is kept of a validated result into `destination`, which
    /// must not exist yet (its parent is created if missing): `result.json`,
    /// the `report` file at the same relative path, and up to [`MAX_CARDS`]
    /// regular `.svg` files beside the report. Nothing under `work/` is ever
    /// read. On failure the partly written `destination` is removed.
    pub fn archive(&self, result: &GradeResult, destination: &Path) -> Result<(), GradeError> {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|_| GradeError::ArchiveFailed)?;
        }
        std::fs::create_dir(destination).map_err(|_| GradeError::ArchiveFailed)?;
        let copied = self.archive_into(result, destination);
        if copied.is_err() {
            let _ = std::fs::remove_dir_all(destination);
        }
        copied
    }

    fn archive_into(&self, result: &GradeResult, destination: &Path) -> Result<(), GradeError> {
        let grade = self.grade_dir();
        keep(
            &grade.join(RESULT_FILE),
            &destination.join(RESULT_FILE),
            MAX_RESULT_BYTES,
        )?;
        let Some(report) = &result.report else {
            return Ok(());
        };
        let relative = Path::new(report);
        // The protocol already refused a report in the work folder; this is
        // the copy's own guard, so it holds whatever validated the result.
        if relative
            .components()
            .next()
            .and_then(|first| first.as_os_str().to_str())
            .is_none_or(|first| first.eq_ignore_ascii_case(WORK_DIR))
        {
            return Err(GradeError::ArchiveFailed);
        }
        keep(
            &grade.join(relative),
            &destination.join(relative),
            MAX_REPORT_BYTES,
        )?;
        let folder = relative.parent().unwrap_or(Path::new(""));
        let mut cards: Vec<_> = std::fs::read_dir(grade.join(folder))
            .map_err(|_| GradeError::ArchiveFailed)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name())
            .filter(|name| Path::new(name).extension() == Some(OsStr::new(CARD_EXTENSION)))
            .collect();
        cards.sort();
        for name in cards.into_iter().take(MAX_CARDS) {
            let relative = folder.join(&name);
            keep(
                &grade.join(&relative),
                &destination.join(&relative),
                MAX_CARD_BYTES,
            )?;
        }
        Ok(())
    }

    /// Remove the staging folder now. Returns whether it is gone.
    pub fn remove(mut self) -> bool {
        self.removed = true;
        remove_tree(&self.root)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.removed {
            remove_tree(&self.root);
        }
    }
}

fn remove_tree(path: &Path) -> bool {
    match std::fs::remove_dir_all(path) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Copy one regular file, never through a link, at most `cap` bytes.
fn keep(source: &Path, destination: &Path, cap: u64) -> Result<(), GradeError> {
    let bytes = regular_file::read_capped(source, cap).map_err(|_| GradeError::ArchiveFailed)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|_| GradeError::ArchiveFailed)?;
    }
    write_new(destination, &bytes).map_err(|_| GradeError::ArchiveFailed)
}

/// Write a new owner-only file; one already at the path is never replaced.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_new(path, bytes)
}

/// Stop the process group a leftover staging folder `root` recorded, only
/// while its leader is still exactly the recorded process
/// ([`GroupLeader::stop_group`]): `SIGTERM`, then `SIGKILL` after `grace`.
/// A missing, unreadable or malformed record, a record a link points to,
/// or a `root` that is itself a link (or not a folder), stops nothing.
pub fn stop_recorded_leader(root: &Path, grace: std::time::Duration) -> GroupStop {
    // A run's own folder is a real folder; a link could name any record.
    if !std::fs::symlink_metadata(root).is_ok_and(|meta| meta.file_type().is_dir()) {
        return GroupStop::NotRunning;
    }
    regular_file::read_capped(&root.join(LEADER_RECORD), MAX_RECORD_BYTES as u64)
        .ok()
        .and_then(|bytes| GroupLeader::parse(&bytes))
        .map_or(GroupStop::NotRunning, |leader| leader.stop_group(grace))
}

#[cfg(unix)]
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_new(_: &Path, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Copy a skill folder within `limits`: folders and regular files, skipping
/// hidden entries and Python's `__pycache__`. A link is copied as the file it
/// names only when that resolves to a regular file inside `source` (itself
/// resolved); a link to a folder, out of `source` or to nothing refuses the
/// skill ([`GradeError::SkillLinkRefused`]). More files and folders, bytes
/// or folder levels than `limits` allow, or a folder holding more entries
/// than `limits.files` (skipped ones included), refuse it too
/// ([`GradeError::SkillTooLarge`]); any other kind of entry, or a name that
/// is not UTF-8, fails staging.
#[cfg(unix)]
pub(crate) fn copy_skill(
    source: &Path,
    destination: &Path,
    limits: CopyLimits,
) -> Result<(), GradeError> {
    copy_skill_except(source, destination, limits, None)
}

/// [`copy_skill`], leaving out the entry named `skip` directly in `source`
/// whatever kind of entry it is (it still counts toward the entries a
/// folder may hold, as a skipped hidden entry does).
#[cfg(unix)]
pub(crate) fn copy_skill_except(
    source: &Path,
    destination: &Path,
    limits: CopyLimits,
    skip: Option<&str>,
) -> Result<(), GradeError> {
    use std::os::unix::fs::DirBuilderExt;
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    let mut pending = vec![(PathBuf::new(), 0_usize)];
    while let Some((relative, depth)) = pending.pop() {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(destination.join(&relative))
            .map_err(|_| GradeError::StagingFailed)?;
        let entries =
            std::fs::read_dir(source.join(&relative)).map_err(|_| GradeError::StagingFailed)?;
        for (read, entry) in entries.enumerate() {
            if read >= limits.files {
                return Err(GradeError::SkillTooLarge);
            }
            let entry = entry.map_err(|_| GradeError::StagingFailed)?;
            let name = entry.file_name();
            let Some(text) = name.to_str() else {
                return Err(GradeError::StagingFailed);
            };
            if text.starts_with('.') || text == "__pycache__" || (depth == 0 && skip == Some(text))
            {
                continue;
            }
            let path = relative.join(&name);
            let kind = entry.file_type().map_err(|_| GradeError::StagingFailed)?;
            // Folders count toward the limit like files.
            files += 1;
            if files > limits.files {
                return Err(GradeError::SkillTooLarge);
            }
            let read_from = if kind.is_dir() {
                if depth + 1 > limits.depth {
                    return Err(GradeError::SkillTooLarge);
                }
                pending.push((path, depth + 1));
                continue;
            } else if kind.is_file() {
                source.join(&path)
            } else if kind.is_symlink() {
                linked_file(source, &source.join(&path))?
            } else {
                return Err(GradeError::StagingFailed);
            };
            let content =
                regular_file::read_capped(&read_from, limits.bytes - bytes).map_err(|error| {
                    match error {
                        regular_file::RegularFileError::TooLarge => GradeError::SkillTooLarge,
                        _ => GradeError::StagingFailed,
                    }
                })?;
            bytes += content.len() as u64;
            if bytes > limits.bytes {
                return Err(GradeError::SkillTooLarge);
            }
            write_new(&destination.join(&path), &content).map_err(|_| GradeError::StagingFailed)?;
        }
    }
    Ok(())
}

/// The resolved regular file the link at `link` names, when it is inside
/// the resolved skill folder `root`.
#[cfg(unix)]
fn linked_file(root: &Path, link: &Path) -> Result<PathBuf, GradeError> {
    let target = std::fs::canonicalize(link).map_err(|_| GradeError::SkillLinkRefused)?;
    let inside = target.starts_with(root) && target != root;
    let file = std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_file());
    if inside && file {
        Ok(target)
    } else {
        Err(GradeError::SkillLinkRefused)
    }
}

/// Remove every entry directly in `parent` whose name is a run ID that
/// `remove` selects: a folder with everything in it, a link itself (never
/// what it points to). Anything else is left alone. Returns how many were
/// removed; a missing `parent` removes nothing.
pub fn remove_run_folders(parent: &Path, remove: impl Fn(&str) -> bool) -> usize {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_run_id(name) || !remove(name) {
            continue;
        }
        let path = entry.path();
        let gone = match entry.file_type() {
            Ok(kind) if kind.is_dir() => remove_tree(&path),
            Ok(_) => std::fs::remove_file(&path).is_ok(),
            Err(_) => false,
        };
        removed += usize::from(gone);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lowercase_uuids_are_run_ids() {
        assert!(is_run_id("0f8fad5b-d9cb-469f-a165-70867728950e"));
        for bad in [
            "",
            "0F8FAD5B-D9CB-469F-A165-70867728950E",
            "0f8fad5b-d9cb-469f-a165-70867728950",
            "0f8fad5bxd9cb-469f-a165-70867728950e",
            "../8fad5b-d9cb-469f-a165-70867728950e",
            "0f8fad5b-d9cb-469f-a165-70867728950g",
        ] {
            assert!(!is_run_id(bad), "{bad}");
        }
    }
}
