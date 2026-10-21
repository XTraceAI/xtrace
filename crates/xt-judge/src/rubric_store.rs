//! Where a skill's rubric lives ([`applicable`]) and how a new one is saved
//! ([`save`]).
//!
//! A rubric is in one of two places:
//!
//! * **The skill's folder**: `xtrace-rubric.md` next to its `SKILL.md`.
//! * **XTrace's rubric store**: `<app data>/rubrics/<key>/xtrace-rubric.md`
//!   ([`RubricStore`]), keyed by the skill's stable location
//!   ([`crate::installed::Location`]): a Claude Code plugin's skill by its
//!   plugin and folder name (`plugin:<key>/<skill>`), so a plugin update
//!   that installs into a new versioned folder keeps its rubric; any other
//!   skill by its folder path. The rubric maker saves there for plugin
//!   skills, and for a skill folder XTrace cannot write.
//!
//! **One rule** decides which rubric applies to a skill ([`applicable`]):
//! the folder's own `xtrace-rubric.md` whenever the folder has an entry by
//! that name (valid or not, so a broken rubric in the folder is reported, not
//! hidden by a stored one), otherwise the stored one when the store has an
//! entry for the skill, otherwise none. Discovery ([`crate::installed`]),
//! opening an added skill and staging a run all ask it.
//!
//! Saving never overwrites a rubric silently: the new file is written to a
//! hidden temporary file in the same folder and then linked into place, which
//! fails when a rubric appeared meanwhile ([`SaveError::Exists`]). Replacing
//! a rubric (making it again) first moves the old file to
//! `xtrace-rubric.md.bak`, over an older backup.

use crate::{installed::Location, rubric::RUBRIC_FILE};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The rubric store's folder inside the app's data folder.
pub const STORE_DIR: &str = "rubrics";
/// Where a replaced rubric is moved, beside the new one.
pub const BACKUP_FILE: &str = "xtrace-rubric.md.bak";
/// Most characters of a location kept readable in its store folder's name.
const READABLE_KEY_CHARS: usize = 40;
/// Hexadecimal digits of the location's hash in its store folder's name.
const KEY_HASH_CHARS: usize = 16;

/// XTrace's own rubrics, one folder per skill location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RubricStore {
    root: PathBuf,
}

impl RubricStore {
    /// The store at `root` (`<app data>/rubrics`); nothing is read or made.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder kept for a skill at `location`; none for a folder whose
    /// path is not UTF-8.
    pub fn folder(&self, location: &Location) -> Option<PathBuf> {
        location.text().map(|text| self.root.join(store_key(&text)))
    }

    /// The rubric file kept for a skill at `location`.
    pub fn file(&self, location: &Location) -> Option<PathBuf> {
        self.folder(location).map(|folder| folder.join(RUBRIC_FILE))
    }
}

/// A store folder's name for a location's text: a readable part (lowercase
/// letters and digits, every other run of characters one `-`, at most
/// [`READABLE_KEY_CHARS`]) and a hash of the exact text, so two locations
/// never share a folder and the name is always a plain, safe file name.
pub fn store_key(location: &str) -> String {
    let mut readable = String::new();
    for c in location.chars() {
        if c.is_ascii_alphanumeric() {
            readable.push(c.to_ascii_lowercase());
        } else if !readable.ends_with('-') {
            readable.push('-');
        }
    }
    let readable: String = readable
        .trim_matches('-')
        .chars()
        .take(READABLE_KEY_CHARS)
        .collect();
    let hash: String = Sha256::digest(location.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(KEY_HASH_CHARS)
        .collect();
    let readable = readable.trim_end_matches('-');
    if readable.is_empty() {
        hash
    } else {
        format!("{readable}-{hash}")
    }
}

/// The rubric file that applies to a skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RubricFile {
    /// The file to read; it may not exist (the skill then has no rubric).
    pub path: PathBuf,
    /// Whether it is the one XTrace keeps rather than the folder's own.
    pub kept_by_xtrace: bool,
}

/// The one rule for which rubric applies to the skill in `folder`, found at
/// `location`: the folder's own `xtrace-rubric.md` when the folder has any
/// entry by that name; otherwise the store's when the store has any entry
/// for the location; otherwise the folder's (missing) one. Nothing is
/// followed: an entry that is a link still decides, and reading it is then
/// refused as not a regular file.
pub fn applicable(folder: &Path, location: &Location, store: Option<&RubricStore>) -> RubricFile {
    let own = folder.join(RUBRIC_FILE);
    let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    if !exists(&own)
        && let Some(kept) = store.and_then(|store| store.file(location))
        && exists(&kept)
    {
        return RubricFile {
            path: kept,
            kept_by_xtrace: true,
        };
    }
    RubricFile {
        path: own,
        kept_by_xtrace: false,
    }
}

/// Why a rubric was not saved. No path in messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("a rubric is already there")]
    Exists,
    #[error("the folder cannot be written")]
    NotWritable,
    #[error("the rubric could not be saved")]
    Failed,
}

fn write_error(error: &std::io::Error) -> SaveError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            SaveError::NotWritable
        }
        _ => SaveError::Failed,
    }
}

