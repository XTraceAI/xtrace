//! Rubric-making refusals that last across restarts ([`RefusalStore`]), and
//! the one rule for whether a kept refusal still applies ([`in_force`]).
//!
//! When the rubric maker judges a skill unfit for scoring projects
//! ([`crate::maker::Written::Refused`]), XTrace keeps the latest refusal per
//! skill location in one small JSON file, `<app data>/rubric-refusals.json`:
//! the location, the job's ID, Claude's short reason (at most
//! [`crate::maker::MAX_REFUSAL_CHARS`] characters about the skill, never
//! conversation content), when it was refused, and the skill's
//! [`fingerprint`] at the time.
//!
//! **One rule** ([`in_force`]): a kept refusal applies only while the skill's
//! fingerprint now is exactly the one kept with it. A skill that was edited
//! or updated since is offered a rubric again. Making a rubric again ("Try
//! again") drops the kept refusal first ([`cleared`]).
//!
//! The file is small and bounded: at most [`MAX_REFUSALS`] entries (the
//! oldest go first), each within the [`keepable`] bounds, and at most
//! [`MAX_FILE_BYTES`] read. "Delete stored content" removes it
//! ([`RefusalStore::clear`]). A file that is missing, a link,
//! too large or not this layout reads as no refusals; the next write
//! replaces it. Writes go to a hidden temporary file beside it that is then
//! renamed into place.

use crate::{rubric::RUBRIC_FILE, rubric_store::BACKUP_FILE, skill::INSTRUCTIONS_FILE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use xt_probes::regular_file;

/// The refusals file's name inside the app's data folder.
pub const REFUSALS_FILE: &str = "rubric-refusals.json";
/// Most refusals kept; the oldest are dropped first. At the field bounds
/// below, a full file stays far under [`MAX_FILE_BYTES`] (tested).
pub const MAX_REFUSALS: usize = 200;
/// Largest refusals file read.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// The layout of the file this module writes.
const LAYOUT: u32 = 1;
/// Longest reason kept, in bytes: the maker's longest reason at four bytes
/// a character.
const MAX_REASON_BYTES: usize = crate::maker::MAX_REFUSAL_CHARS * 4;
/// Longest location kept, in bytes: a folder path on macOS (`PATH_MAX`), or
/// a plugin's skill by name.
pub const MAX_LOCATION_BYTES: usize = 1_024;
/// Longest job ID kept, in bytes (a UUID has 36).
const MAX_JOB_ID_BYTES: usize = 64;
/// Most folder entries a fingerprint lists; past this it notes that more
/// exist.
pub const MAX_FINGERPRINT_ENTRIES: usize = crate::staging::MAX_INSTALLED_SKILL_FILES;
/// Deepest folder a fingerprint lists.
const MAX_FINGERPRINT_DEPTH: usize = crate::staging::MAX_SKILL_DEPTH;

/// One kept refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The skill's location ([`crate::installed::Location::text`]).
    pub location: String,
    /// The rubric-making job that was refused.
    pub job_id: String,
    /// Claude's short reason.
    pub reason: String,
    /// When the job ended, in milliseconds since the Unix epoch.
    pub refused_at_ms: i64,
    /// The skill's [`fingerprint`] when the job started.
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    layout: u32,
    refusals: Vec<Refusal>,
}

/// The refusals file; nothing is read or made until asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefusalStore {
    path: PathBuf,
}

