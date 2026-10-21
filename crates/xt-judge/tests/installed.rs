//! Finding the user's installed skills under a synthetic home folder: the
//! three skills folders and Claude Code plugins, the rubric status of each,
//! and what is never read. No real home folder is read.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};
use xt_judge::{
    installed::{
        self, Discovery, Location, PluginFile, PluginProblem, PluginReadFailure, RubricStatus,
        Source,
    },
    rubric::{RUBRIC_FILE, RubricError},
    skill::MAX_INSTRUCTIONS_BYTES,
};

fn home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("home-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn rubric(id: &str) -> String {
    format!(
        "---\nprotocol: 1\nid: {id}\nname: {id} grader\nversion: 1.0\n\
         description: Grades {id}.\ntarget: project\n---\n## All (100 points)\nAll of it.\n"
    )
}

/// A skill folder with a SKILL.md and, optionally, a rubric.
fn skill(folder: &Path, front: &str, rubric: Option<&str>) {
    fs::create_dir_all(folder).unwrap();
    fs::write(
        folder.join("SKILL.md"),
        format!("{front}\n# Body\nDo it.\n"),
    )
    .unwrap();
    if let Some(rubric) = rubric {
        fs::write(folder.join(RUBRIC_FILE), rubric).unwrap();
    }
}

fn front(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---")
}

/// Rows as (source, folder name, name, description, status) for comparing.
fn rows(found: &Discovery) -> Vec<(Source, String, String, String, String)> {
    found
        .skills
        .iter()
        .map(|skill| {
            (
                skill.source.clone(),
                skill
                    .folder
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                skill.name.clone(),
                skill.description.clone(),
                match &skill.rubric {
                    RubricStatus::Ready(rubric) => format!("ready:{}", rubric.id),
                    RubricStatus::Missing => "missing".to_owned(),
                    RubricStatus::Problem(problem) => format!("problem:{problem:?}"),
                },
            )
        })
        .collect()
}

fn plugins(home: &Path, installed: &str, settings: &str) {
    fs::create_dir_all(home.join(".claude/plugins")).unwrap();
    fs::write(
        home.join(".claude/plugins/installed_plugins.json"),
        installed,
    )
    .unwrap();
    fs::write(home.join(".claude/settings.json"), settings).unwrap();
}