/// Save `bytes` as `<folder>/xtrace-rubric.md` with file mode `mode`.
///
/// The bytes go to a hidden temporary file in `folder`
/// (`.xtrace-rubric.md.<nonce>.tmp`, `nonce` a run ID), synced, and are then
/// hard-linked into place: the link is atomic and never replaces a file, so
/// a rubric that appeared meanwhile is [`SaveError::Exists`]. When `replace`
/// is true, an existing rubric entry is first renamed to
/// [`BACKUP_FILE`] (over an older backup); otherwise it is
/// [`SaveError::Exists`] and nothing changes. When the new file then cannot
/// be linked into place, the old one is moved back from the backup (when
/// nothing took its place meanwhile) before the error is returned. A folder
/// whose temporary file cannot be created for lack of permission (or a
/// read-only file system) is [`SaveError::NotWritable`]. The temporary file
/// is removed either way.
#[cfg(unix)]
pub fn save(
    folder: &Path,
    bytes: &[u8],
    replace: bool,
    nonce: &str,
    mode: u32,
) -> Result<(), SaveError> {
    save_linking(folder, bytes, replace, nonce, mode, |from, to| {
        std::fs::hard_link(from, to)
    })
}

/// [`save`], placing the new file with `link` (tests make it fail).
#[cfg(unix)]
fn save_linking(
    folder: &Path,
    bytes: &[u8],
    replace: bool,
    nonce: &str,
    mode: u32,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), SaveError> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if nonce.is_empty()
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(SaveError::Failed);
    }
    let target = folder.join(RUBRIC_FILE);
    let temp = folder.join(format!(".{RUBRIC_FILE}.{nonce}.tmp"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .map_err(|error| write_error(&error))?;
    let placed = (|| {
        file.write_all(bytes).map_err(|_| SaveError::Failed)?;
        file.sync_all().map_err(|_| SaveError::Failed)?;
        let backup = folder.join(BACKUP_FILE);
        let mut backed_up = false;
        if std::fs::symlink_metadata(&target).is_ok() {
            if !replace {
                return Err(SaveError::Exists);
            }
            std::fs::rename(&target, &backup).map_err(|error| write_error(&error))?;
            backed_up = true;
        }
        link(&temp, &target).map_err(|error| {
            // The replaced rubric goes back where it was, unless something
            // took its place meanwhile.
            if backed_up && std::fs::symlink_metadata(&target).is_err() {
                let _ = std::fs::rename(&backup, &target);
            }
            match error.kind() {
                std::io::ErrorKind::AlreadyExists => SaveError::Exists,
                _ => write_error(&error),
            }
        })
    })();
    drop(file);
    let _ = std::fs::remove_file(&temp);
    placed
}

#[cfg(not(unix))]
pub fn save(_: &Path, _: &[u8], _: bool, _: &str, _: u32) -> Result<(), SaveError> {
    Err(SaveError::Failed)
}

/// Make the store folder for `location` (and the store itself), owner-only,
/// and return it.
#[cfg(unix)]
pub fn make_store_folder(store: &RubricStore, location: &Location) -> Result<PathBuf, SaveError> {
    use std::os::unix::fs::DirBuilderExt;
    let folder = store.folder(location).ok_or(SaveError::Failed)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)
        .map_err(|error| write_error(&error))?;
    // A folder that is a link is not the store's own.
    if !std::fs::symlink_metadata(&folder).is_ok_and(|meta| meta.is_dir()) {
        return Err(SaveError::Failed);
    }
    Ok(folder)
}

#[cfg(not(unix))]
pub fn make_store_folder(_: &RubricStore, _: &Location) -> Result<PathBuf, SaveError> {
    Err(SaveError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_replacement_that_cannot_be_linked_puts_the_old_rubric_back() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        std::fs::write(folder.join(RUBRIC_FILE), "old").unwrap();
        std::fs::write(folder.join(BACKUP_FILE), "older").unwrap();
        let failed = save_linking(folder, b"new", true, "nonce-1", 0o644, |_, _| {
            Err(std::io::ErrorKind::Unsupported.into())
        });
        assert_eq!(failed, Err(SaveError::Failed));
        assert_eq!(
            std::fs::read_to_string(folder.join(RUBRIC_FILE)).unwrap(),
            "old"
        );
        // The temporary file is gone; only the rubric and a backup remain.
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [RUBRIC_FILE]);
        // A link that works replaces it and keeps the old one as the backup.
        save(folder, b"new", true, "nonce-2", 0o644).unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.join(RUBRIC_FILE)).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join(BACKUP_FILE)).unwrap(),
            "old"
        );
    }

    #[test]
    fn store_keys_are_plain_distinct_file_names() {
        let plugin = store_key("plugin:tools@market/lint");
        assert!(plugin.starts_with("plugin-tools-market-lint-"), "{plugin}");
        let folder = store_key("/Users/you/.claude/skills/My Skill");
        assert!(
            folder.starts_with("users-you-claude-skills-my-skill-"),
            "{folder}"
        );
        for key in [&plugin, &folder, &store_key("../../"), &store_key("")] {
            assert!(!key.is_empty());
            assert!(!key.starts_with('.') && !key.starts_with('-'));
            assert!(
                key.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{key}"
            );
            assert!(key.len() <= READABLE_KEY_CHARS + 1 + KEY_HASH_CHARS);
        }
        // Same readable part, different text: different folders.
        assert_ne!(store_key("plugin:a@b/c"), store_key("plugin:a-b/c"));
    }
}