impl RefusalStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The kept refusals, oldest first. Anything unusable reads as none.
    pub fn read(&self) -> Vec<Refusal> {
        let Ok(bytes) = regular_file::read_capped(&self.path, MAX_FILE_BYTES) else {
            return Vec::new();
        };
        let Ok(file) = serde_json::from_slice::<File>(&bytes) else {
            return Vec::new();
        };
        if file.layout != LAYOUT {
            return Vec::new();
        }
        let mut refusals: Vec<Refusal> = Vec::new();
        for refusal in file.refusals {
            recorded(&mut refusals, refusal);
        }
        refusals
    }

    /// Remove the file ("Delete stored content"); a missing one is fine.
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Replace the file with `refusals`, through a temporary file beside it.
    pub fn write(&self, refusals: &[Refusal]) -> std::io::Result<()> {
        let folder = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(folder)?;
        let bytes = serde_json::to_vec(&File {
            layout: LAYOUT,
            refusals: refusals.to_vec(),
        })
        .map_err(std::io::Error::other)?;
        let temp = folder.join(format!(".{REFUSALS_FILE}.{}.tmp", std::process::id()));
        let written = (|| {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written
    }
}

/// Whether a refusal fits the bounds that keep the file small: a location
/// of at most [`MAX_LOCATION_BYTES`], a reason of at most the maker's
/// [`crate::maker::MAX_REFUSAL_CHARS`] characters, an ASCII job ID, a
/// 64-digit fingerprint, and no control characters.
pub fn keepable(refusal: &Refusal) -> bool {
    let plain = |text: &str| !text.chars().any(char::is_control);
    !refusal.location.is_empty()
        && refusal.location.len() <= MAX_LOCATION_BYTES
        && plain(&refusal.location)
        && refusal.reason.len() <= MAX_REASON_BYTES
        && refusal.reason.chars().count() <= crate::maker::MAX_REFUSAL_CHARS
        && plain(&refusal.reason)
        && !refusal.job_id.is_empty()
        && refusal.job_id.len() <= MAX_JOB_ID_BYTES
        && refusal
            .job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && refusal.fingerprint.len() == 64
        && refusal
            .fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

/// Keep `refusal` as its location's latest, dropping the oldest past
/// [`MAX_REFUSALS`]. One that is not [`keepable`] is not kept, and leaves
/// any earlier refusal of its location in place.
pub fn recorded(refusals: &mut Vec<Refusal>, refusal: Refusal) {
    if !keepable(&refusal) {
        return;
    }
    refusals.retain(|kept| kept.location != refusal.location);
    refusals.push(refusal);
    if refusals.len() > MAX_REFUSALS {
        let extra = refusals.len() - MAX_REFUSALS;
        refusals.drain(..extra);
    }
}

/// Drop the refusal kept for `location`; whether there was one.
pub fn cleared(refusals: &mut Vec<Refusal>, location: &str) -> bool {
    let before = refusals.len();
    refusals.retain(|kept| kept.location != location);
    refusals.len() != before
}

/// The one rule: a kept refusal applies while the skill's fingerprint now
/// (none when it cannot be read) is exactly the one kept with it.
pub fn in_force(refusal: &Refusal, now: Option<&str>) -> bool {
    !refusal.fingerprint.is_empty() && now == Some(refusal.fingerprint.as_str())
}

/// What a skill is, for telling whether it changed: a SHA-256 over its
/// `SKILL.md` (at most [`crate::skill::MAX_INSTRUCTIONS_BYTES`]) and its
/// folder listing (each entry's path below the folder, kind and, for a
/// file, size; links are listed, never followed), sorted by path, at most
/// [`MAX_FINGERPRINT_ENTRIES`] entries [`MAX_FINGERPRINT_DEPTH`] deep. The
/// folder's own `xtrace-rubric.md` and its backup are left out: they are
/// XTrace's, not the skill's. None when the folder or its `SKILL.md`
/// cannot be read. An edit that keeps every other file's size is noticed
/// only in `SKILL.md`.
pub fn fingerprint(folder: &Path) -> Option<String> {
    let instructions = regular_file::read_capped(
        &folder.join(INSTRUCTIONS_FILE),
        crate::skill::MAX_INSTRUCTIONS_BYTES,
    )
    .ok()?;
    let mut hash = Sha256::new();
    hash.update(b"xtrace-skill-fingerprint-1\0");
    hash.update((instructions.len() as u64).to_le_bytes());
    hash.update(&instructions);
    let mut listed = 0_usize;
    let complete = list(folder, "", 0, &mut hash, &mut listed)?;
    if !complete {
        hash.update(b"\0more");
    }
    Some(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Adds `folder`'s entries below `prefix` to `hash`, in path order; false
/// once a bound stopped the listing; none when `folder` cannot be read.
fn list(
    folder: &Path,
    prefix: &str,
    depth: usize,
    hash: &mut Sha256,
    listed: &mut usize,
) -> Option<bool> {
    let mut entries: Vec<(Vec<u8>, std::fs::DirEntry)> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| (entry.file_name().as_encoded_bytes().to_vec(), entry))
        .collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (name, entry) in entries {
        if depth == 0 && (name == RUBRIC_FILE.as_bytes() || name == BACKUP_FILE.as_bytes()) {
            continue;
        }
        if *listed >= MAX_FINGERPRINT_ENTRIES {
            return Some(false);
        }
        *listed += 1;
        let path = format!("{prefix}{}", String::from_utf8_lossy(&name));
        let meta = std::fs::symlink_metadata(entry.path()).ok();
        let kind = meta.as_ref().map(std::fs::Metadata::file_type);
        let (mark, size) = match kind {
            Some(kind) if kind.is_symlink() => (b'l', 0),
            Some(kind) if kind.is_dir() => (b'd', 0),
            Some(kind) if kind.is_file() => (b'f', meta.as_ref().map_or(0, |meta| meta.len())),
            Some(_) => (b'o', 0),
            None => (b'?', 0),
        };
        hash.update([0, mark]);
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(size.to_le_bytes());
        if mark == b'd' {
            if depth + 1 >= MAX_FINGERPRINT_DEPTH {
                return Some(false);
            }
            if !list(&entry.path(), &format!("{path}/"), depth + 1, hash, listed)? {
                return Some(false);
            }
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(location: &str, fingerprint: &str) -> Refusal {
        Refusal {
            location: location.into(),
            job_id: "00000000-0000-4000-8000-000000000001".into(),
            reason: "It publishes releases; it does not review a project.".into(),
            refused_at_ms: 1_784_000_000_000,
            // A short test value stands for a full 64-digit fingerprint.
            fingerprint: if fingerprint.is_empty() {
                String::new()
            } else {
                format!("{fingerprint:0>64}")
            },
        }
    }

    fn skill(root: &Path) -> PathBuf {
        let folder = root.join("skill");
        std::fs::create_dir_all(folder.join("scripts")).unwrap();
        std::fs::write(folder.join("SKILL.md"), "---\nname: s\n---\nDo it.\n").unwrap();
        std::fs::write(folder.join("scripts/run.py"), "print(1)\n").unwrap();
        folder
    }

    #[test]
    fn refusals_are_written_and_read_back_and_one_per_location() {
        let dir = tempfile::tempdir().unwrap();
        let store = RefusalStore::new(dir.path().join("data").join(REFUSALS_FILE));
        assert_eq!(store.read(), []);
        let mut kept = Vec::new();
        recorded(&mut kept, refusal("/a", "1"));
        recorded(&mut kept, refusal("/b", "2"));
        recorded(&mut kept, refusal("/a", "3"));
        store.write(&kept).unwrap();
        // A fresh store over the same file: what a restart reads.
        let again = RefusalStore::new(store.path().to_owned()).read();
        assert_eq!(again, [refusal("/b", "2"), refusal("/a", "3")]);
        assert!(cleared(&mut kept, "/b"));
        assert!(!cleared(&mut kept, "/b"));
        store.write(&kept).unwrap();
        assert_eq!(store.read(), [refusal("/a", "3")]);
        // No temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(dir.path().join("data"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [REFUSALS_FILE]);
    }

    #[test]
    fn an_unusable_file_reads_as_none_and_the_list_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REFUSALS_FILE);
        let store = RefusalStore::new(path.clone());
        for text in [
            "not json",
            "{\"layout\":2,\"refusals\":[]}",
            "{\"refusals\":[]}",
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(store.read(), [], "{text}");
        }
        std::fs::remove_file(&path).unwrap();
        let elsewhere = dir.path().join("elsewhere.json");
        store.write(&[refusal("/a", "1")]).unwrap();
        std::fs::rename(&path, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert_eq!(store.read(), [], "a link is not read");
        std::fs::remove_file(&path).unwrap();
        // Entries without a fingerprint or with an overlong reason are dropped.
        let mut long = refusal("/long", "1");
        long.reason = "x".repeat(MAX_REASON_BYTES + 1);
        store
            .write(&[refusal("/none", ""), long, refusal("/ok", "1")])
            .unwrap();
        assert_eq!(store.read(), [refusal("/ok", "1")]);
        let mut kept = Vec::new();
        for index in 0..=MAX_REFUSALS {
            recorded(&mut kept, refusal(&format!("/{index}"), "1"));
        }
        assert_eq!(kept.len(), MAX_REFUSALS);
        assert_eq!(kept[0].location, "/1", "the oldest went first");
    }

    #[test]
    fn a_full_file_at_every_bound_stays_under_the_read_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = RefusalStore::new(dir.path().join(REFUSALS_FILE));
        // The longest fields, in the characters JSON writes longest: a
        // quote is escaped to two bytes, a four-byte character is kept.
        let mut kept = Vec::new();
        for index in 0..MAX_REFUSALS {
            let tag = format!("/{index}");
            let mut refusal = refusal(
                &format!("{tag}{}", "\"".repeat(MAX_LOCATION_BYTES - tag.len())),
                "f",
            );
            refusal.reason = "😀".repeat(crate::maker::MAX_REFUSAL_CHARS);
            refusal.job_id = "a".repeat(MAX_JOB_ID_BYTES);
            refusal.refused_at_ms = i64::MIN;
            assert!(keepable(&refusal));
            recorded(&mut kept, refusal);
        }
        assert_eq!(kept.len(), MAX_REFUSALS);
        store.write(&kept).unwrap();
        let size = std::fs::metadata(store.path()).unwrap().len();
        assert!(size <= MAX_FILE_BYTES / 2, "{size} bytes");
        assert_eq!(store.read(), kept);
        // One byte over a bound is not kept.
        let mut long = kept[0].clone();
        long.location.push('x');
        assert!(!keepable(&long));
        let mut control = refusal("/c", "1");
        control.reason = "No.\u{7}".into();
        assert!(!keepable(&control));
        // Clearing removes the file; clearing again is fine.
        store.clear().unwrap();
        assert!(!store.path().exists());
        store.clear().unwrap();
        assert_eq!(store.read(), []);
    }

    #[test]
    fn a_refusal_applies_only_while_the_skill_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let folder = skill(dir.path());
        let first = fingerprint(&folder).unwrap();
        assert_eq!(first.len(), 64);
        let kept = refusal("/skill", &first);
        assert!(in_force(&kept, fingerprint(&folder).as_deref()));
        // XTrace's own rubric files do not change the skill.
        std::fs::write(folder.join(RUBRIC_FILE), "broken").unwrap();
        std::fs::write(folder.join(BACKUP_FILE), "older").unwrap();
        assert_eq!(fingerprint(&folder).unwrap(), first);
        // An edited SKILL.md, a new file, a file of another size: changed.
        std::fs::write(folder.join("SKILL.md"), "---\nname: s\n---\nDo more.\n").unwrap();
        assert!(!in_force(&kept, fingerprint(&folder).as_deref()));
        std::fs::write(folder.join("SKILL.md"), "---\nname: s\n---\nDo it.\n").unwrap();
        assert!(in_force(&kept, fingerprint(&folder).as_deref()));
        std::fs::write(folder.join("scripts/new.py"), "").unwrap();
        assert!(!in_force(&kept, fingerprint(&folder).as_deref()));
        std::fs::remove_file(folder.join("scripts/new.py")).unwrap();
        std::fs::write(folder.join("scripts/run.py"), "print(12)\n").unwrap();
        assert!(!in_force(&kept, fingerprint(&folder).as_deref()));
        // A skill that cannot be read now, or a kept refusal without a
        // fingerprint, never applies.
        assert!(!in_force(&kept, None));
        assert!(!in_force(&refusal("/skill", ""), Some("")));
        std::fs::remove_file(folder.join("SKILL.md")).unwrap();
        assert_eq!(fingerprint(&folder), None);
    }

    #[test]
    fn links_in_the_skill_are_listed_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = skill(dir.path());
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("docs")).unwrap();
        let before = fingerprint(&folder).unwrap();
        std::fs::write(outside.join("big.md"), "anything").unwrap();
        assert_eq!(fingerprint(&folder).unwrap(), before);
    }
}