#[test]
fn every_source_is_read_with_its_rubric_status() {
    let (_dir, home) = home();
    skill(
        &home.join(".claude/skills/doctor-copy"),
        &front("claude-md-doctor", "My own copy."),
        None,
    );
    skill(
        &home.join(".claude/skills/readme"),
        &front("readme", "Checks READMEs.\nmore"),
        Some(&rubric("readme-check")),
    );
    skill(
        &home.join(".agents/skills/broken"),
        &front("broken", "Has a bad rubric."),
        Some("no header"),
    );
    skill(
        &home.join(".agents/skills/plain"),
        "# No front matter at all",
        None,
    );
    skill(
        &home.join(".codex/skills/tests"),
        &front("tests", "Checks tests."),
        Some(&rubric("test-check")),
    );
    // Codex keeps its own skills in a hidden folder: not the user's.
    skill(
        &home.join(".codex/skills/.system/hidden"),
        &front("h", "d"),
        None,
    );
    // A folder without SKILL.md and a stray file are not skills.
    fs::create_dir_all(home.join(".claude/skills/notes")).unwrap();
    fs::write(home.join(".claude/skills/README.md"), "x").unwrap();

    let cache = home.join(".claude/plugins/cache/market");
    skill(
        &cache.join("tools/1.0/skills/lint"),
        &front("lint", "Lints."),
        Some(&rubric("lint-check")),
    );
    skill(
        &cache.join("off/1.0/skills/off"),
        &front("off", "Off."),
        None,
    );
    skill(
        &cache.join("project/1.0/skills/proj"),
        &front("proj", "P."),
        None,
    );
    skill(
        &cache.join("absent/1.0/skills/abs"),
        &front("abs", "A."),
        None,
    );
    plugins(
        &home,
        &format!(
            r#"{{"version": 2, "plugins": {{
                "tools@market": [{{"scope": "user", "installPath": "{tools}"}}],
                "off@market": [{{"scope": "user", "installPath": "{off}"}}],
                "project@market": [{{"scope": "project", "projectPath": "/work/p", "installPath": "{project}"}}],
                "absent@market": [{{"scope": "user", "installPath": "{absent}"}}]
            }}}}"#,
            tools = cache.join("tools/1.0").display(),
            off = cache.join("off/1.0").display(),
            project = cache.join("project/1.0").display(),
            absent = cache.join("absent/1.0").display(),
        ),
        r#"{"model": "private", "permissions": {"allow": ["Bash"]},
            "enabledPlugins": {"tools@market": true, "off@market": false, "project@market": true}}"#,
    );

    let found = installed::discover(&home);
    assert_eq!(
        rows(&found),
        [
            (
                Source::Claude,
                "doctor-copy".into(),
                "claude-md-doctor".into(),
                "My own copy.".into(),
                "missing".into()
            ),
            (
                Source::Claude,
                "readme".into(),
                "readme".into(),
                "Checks READMEs.".into(),
                "ready:readme-check".into()
            ),
            (
                Source::Agents,
                "broken".into(),
                "broken".into(),
                "Has a bad rubric.".into(),
                "problem:NoHeader".into()
            ),
            (
                Source::Agents,
                "plain".into(),
                "plain".into(),
                String::new(),
                "missing".into()
            ),
            (
                Source::Codex,
                "tests".into(),
                "tests".into(),
                "Checks tests.".into(),
                "ready:test-check".into()
            ),
            (
                Source::Plugin("tools".into()),
                "lint".into(),
                "lint".into(),
                "Lints.".into(),
                "ready:lint-check".into()
            ),
        ]
    );
    assert_eq!((found.skipped, found.incomplete), (0, false));
    assert_eq!(found.plugins_unread, None);
    assert_eq!(found.skills[1].folder, home.join(".claude/skills/readme"));
    assert_eq!(
        found.skills[1].location,
        Location::Folder(home.join(".claude/skills/readme"))
    );
    assert_eq!(
        found.skills[5].location,
        Location::Plugin {
            key: "tools@market".into(),
            skill: "lint".into()
        }
    );
    let RubricStatus::Ready(ready) = &found.skills[1].rubric else {
        panic!("ready");
    };
    assert_eq!(ready.name, "readme-check grader");
}

#[test]
fn links_are_never_followed_out_of_a_folder() {
    let (_dir, home) = home();
    let (_elsewhere_dir, elsewhere) = self::home();
    skill(
        &elsewhere.join("away"),
        &front("away", "Away."),
        Some(&rubric("away")),
    );
    skill(
        &home.join(".agents/skills/real"),
        &front("real", "Real."),
        None,
    );
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    // A skill folder that is a link out of the skills folders read.
    symlink(elsewhere.join("away"), home.join(".claude/skills/away")).unwrap();
    // One to another skills folder read here is that skill, listed once.
    symlink(
        home.join(".agents/skills/real"),
        home.join(".claude/skills/real"),
    )
    .unwrap();
    // A SKILL.md that is a link.
    fs::create_dir_all(home.join(".claude/skills/linked-md")).unwrap();
    symlink(
        elsewhere.join("away/SKILL.md"),
        home.join(".claude/skills/linked-md/SKILL.md"),
    )
    .unwrap();
    // A skills folder that is itself a link.
    fs::create_dir_all(home.join(".codex")).unwrap();
    symlink(&elsewhere, home.join(".codex/skills")).unwrap();
    // A rubric that is a link is the rubric's own problem.
    skill(
        &home.join(".agents/skills/linked-rubric"),
        &front("lr", "L."),
        None,
    );
    symlink(
        elsewhere.join("away").join(RUBRIC_FILE),
        home.join(".agents/skills/linked-rubric").join(RUBRIC_FILE),
    )
    .unwrap();
    // A plugin installed outside the plugins folder.
    skill(&elsewhere.join("plug/skills/p"), &front("p", "P."), None);
    plugins(
        &home,
        &format!(
            r#"{{"plugins": {{"out@m": [{{"scope": "user", "installPath": "{}"}}]}}}}"#,
            elsewhere.join("plug").display()
        ),
        r#"{"enabledPlugins": {"out@m": true}}"#,
    );

    let found = installed::discover(&home);
    let names: Vec<_> = rows(&found)
        .into_iter()
        .map(|(_, folder, _, _, status)| (folder, status))
        .collect();
    assert_eq!(
        names,
        [
            (
                "linked-rubric".to_owned(),
                format!("problem:{:?}", RubricError::NotRegularFile)
            ),
            ("real".to_owned(), "missing".to_owned()),
        ]
    );
    // away, linked-md, the Codex folder, the plugin; not real's link.
    assert_eq!(found.skipped, 4);
}

