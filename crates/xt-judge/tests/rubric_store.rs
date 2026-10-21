//! Which rubric applies to a skill, and saving a made one, in synthetic
//! folders. No real home or app data folder is read.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use xt_judge::{
    RubricError,
    installed::{self, Location, RubricStatus},
    rubric::{self, RUBRIC_FILE},
    rubric_store::{self, BACKUP_FILE, RubricStore, SaveError},
    skill::GradingSkill,
};

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("rubric-store-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn rubric_text(id: &str) -> String {
    format!(
        "---\nprotocol: 1\nid: {id}\nname: {id} grader\nversion: 1\n\
         description: Grades {id}.\ntarget: project\n---\n## All (100 points)\nAll of it.\n"
    )
}

fn skill(folder: &Path) {
    fs::create_dir_all(folder).unwrap();
    fs::write(
        folder.join("SKILL.md"),
        "---\nname: lint\ndescription: Lints.\n---\nRun it.\n",
    )
    .unwrap();
}

const NONCE: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

#[test]
fn the_folders_own_rubric_wins_even_when_broken_then_the_kept_one() {
    let (_dir, root) = scratch();
    let folder = root.join("skills/lint");
    skill(&folder);
    let store = RubricStore::new(root.join("rubrics"));
    let location = Location::Folder(folder.clone());
    // Neither: the folder's own path, which reads as missing.
    let none = rubric_store::applicable(&folder, &location, Some(&store));
    assert_eq!(none.path, folder.join(RUBRIC_FILE));
    assert!(!none.kept_by_xtrace);
    // Kept only: the kept one.
    let kept = rubric_store::make_store_folder(&store, &location).unwrap();
    fs::write(kept.join(RUBRIC_FILE), rubric_text("kept")).unwrap();
    let applies = rubric_store::applicable(&folder, &location, Some(&store));
    assert_eq!(applies.path, kept.join(RUBRIC_FILE));
    assert!(applies.kept_by_xtrace);
    let opened = GradingSkill::open_with(&folder, Some(&applies.path)).unwrap();
    assert_eq!(opened.id(), "kept");
    assert!(!opened.rubric_in_folder());
    // Without a store, only the folder counts.
    assert!(!rubric_store::applicable(&folder, &location, None).kept_by_xtrace);
    // The folder's own, even a broken one, wins over the kept one.
    fs::write(folder.join(RUBRIC_FILE), "no header").unwrap();
    let applies = rubric_store::applicable(&folder, &location, Some(&store));
    assert_eq!(applies.path, folder.join(RUBRIC_FILE));
    assert_eq!(
        GradingSkill::open_with(&folder, Some(&applies.path)).map(|_| ()),
        Err(RubricError::NoHeader.into())
    );
}

#[test]
fn a_plugin_skill_keeps_its_rubric_across_an_update_by_its_plugin_location() {
    let (_dir, home) = scratch();
    let plugins = home.join(".claude/plugins");
    let install = |version: &str| {
        let root = plugins.join("cache/market/tools").join(version);
        skill(&root.join("skills/lint"));
        fs::write(
            plugins.join("installed_plugins.json"),
            format!(
                r#"{{"version": 2, "plugins": {{"tools@market": [{{"scope": "user", "installPath": "{}"}}]}}}}"#,
                root.display()
            ),
        )
        .unwrap();
        root.join("skills/lint")
    };
    install("1.0.0");
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"enabledPlugins": {"tools@market": true}}"#,
    )
    .unwrap();
    let data = home.join("app data");
    let store = RubricStore::new(data.join(rubric_store::STORE_DIR));
    let location = Location::Plugin {
        key: "tools@market".into(),
        skill: "lint".into(),
    };
    let status = |store: Option<&RubricStore>| {
        let found = installed::discover_with(&home, store);
        assert_eq!(found.skills.len(), 1);
        (
            found.skills[0].rubric.clone(),
            found.skills[0].rubric_kept_by_xtrace,
        )
    };
    assert_eq!(status(Some(&store)), (RubricStatus::Missing, false));
    let folder = rubric_store::make_store_folder(&store, &location).unwrap();
    rubric_store::save(
        &folder,
        rubric_text("lint-check").as_bytes(),
        false,
        NONCE,
        0o600,
    )
    .unwrap();
    let (rubric, kept) = status(Some(&store));
    assert!(kept);
    assert!(matches!(rubric, RubricStatus::Ready(ref r) if r.id == "lint-check"));
    // Without the store the skill has none; the plugin's folder is untouched.
    assert_eq!(status(None), (RubricStatus::Missing, false));
    // The plugin updates into a new versioned folder; the rubric still applies.
    let second = install("2.0.0");
    fs::remove_dir_all(plugins.join("cache/market/tools/1.0.0")).unwrap();
    let (rubric, kept) = status(Some(&store));
    assert!(kept);
    assert!(matches!(rubric, RubricStatus::Ready(ref r) if r.id == "lint-check"));
    assert!(!second.join(RUBRIC_FILE).exists());
    // The store folder's name is the plain key of the location.
    assert_eq!(
        folder.file_name().unwrap().to_str().unwrap(),
        rubric_store::store_key("plugin:tools@market/lint")
    );
}

