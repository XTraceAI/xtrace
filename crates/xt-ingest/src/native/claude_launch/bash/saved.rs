//! What both `Bash` launch readers need to save a read of a transcript that
//! held no launch, and to tell later that a saved read still describes the
//! file ([`xt_store::transcript_read`]).
//!
//! A saved read is used only when everything it was read with is still the
//! same: the reader's version and the source of its rules, its longest
//! line, the Claude launchers installed (a command naming a launcher reads
//! differently once it is installed), the session owning the file, its device and inode, and a
//! fingerprint of the bytes read. Anything else reads the file as if nothing
//! was saved.

use super::super::source::Budget;
use sha2::Digest;
use std::{fs::File, os::unix::fs::FileExt, path::PathBuf, sync::OnceLock};

/// The literal-prompt match's reading rules. Raise it whenever what a read of
/// the same bytes finds may change (the word reader, the option map, what a
/// line must hold to be read), so every saved read is read again once.
pub(super) const PROMPT_READ_VERSION: u32 = 1;
/// The same for the reader that compares printed answers.
pub(super) const BASH_READ_VERSION: u32 = 1;

/// Bytes at each end of what was read that the fingerprint covers.
const WINDOW: u64 = 4096;

/// The source of everything that decides what a read of a transcript finds,
/// for both readers: a change to any of it reads every saved file again once,
/// whether or not a version above was raised.
const RULES: &[&str] = &[
    include_str!("mod.rs"),
    include_str!("parent.rs"),
    include_str!("script.rs"),
    include_str!("prompt/mod.rs"),
    include_str!("prompt/words.rs"),
    include_str!("../options.rs"),
    include_str!("../command.rs"),
    include_str!("../shell.rs"),
    include_str!("../rows.rs"),
    include_str!("../source.rs"),
];

/// A digest of [`RULES`], worked out once per process.
fn rules_source() -> [u8; 32] {
    static SOURCE: OnceLock<[u8; 32]> = OnceLock::new();
    *SOURCE.get_or_init(|| {
        let mut digest = sha2::Sha256::new();
        for rules in RULES {
            digest.update((rules.len() as u64).to_le_bytes());
            digest.update(rules.as_bytes());
        }
        digest.finalize().into()
    })
}

/// A digest of the readers' rules ([`RULES`]) and of the Claude launcher
/// programs `programs`, in order: those a read was made with.
pub(super) fn rules_digest(programs: &[PathBuf]) -> [u8; 32] {
    let mut digest = sha2::Sha256::new();
    digest.update(rules_source());
    for program in programs {
        let bytes = program.as_os_str().as_encoded_bytes();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    digest.finalize().into()
}

/// What [`fingerprint`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fingerprint {
    Digest([u8; 32]),
    /// The bytes could not be read.
    Unreadable,
    /// What is left of the pass's bytes cannot cover them, after other reads
    /// this pass: nothing was read, and a later pass may.
    Deferred,
}