#[test]
fn an_oversized_or_unreadable_skill_md_is_skipped() {
    let (_dir, home) = home();
    let big = home.join(".claude/skills/big");
    fs::create_dir_all(&big).unwrap();
    fs::write(
        big.join("SKILL.md"),
        vec![b'a'; MAX_INSTRUCTIONS_BYTES as usize + 1],
    )
    .unwrap();
    let binary = home.join(".claude/skills/binary");
    fs::create_dir_all(&binary).unwrap();
    fs::write(binary.join("SKILL.md"), [0xff, 0xfe, 0x00]).unwrap();
    skill(&home.join(".claude/skills/ok"), &front("ok", "Fine."), None);
    let found = installed::discover(&home);
    assert_eq!(found.skills.len(), 1);
    assert_eq!(found.skills[0].name, "ok");
    assert_eq!(found.skipped, 2);
}

#[test]
fn missing_or_unreadable_plugin_files_list_no_plugin() {
    let (_dir, home) = home();
    let install = home.join(".claude/plugins/cache/m/a/1");
    skill(&install.join("skills/s"), &front("s", "S."), None);
    let list = format!(
        r#"{{"plugins": {{"a@m": [{{"scope": "user", "installPath": "{}"}}]}}}}"#,
        install.display()
    );
    // No settings file: nothing is enabled, and that is no problem.
    plugins(&home, &list, "{}");
    fs::remove_file(home.join(".claude/settings.json")).unwrap();
    let found = installed::discover(&home);
    assert!(found.skills.is_empty());
    assert_eq!((found.incomplete, found.plugins_unread), (false, None));
    // An enabled value that is not `true` is not enabled.
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"enabledPlugins": {"a@m": "yes"}}"#,
    )
    .unwrap();
    assert_eq!(installed::discover(&home).skills.len(), 0);
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"enabledPlugins": {"a@m": true}}"#,
    )
    .unwrap();
    assert_eq!(installed::discover(&home).skills.len(), 1);
    // One plugin's installs of another shape: that plugin is not listed,
    // and the dialog says the plugin list was not all read.
    fs::write(
        home.join(".claude/plugins/installed_plugins.json"),
        r#"{"plugins": {"a@m": {"installPath": "x"}}}"#,
    )
    .unwrap();
    let found = installed::discover(&home);
    assert!(found.skills.is_empty());
    assert!(found.incomplete);
    assert_eq!(
        found.plugins_unread,
        Some(PluginProblem {
            file: PluginFile::InstalledPlugins,
            failure: PluginReadFailure::UnknownShape
        })
    );
    // An empty home has nothing and skips nothing.
    let (_empty_dir, empty) = self::home();
    assert_eq!(installed::discover(&empty), Discovery::default());
}

#[test]
fn a_skills_folder_with_too_many_entries_is_read_in_part() {
    let (_dir, home) = home();
    let folder = home.join(".agents/skills");
    fs::create_dir_all(&folder).unwrap();
    for index in 0..installed::MAX_ENTRIES_PER_FOLDER + 5 {
        fs::write(folder.join(format!("file-{index}")), "x").unwrap();
    }
    let found = installed::discover(&home);
    assert!(found.incomplete);
    assert!(found.skills.is_empty());
}