#[test]
fn saving_never_overwrites_without_replace_and_replace_keeps_a_backup() {
    let (_dir, root) = scratch();
    let folder = root.join("skill");
    skill(&folder);
    rubric_store::save(&folder, b"first", false, NONCE, 0o644).unwrap();
    assert_eq!(fs::read(folder.join(RUBRIC_FILE)).unwrap(), b"first");
    assert_eq!(
        fs::metadata(folder.join(RUBRIC_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    // Already there: refused, nothing changes.
    assert_eq!(
        rubric_store::save(&folder, b"second", false, NONCE, 0o644),
        Err(SaveError::Exists)
    );
    assert_eq!(fs::read(folder.join(RUBRIC_FILE)).unwrap(), b"first");
    // Replace: the old one becomes the backup, over an older backup.
    fs::write(folder.join(BACKUP_FILE), "older").unwrap();
    rubric_store::save(&folder, b"second", true, NONCE, 0o644).unwrap();
    assert_eq!(fs::read(folder.join(RUBRIC_FILE)).unwrap(), b"second");
    assert_eq!(fs::read(folder.join(BACKUP_FILE)).unwrap(), b"first");
    // No temporary file is left behind.
    let names: Vec<String> = fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(names.is_empty(), "{names:?}");
    // A bad nonce writes nothing.
    assert_eq!(
        rubric_store::save(&folder, b"x", true, "../x", 0o644),
        Err(SaveError::Failed)
    );
}

#[test]
fn a_folder_that_cannot_be_written_says_so_and_stays_unchanged() {
    let (_dir, root) = scratch();
    let folder = root.join("skill");
    skill(&folder);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
    let saved = rubric_store::save(&folder, b"x", false, NONCE, 0o644);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(saved, Err(SaveError::NotWritable));
    assert!(!folder.join(RUBRIC_FILE).exists());
}

#[test]
fn an_id_is_replaced_in_the_header_only() {
    let text = rubric_text("lint").replace("All of it.", "id: not a header line");
    let (bytes, rubric) = rubric::with_id(text.as_bytes(), "lint-2").unwrap();
    assert_eq!(rubric.id, "lint-2");
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        text.replacen("id: lint\n", "id: lint-2\n", 1)
    );
    // Line endings and everything else stay byte for byte.
    let crlf = rubric_text("lint").replace('\n', "\r\n");
    let (bytes, _) = rubric::with_id(crlf.as_bytes(), "other").unwrap();
    assert_eq!(
        bytes,
        crlf.replace("id: lint\r\n", "id: other\r\n").into_bytes()
    );
    // Only valid rubrics and ids.
    assert_eq!(
        rubric::with_id(b"no header", "x"),
        Err(RubricError::NoHeader)
    );
    assert_eq!(
        rubric::with_id(rubric_text("lint").as_bytes(), "Not A Slug").map(|_| ()),
        Err(RubricError::Id)
    );
}

#[test]
fn a_version_is_replaced_in_the_header_and_read_back_loosely() {
    let text = rubric_text("lint").replace("All of it.", "version: 9 in the guidance");
    let (bytes, rubric) = rubric::with_version(text.as_bytes(), "4").unwrap();
    assert_eq!(rubric.version, "4");
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        text.replacen("version: 1\n", "version: 4\n", 1)
    );
    // A refused rubric still says its version; text without a header none.
    let refused = "---\nversion: 7 \nid: Not A Slug\n---\n";
    assert_eq!(
        rubric::header_value(refused.as_bytes(), "version").as_deref(),
        Some("7")
    );
    assert_eq!(rubric::header_value(b"version: 3\n", "version"), None);
    assert_eq!(
        rubric::header_value(b"---\nid: x\n---\nversion: 3\n", "version"),
        None
    );
    assert_eq!(
        rubric::with_version(rubric_text("lint").as_bytes(), "1 2").map(|_| ()),
        Err(RubricError::Field)
    );
}