/// A digest of `read_to` and the first and last [`WINDOW`] bytes of `file`
/// before it: a file rewritten in place under the same name, device and inode
/// differs. Its bytes are asked of `budget` first, all at once, and charged
/// to it.
pub(super) fn fingerprint(file: &File, read_to: u64, budget: &mut Budget<'_>) -> Fingerprint {
    let head = read_to.min(WINDOW);
    let tail = read_to.saturating_sub(WINDOW);
    let windows = [(0, head), (tail, read_to)];
    let wanted: u64 = windows.iter().map(|(from, to)| to - from).sum();
    if !budget.fits(wanted) {
        // A pass whose whole share is smaller never could: the file is read
        // as if nothing was saved, within its share.
        return if budget.spent > 0 {
            Fingerprint::Deferred
        } else {
            Fingerprint::Unreadable
        };
    }
    let mut digest = sha2::Sha256::new();
    digest.update(read_to.to_le_bytes());
    for (from, to) in windows {
        let Ok(length) = usize::try_from(to - from) else {
            return Fingerprint::Unreadable;
        };
        let mut bytes = vec![0_u8; length];
        let read = file.read_exact_at(&mut bytes, from);
        // What was asked for is charged, read whole or not.
        budget.charge(to - from);
        #[cfg(test)]
        super::super::source::READ.with(|total| total.set(total.get() + (to - from)));
        if read.is_err() {
            return Fingerprint::Unreadable;
        }
        digest.update(&bytes);
    }
    Fingerprint::Digest(digest.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, time::Duration};

    /// [`fingerprint`] within a budget that covers it.
    fn digest(file: &File, read_to: u64) -> Option<[u8; 32]> {
        match fingerprint(
            file,
            read_to,
            &mut Budget::new(1 << 20, Duration::from_secs(5), None),
        ) {
            Fingerprint::Digest(digest) => Some(digest),
            Fingerprint::Unreadable => None,
            Fingerprint::Deferred => panic!("the budget covers it"),
        }
    }

    #[test]
    fn fingerprint_tells_rewritten_bytes_and_lengths_apart() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("t.jsonl");
        let mut long = vec![b'a'; 10_000];
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&long)
            .unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let whole = digest(&file, 10_000).unwrap();
        assert_eq!(digest(&file, 10_000), Some(whole));
        assert_ne!(digest(&file, 9_999), Some(whole));
        // A change in the first or the last window before the point.
        for at in [0, 9_999] {
            long[at] = b'b';
            std::fs::write(&path, &long).unwrap();
            assert_ne!(digest(&file, 10_000), Some(whole), "{at}");
            long[at] = b'a';
        }
        // Beyond what was read is not covered.
        std::fs::write(&path, [long.as_slice(), b"more"].concat()).unwrap();
        assert_eq!(digest(&file, 10_000), Some(whole));
        // Bytes that cannot be read.
        assert_eq!(digest(&file, 20_000), None);
        // A short file is covered whole.
        std::fs::write(&path, b"ab\n").unwrap();
        assert!(digest(&file, 3).is_some());
        assert_ne!(digest(&file, 3), digest(&file, 2));
    }

    #[test]
    fn fingerprint_bytes_are_asked_of_the_budget_and_charged() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("t.jsonl");
        std::fs::write(&path, vec![b'a'; 10_000]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // Both windows, 8 KiB, are charged.
        let mut budget = Budget::new(1 << 20, Duration::from_secs(5), None);
        assert!(matches!(
            fingerprint(&file, 10_000, &mut budget),
            Fingerprint::Digest(_)
        ));
        assert_eq!(budget.spent, 2 * WINDOW);
        // A short file: both windows are the whole file.
        let mut budget = Budget::new(1 << 20, Duration::from_secs(5), None);
        fingerprint(&file, 100, &mut budget);
        assert_eq!(budget.spent, 200);
        // Less left than both windows: nothing is read or charged. A pass
        // whose whole share is smaller never could check it: no shortcut.
        let mut budget = Budget::new(2 * WINDOW - 1, Duration::from_secs(5), None);
        assert_eq!(
            fingerprint(&file, 10_000, &mut budget),
            Fingerprint::Unreadable
        );
        assert_eq!(budget.spent, 0);
        let mut budget = Budget::new(0, Duration::from_secs(5), None);
        assert_eq!(fingerprint(&file, 3, &mut budget), Fingerprint::Unreadable);
        // After other reads this pass: deferred to a later one.
        let mut budget = Budget::new(2 * WINDOW, Duration::from_secs(5), None);
        budget.charge(1);
        assert_eq!(
            fingerprint(&file, 10_000, &mut budget),
            Fingerprint::Deferred
        );
        assert_eq!(budget.spent, 1);
    }

    #[test]
    fn rules_digest_follows_each_installed_launcher() {
        let none = rules_digest(&[]);
        let one = rules_digest(&[PathBuf::from("/opt/homebrew/bin/claude")]);
        let two = rules_digest(&[
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
        ]);
        assert_ne!(none, one);
        assert_ne!(one, two);
        assert_eq!(
            one,
            rules_digest(&[PathBuf::from("/opt/homebrew/bin/claude")])
        );
    }
}
