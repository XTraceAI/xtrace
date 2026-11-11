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

use sha2::Digest;
use std::{fs::File, os::unix::fs::FileExt, path::PathBuf};

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

/// A digest of the readers' rules ([`RULES`]) and of the Claude launcher
/// programs installed now, in order.
pub(super) fn rules_digest(programs: &[PathBuf]) -> [u8; 32] {
    let mut digest = sha2::Sha256::new();
    for rules in RULES {
        digest.update((rules.len() as u64).to_le_bytes());
        digest.update(rules.as_bytes());
    }
    for program in programs {
        let bytes = program.as_os_str().as_encoded_bytes();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    digest.finalize().into()
}

/// A digest of `read_to` and the first and last [`WINDOW`] bytes of `file`
/// before it: a file rewritten in place under the same name, device and inode
/// differs. `None` when those bytes cannot be read.
pub(super) fn fingerprint(file: &File, read_to: u64) -> Option<[u8; 32]> {
    let mut digest = sha2::Sha256::new();
    digest.update(read_to.to_le_bytes());
    let head = read_to.min(WINDOW);
    let tail = read_to.saturating_sub(WINDOW);
    for (from, to) in [(0, head), (tail, read_to)] {
        let mut bytes = vec![0_u8; usize::try_from(to - from).ok()?];
        file.read_exact_at(&mut bytes, from).ok()?;
        digest.update(&bytes);
    }
    Some(digest.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
        let whole = fingerprint(&file, 10_000).unwrap();
        assert_eq!(fingerprint(&file, 10_000), Some(whole));
        assert_ne!(fingerprint(&file, 9_999), Some(whole));
        // A change in the first or the last window before the point.
        for at in [0, 9_999] {
            long[at] = b'b';
            std::fs::write(&path, &long).unwrap();
            assert_ne!(fingerprint(&file, 10_000), Some(whole), "{at}");
            long[at] = b'a';
        }
        // Beyond what was read is not covered.
        std::fs::write(&path, [long.as_slice(), b"more"].concat()).unwrap();
        assert_eq!(fingerprint(&file, 10_000), Some(whole));
        // Bytes that cannot be read.
        assert_eq!(fingerprint(&file, 20_000), None);
        // A short file is covered whole.
        std::fs::write(&path, b"ab\n").unwrap();
        assert!(fingerprint(&file, 3).is_some());
        assert_ne!(fingerprint(&file, 3), fingerprint(&file, 2));
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