#[test]
fn a_link_into_another_skills_folder_read_here_is_listed_once_and_not_skipped() {
    let (_dir, home) = home();
    let (_elsewhere_dir, elsewhere) = self::home();
    skill(
        &home.join(".agents/skills/x"),
        &front("x", "The x skill."),
        Some(&rubric("x-check")),
    );
    skill(&elsewhere.join("y"), &front("y", "Y."), None);
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    // ~/.claude/skills/x → ~/.agents/skills/x, as a relative link too.
    symlink(home.join(".agents/skills/x"), home.join(".claude/skills/x")).unwrap();
    symlink(
        "../../.agents/skills/x",
        home.join(".claude/skills/x-again"),
    )
    .unwrap();
    // Other links stay skipped: out of the skills folders, to a file, and to
    // nothing.
    symlink(elsewhere.join("y"), home.join(".claude/skills/y")).unwrap();
    fs::write(home.join(".agents/skills/notes.md"), "n").unwrap();
    symlink(
        home.join(".agents/skills/notes.md"),
        home.join(".claude/skills/notes"),
    )
    .unwrap();
    symlink(
        home.join(".agents/skills/gone"),
        home.join(".claude/skills/gone"),
    )
    .unwrap();
    let found = installed::discover(&home);
    assert_eq!(
        rows(&found),
        [(
            Source::Agents,
            "x".into(),
            "x".into(),
            "The x skill.".into(),
            "ready:x-check".into()
        )]
    );
    assert_eq!(found.skipped, 3);
    assert!(!found.incomplete);
}

/// A plugin `tools@market` installed (user scope) at `cache/market/tools/<version>`
/// with one skill `lint`, enabled.
fn install_plugin(home: &Path, version: &str, enabled: bool) -> PathBuf {
    let install = home
        .join(".claude/plugins/cache/market/tools")
        .join(version);
    skill(
        &install.join("skills/lint"),
        &front("lint", "Lints."),
        Some(&rubric("lint-check")),
    );
    plugins(
        home,
        &format!(
            r#"{{"version": 2, "plugins": {{"tools@market": [
                {{"scope": "project", "projectPath": "/work/p", "installPath": "{project}"}},
                {{"scope": "user", "installPath": "{install}", "version": "{version}"}}]}}}}"#,
            project = home
                .join(".claude/plugins/cache/market/tools/project")
                .display(),
            install = install.display(),
        ),
        &format!(r#"{{"enabledPlugins": {{"tools@market": {enabled}}}}}"#),
    );
    install
}

#[test]
fn a_plugin_skill_is_found_again_by_its_plugin_after_an_update() {
    let (_dir, home) = home();
    let first = install_plugin(&home, "1.0.0", true);
    let found = installed::discover(&home);
    assert_eq!(found.skills.len(), 1);
    let location = found.skills[0].location.clone();
    assert_eq!(found.skills[0].folder, first.join("skills/lint"));
    let text = location.text().unwrap();
    assert_eq!(text, "plugin:tools@market/lint");
    assert_eq!(Location::parse(&text), Some(location.clone()));
    assert_eq!(location.plugin(), Some("tools"));
    assert_eq!(
        installed::locate(&home, &location),
        Some(first.join("skills/lint"))
    );
    // An update installs into a new folder and drops the old one.
    let second = install_plugin(&home, "1.1.0", true);
    fs::remove_dir_all(&first).unwrap();
    assert_eq!(
        installed::locate(&home, &location),
        Some(second.join("skills/lint"))
    );
    assert_eq!(installed::discover(&home).skills[0].location, location);
    // Turned off, or uninstalled: it is not found.
    install_plugin(&home, "1.1.0", false);
    assert_eq!(installed::locate(&home, &location), None);
    fs::write(
        home.join(".claude/plugins/installed_plugins.json"),
        r#"{"version": 2, "plugins": {}}"#,
    )
    .unwrap();
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"enabledPlugins": {"tools@market": true}}"#,
    )
    .unwrap();
    assert_eq!(installed::locate(&home, &location), None);
    // A folder location is itself; other text is not a location.
    let folder = Location::Folder(home.join(".claude/skills/a"));
    assert_eq!(
        installed::locate(&home, &folder),
        Some(home.join(".claude/skills/a"))
    );
    assert_eq!(Location::parse(&folder.text().unwrap()), Some(folder));
    for bad in [
        "relative/path",
        "plugin:",
        "plugin:/lint",
        "plugin:k/",
        "plugin:k/.hidden",
        "",
    ] {
        assert_eq!(Location::parse(bad), None, "{bad}");
    }
}

/// One change to the plugin list files.
type Change<'a> = Box<dyn Fn() + 'a>;

