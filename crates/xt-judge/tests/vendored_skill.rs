//! The bundled claude-md-doctor is the pinned upstream skill, byte for
//! byte, plus XTrace's rubric and nothing else.
//!
//! Every file is compared with the Git blob ID `pin.json` records for the
//! pinned upstream commit (`git hash-object`: SHA-1 of `blob <len>\0` and
//! the bytes). Entries a staged copy skips (hidden entries and Python's
//! `__pycache__`, [`xt_judge::staging`]) are skipped here too.

use sha1::{Digest, Sha1};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn vendor() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/claude-md-doctor"
    ))
}

fn blob_id(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(&bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Every regular file under `root`, relative, `/`-separated.
fn files(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&relative)).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if name.starts_with('.') || name == "__pycache__" {
                continue;
            }
            let path = relative.join(&name);
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "{} is a link", path.display());
            if kind.is_dir() {
                pending.push(path);
            } else {
                found.insert(path.to_str().unwrap().replace('\\', "/"));
            }
        }
    }
    found
}

#[test]
fn the_vendored_skill_is_the_pinned_upstream_plus_the_rubric() {
    let pin: serde_json::Value =
        serde_json::from_slice(&std::fs::read(vendor().join("pin.json")).unwrap()).unwrap();
    let upstream = pin["upstream_files"].as_object().unwrap();
    let added: BTreeSet<String> = pin["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        added,
        BTreeSet::from([xt_judge::rubric::RUBRIC_FILE.to_owned()])
    );
    assert!(pin.get("patched").is_none(), "no upstream file is patched");
    let skill = vendor().join("skill");
    let expected: BTreeSet<String> = upstream.keys().cloned().chain(added).collect();
    assert_eq!(files(&skill), expected);
    for (relative, blob) in upstream {
        assert_eq!(
            blob_id(&skill.join(relative)),
            blob.as_str().unwrap(),
            "{relative} differs from the pinned upstream file"
        );
    }
    assert_eq!(
        blob_id(&vendor().join("LICENSE")),
        pin["license_blob"].as_str().unwrap()
    );
    // Beside the skill: only the license, this pin and the README.
    let beside: BTreeSet<String> = std::fs::read_dir(vendor())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    assert_eq!(
        beside,
        BTreeSet::from(["LICENSE", "README.md", "pin.json", "skill"].map(str::to_owned))
    );
}

#[test]
fn blob_ids_are_git_blob_ids() {
    // `printf 'hello\n' | git hash-object --stdin`
    let dir = tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let path = dir.path().join("hello");
    std::fs::write(&path, "hello\n").unwrap();
    assert_eq!(blob_id(&path), "ce013625030ba8dba906f756967f9e9ca394464a");
}