#[test]
fn a_plugin_list_that_cannot_be_used_lists_no_plugin_and_says_why() {
    let (_dir, home) = home();
    let (_elsewhere_dir, elsewhere) = self::home();
    let reset = || {
        let _ = fs::remove_file(home.join(".claude/settings.json"));
        let _ = fs::remove_file(home.join(".claude/plugins/installed_plugins.json"));
        install_plugin(&home, "1.0.0", true);
    };
    let problem = |file, failure| Some(PluginProblem { file, failure });
    let settings = home.join(".claude/settings.json");
    let list = home.join(".claude/plugins/installed_plugins.json");
    let cases: Vec<(&str, Change, Option<PluginProblem>)> = vec![
        (
            "settings link",
            Box::new(|| {
                fs::write(
                    elsewhere.join("settings.json"),
                    fs::read(&settings).unwrap(),
                )
                .unwrap();
                fs::remove_file(&settings).unwrap();
                symlink(elsewhere.join("settings.json"), &settings).unwrap();
            }),
            problem(PluginFile::Settings, PluginReadFailure::Link),
        ),
        (
            "list link",
            Box::new(|| {
                fs::write(elsewhere.join("list.json"), fs::read(&list).unwrap()).unwrap();
                fs::remove_file(&list).unwrap();
                symlink(elsewhere.join("list.json"), &list).unwrap();
            }),
            problem(PluginFile::InstalledPlugins, PluginReadFailure::Link),
        ),
        (
            "list unreadable",
            Box::new(|| {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&list, fs::Permissions::from_mode(0o000)).unwrap();
            }),
            problem(PluginFile::InstalledPlugins, PluginReadFailure::Unreadable),
        ),
        (
            "settings not JSON",
            Box::new(|| fs::write(&settings, "not json").unwrap()),
            problem(PluginFile::Settings, PluginReadFailure::Malformed),
        ),
        (
            "list not JSON",
            Box::new(|| fs::write(&list, "{\"plugins\": ").unwrap()),
            problem(PluginFile::InstalledPlugins, PluginReadFailure::Malformed),
        ),
        (
            "list without plugins",
            Box::new(|| fs::write(&list, r#"{"version": 2}"#).unwrap()),
            problem(
                PluginFile::InstalledPlugins,
                PluginReadFailure::UnknownShape,
            ),
        ),
        (
            "list an array",
            Box::new(|| fs::write(&list, "[]").unwrap()),
            problem(
                PluginFile::InstalledPlugins,
                PluginReadFailure::UnknownShape,
            ),
        ),
        (
            "enabled not an object",
            Box::new(|| fs::write(&settings, r#"{"enabledPlugins": ["tools@market"]}"#).unwrap()),
            problem(PluginFile::Settings, PluginReadFailure::UnknownShape),
        ),
        (
            "user install without a path",
            Box::new(|| {
                fs::write(
                    &list,
                    r#"{"plugins": {"tools@market": [{"scope": "user"}]}}"#,
                )
                .unwrap()
            }),
            problem(
                PluginFile::InstalledPlugins,
                PluginReadFailure::UnknownShape,
            ),
        ),
    ];
    for (name, change, expected) in cases {
        reset();
        assert_eq!(installed::discover(&home).skills.len(), 1, "{name}: before");
        change();
        let found = installed::discover(&home);
        assert!(found.skills.is_empty(), "{name}");
        assert!(found.incomplete, "{name}");
        assert_eq!(found.plugins_unread, expected, "{name}");
        // The stored location of the plugin's skill is not found either.
        assert_eq!(
            installed::locate(
                &home,
                &Location::Plugin {
                    key: "tools@market".into(),
                    skill: "lint".into()
                }
            ),
            None,
            "{name}"
        );
        use std::os::unix::fs::PermissionsExt;
        if fs::symlink_metadata(&list).is_ok_and(|meta| meta.is_file()) {
            fs::set_permissions(&list, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
    // The plugins folder as a link.
    let (_other_dir, other) = self::home();
    fs::create_dir_all(other.join(".claude")).unwrap();
    symlink(home.join(".claude/plugins"), other.join(".claude/plugins")).unwrap();
    let found = installed::discover(&other);
    assert_eq!(
        found.plugins_unread,
        problem(PluginFile::PluginsFolder, PluginReadFailure::Link)
    );
    assert!(found.incomplete);
}
