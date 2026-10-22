//! Every rule over a synthetic home. The home is made under this workspace's
//! own `target/tmp`, never the system temporary folder (which the temporary
//! rule skips) and never the real home folder.
use super::*;
use std::fs;
use tempfile::TempDir;

/// A synthetic home under the workspace's `target/tmp`.
fn home() -> TempDir {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&parent).unwrap();
    let home = tempfile::Builder::new()
        .prefix("saved-projects-")
        .tempdir_in(parent)
        .unwrap();
    // The rules are stated for the real path; resolve the test's own spelling
    // of it so assertions can compare paths directly.
    assert!(fs::canonicalize(home.path()).is_ok());
    home
}

fn real(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

fn mkdir(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    real(path)
}

fn write(home: &Path, relative: &str, contents: &str) {
    let path = home.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Give `folder` a `.git` entry of its own: a folder, or a file as a linked
/// Git worktree has.
fn git_entry(folder: &Path, as_file: bool) {
    let entry = folder.join(".git");
    if as_file {
        fs::write(entry, "gitdir: /elsewhere\n").unwrap();
    } else {
        fs::create_dir_all(entry).unwrap();
    }
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn claude_code(home: &Path, folders: &[&Path]) {
    let projects: serde_json::Map<String, serde_json::Value> = folders
        .iter()
        .map(|folder| (text(folder), serde_json::json!({"allowedTools": []})))
        .collect();
    write(
        home,
        CLAUDE_CODE_FILE,
        &serde_json::json!({"numStartups": 3, "projects": projects}).to_string(),
    );
}

fn claude_app(home: &Path, presets: &[(&str, &Path)]) {
    let presets: Vec<_> = presets
        .iter()
        .map(|(name, cwd)| {
            serde_json::json!({
                "id": "p", "name": name, "createdAt": 1,
                "target": {"kind": "local", "cwd": text(cwd)}, "settings": {}
            })
        })
        .collect();
    write(
        home,
        CLAUDE_APP_FILE,
        &serde_json::json!({"byAccount": {"acct": {"presets": presets, "builtIns": []}}})
            .to_string(),
    );
}

fn codex_app(home: &Path, projects: &[(&str, &[&Path])]) {
    let projects: serde_json::Map<String, serde_json::Value> = projects
        .iter()
        .enumerate()
        .map(|(index, (name, roots))| {
            let roots: Vec<String> = roots.iter().map(|root| text(root)).collect();
            (
                format!("id-{index}"),
                serde_json::json!({"id": index, "name": name, "rootPaths": roots}),
            )
        })
        .collect();
    write(
        home,
        CODEX_APP_FILE,
        &serde_json::json!({"local-projects": projects, "other": [1, 2]}).to_string(),
    );
}

fn codex_cli(home: &Path, folders: &[&Path]) {
    let mut config = String::from("model = \"gpt\"\n\n");
    for folder in folders {
        config.push_str(&format!(
            "[projects.\"{}\"]\ntrust_level = \"trusted\"\n\n",
            text(folder)
        ));
    }
    write(home, CODEX_CLI_FILE, &config);
}

fn cursor(home: &Path, window: &str, json: &str) {
    write(
        home,
        &format!("{CURSOR_WORKSPACES_DIR}/{window}/workspace.json"),
        json,
    );
}

fn status(listed: &SavedProjects, source: ProjectSource) -> SourceStatus {
    listed
        .sources
        .iter()
        .find(|report| report.source == source)
        .unwrap()
        .status
}

/// Every listed project's path, in path order.
fn paths(listed: &SavedProjects) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = listed.projects.iter().map(|p| p.path.clone()).collect();
    paths.sort();
    paths
}

fn project<'a>(listed: &'a SavedProjects, path: &Path) -> &'a SavedProject {
    let path = real(path);
    listed
        .projects
        .iter()
        .find(|project| project.path == path)
        .unwrap_or_else(|| panic!("{path:?} not listed in {:?}", listed.projects))
}

#[test]
fn an_empty_home_reports_every_source_missing_and_lists_nothing() {
    let home = home();
    let listed = list_saved_projects(home.path());
    assert!(listed.projects.is_empty());
    assert_eq!(listed.skipped, SkipCounts::default());
    assert_eq!(
        listed.sources,
        ProjectSource::ALL
            .iter()
            .map(|&source| SourceReport {
                source,
                status: SourceStatus::Missing
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn each_source_is_parsed() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("code/alpha"));
    let b = mkdir(&h.join("code/beta"));
    let c = mkdir(&h.join("code/gamma"));
    let d = mkdir(&h.join("code/delta"));
    let e = mkdir(&h.join("code/epsilon with space"));
    claude_code(h, &[&a]);
    claude_app(h, &[("Beta app", &b)]);
    codex_app(h, &[("Gamma app", &[&c])]);
    codex_cli(h, &[&d]);
    cursor(
        h,
        "w1",
        &format!(r#"{{"folder": "file://{}"}}"#, text(&e).replace(' ', "%20")),
    );
    let listed = list_saved_projects(h);
    for source in ProjectSource::ALL {
        assert_eq!(status(&listed, source), SourceStatus::Read { count: 1 });
    }
    assert_eq!(listed.skipped, SkipCounts::default());
    let found: Vec<(&str, &Path, &[ProjectSource])> = listed
        .projects
        .iter()
        .map(|p| (p.name.as_str(), p.path.as_path(), p.sources.as_slice()))
        .collect();
    assert_eq!(
        found,
        vec![
            ("alpha", a.as_path(), &[ProjectSource::ClaudeCode][..]),
            ("Beta app", b.as_path(), &[ProjectSource::ClaudeApp][..]),
            ("delta", d.as_path(), &[ProjectSource::CodexCli][..]),
            (
                "epsilon with space",
                e.as_path(),
                &[ProjectSource::Cursor][..]
            ),
            ("Gamma app", c.as_path(), &[ProjectSource::CodexApp][..]),
        ]
    );
}

#[test]
fn non_local_presets_and_presets_without_target_are_ignored() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("a"));
    write(
        h,
        CLAUDE_APP_FILE,
        &serde_json::json!({"byAccount": {
            "one": {"presets": [
                {"name": "Cloud", "target": {"kind": "cloud"}},
                {"name": "No target"},
                {"name": "  ", "target": {"kind": "local", "cwd": text(&a)}}
            ]},
            "two": {}
        }})
        .to_string(),
    );
    let listed = list_saved_projects(h);
    assert_eq!(
        status(&listed, ProjectSource::ClaudeApp),
        SourceStatus::Read { count: 1 }
    );
    // A blank preset name is no name: the folder's own name is used.
    assert_eq!(project(&listed, &a).name, "a");
}

#[test]
fn an_absent_list_key_is_an_empty_read() {
    let home = home();
    let h = home.path();
    write(h, CLAUDE_CODE_FILE, r#"{"numStartups": 1}"#);
    write(h, CLAUDE_APP_FILE, r#"{}"#);
    write(h, CODEX_APP_FILE, r#"{"other": true}"#);
    write(h, CODEX_CLI_FILE, "model = \"x\"\n");
    fs::create_dir_all(h.join(CURSOR_WORKSPACES_DIR).join("empty-window")).unwrap();
    let listed = list_saved_projects(h);
    for source in ProjectSource::ALL {
        assert_eq!(status(&listed, source), SourceStatus::Read { count: 0 });
    }
}

#[test]
fn a_misshapen_source_is_unexpected_layout_and_the_rest_still_list() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("a"));
    cursor(h, "w", &format!(r#"{{"folder": "file://{}"}}"#, text(&a)));
    for (file, contents, source) in [
        (CLAUDE_CODE_FILE, "{not json", ProjectSource::ClaudeCode),
        // A top level that is not an object is never read by position.
        (
            CLAUDE_CODE_FILE,
            r#"[{"/a": {}}]"#,
            ProjectSource::ClaudeCode,
        ),
        (
            CLAUDE_APP_FILE,
            r#"[{"acct": {"presets": []}}]"#,
            ProjectSource::ClaudeApp,
        ),
        (
            CODEX_APP_FILE,
            r#"[{"x": {"name": "n", "rootPaths": ["/a"]}}]"#,
            ProjectSource::CodexApp,
        ),
        (
            CLAUDE_CODE_FILE,
            r#"{"projects": ["/not", "an object"]}"#,
            ProjectSource::ClaudeCode,
        ),
        // Nor is any object inside one that this reads.
        (
            CLAUDE_APP_FILE,
            r#"{"byAccount": {"x": [[]]}}"#,
            ProjectSource::ClaudeApp,
        ),
        (
            CLAUDE_APP_FILE,
            r#"{"byAccount": {"x": {"presets": [["Name", {"kind": "local", "cwd": "/a"}]]}}}"#,
            ProjectSource::ClaudeApp,
        ),
        (
            CLAUDE_APP_FILE,
            r#"{"byAccount": {"x": {"presets": [{"target": ["local", "/a"]}]}}}"#,
            ProjectSource::ClaudeApp,
        ),
        (
            CODEX_APP_FILE,
            r#"{"local-projects": {"x": ["n", ["/a"]]}}"#,
            ProjectSource::CodexApp,
        ),
        (
            CLAUDE_APP_FILE,
            r#"{"byAccount": {"x": {"presets": {"not": "a list"}}}}"#,
            ProjectSource::ClaudeApp,
        ),
        (
            CODEX_APP_FILE,
            r#"{"local-projects": {"x": {"rootPaths": "/not/a/list"}}}"#,
            ProjectSource::CodexApp,
        ),
        (
            CODEX_APP_FILE,
            r#"{"local-projects": ["/a"]}"#,
            ProjectSource::CodexApp,
        ),
        (
            CODEX_CLI_FILE,
            "projects = \"not a table\"\n",
            ProjectSource::CodexCli,
        ),
        (CODEX_CLI_FILE, "[projects\nbroken", ProjectSource::CodexCli),
    ] {
        write(h, file, contents);
        let listed = list_saved_projects(h);
        assert_eq!(
            status(&listed, source),
            SourceStatus::UnexpectedLayout,
            "{contents}"
        );
        // A broken source never fails the list.
        assert_eq!(
            status(&listed, ProjectSource::Cursor),
            SourceStatus::Read { count: 1 }
        );
        assert_eq!(listed.projects.len(), 1);
        fs::remove_file(h.join(file)).unwrap();
    }
}

#[test]
fn unreadable_and_too_large_sources_are_reported_and_never_read() {
    let home = home();
    let h = home.path();
    // A folder where a file is expected cannot be read as one.
    fs::create_dir_all(h.join(CLAUDE_CODE_FILE)).unwrap();
    // Cursor's window folder is a file.
    write(h, CURSOR_WORKSPACES_DIR, "not a folder");
    // A file past the cap, without writing its bytes.
    write(h, CODEX_APP_FILE, "");
    fs::File::options()
        .write(true)
        .open(h.join(CODEX_APP_FILE))
        .unwrap()
        .set_len(MAX_SOURCE_FILE_BYTES + 1)
        .unwrap();
    // Exactly at the cap is still read (and is not valid TOML).
    write(h, CODEX_CLI_FILE, "");
    fs::File::options()
        .write(true)
        .open(h.join(CODEX_CLI_FILE))
        .unwrap()
        .set_len(MAX_SOURCE_FILE_BYTES)
        .unwrap();
    // A file the user cannot open.
    write(h, CLAUDE_APP_FILE, "{}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(h.join(CLAUDE_APP_FILE), fs::Permissions::from_mode(0o000)).unwrap();
    }
    let listed = list_saved_projects(h);
    assert_eq!(
        status(&listed, ProjectSource::ClaudeCode),
        SourceStatus::Unreadable
    );
    assert_eq!(
        status(&listed, ProjectSource::Cursor),
        SourceStatus::Unreadable
    );
    assert_eq!(
        status(&listed, ProjectSource::CodexApp),
        SourceStatus::TooLarge
    );
    assert_eq!(
        status(&listed, ProjectSource::CodexCli),
        SourceStatus::UnexpectedLayout
    );
    #[cfg(unix)]
    assert_eq!(
        status(&listed, ProjectSource::ClaudeApp),
        SourceStatus::Unreadable
    );
}

#[test]
fn too_many_cursor_windows_is_too_large() {
    let home = home();
    let h = home.path();
    let dir = h.join(CURSOR_WORKSPACES_DIR);
    let limits = Limits { cursor_windows: 3 };
    for index in 0..3 {
        fs::create_dir_all(dir.join(index.to_string())).unwrap();
    }
    // Exactly at the cap is still read.
    assert_eq!(
        status(&list_within(h, limits), ProjectSource::Cursor),
        SourceStatus::Read { count: 0 }
    );
    fs::create_dir_all(dir.join("3")).unwrap();
    assert_eq!(
        status(&list_within(h, limits), ProjectSource::Cursor),
        SourceStatus::TooLarge
    );
    // The production cap is the documented one.
    assert_eq!(Limits::default().cursor_windows, MAX_CURSOR_WORKSPACES);
}
#[test]
fn duplicates_across_sources_merge_into_one_project_with_every_source() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("shared"));
    let trailing = format!("{}/", text(&a));
    let doubled = text(&a).replace("/shared", "//shared/.");
    claude_code(h, &[Path::new(&trailing)]);
    claude_app(h, &[("Shared", &a)]);
    codex_app(h, &[("Other name", &[Path::new(&doubled)])]);
    codex_cli(h, &[&a]);
    cursor(h, "w1", &format!(r#"{{"folder": "file://{}"}}"#, text(&a)));
    cursor(h, "w2", &format!(r#"{{"folder": "file://{trailing}"}}"#));
    let listed = list_saved_projects(h);
    assert_eq!(listed.projects.len(), 1, "{:?}", listed.projects);
    assert_eq!(
        listed.projects[0],
        SavedProject {
            name: "Shared".into(),
            path: a.clone(),
            sources: ProjectSource::ALL.to_vec(),
        }
    );
    assert_eq!(
        status(&listed, ProjectSource::Cursor),
        SourceStatus::Read { count: 2 }
    );
}

#[cfg(unix)]
#[test]
fn a_link_to_a_listed_folder_is_the_same_project() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("real"));
    std::os::unix::fs::symlink(&a, h.join("alias")).unwrap();
    claude_code(h, &[&a, &h.join("alias")]);
    let listed = list_saved_projects(h);
    assert_eq!(listed.projects.len(), 1);
    assert_eq!(listed.projects[0].path, a);
}

#[test]
fn every_skip_reason_is_counted() {
    let home = home();
    let h = home.path();
    let kept = mkdir(&h.join("kept"));
    let group = mkdir(&h.join("group"));
    let member = mkdir(&h.join("group/member"));
    let library = mkdir(&h.join("Library/Application Support/Thing"));
    let claude_internal = mkdir(&h.join(".claude/projects/x"));
    let codex_internal = mkdir(&h.join(".codex/sessions"));
    let codex_task = mkdir(&h.join("Documents/Codex/task-1"));
    let file = h.join("a-file");
    fs::write(&file, "").unwrap();
    // A real folder under a classified temporary root, independent of TMPDIR.
    let temporary = tempfile::TempDir::new_in(TEMP_ROOTS[0]).unwrap();
    claude_code(
        h,
        &[
            &kept,
            h,
            &group,
            &member,
            &library,
            &claude_internal,
            &file,
            &h.join("gone"),
            Path::new("relative/path"),
            temporary.path(),
            // `/tmp` is a link to `/private/tmp` on macOS; either is skipped.
            Path::new("/tmp"),
            Path::new("/tmp/does-not-exist-xtrace-saved-projects"),
        ],
    );
    codex_cli(h, &[&codex_internal, &codex_task, &library]);
    // A local preset with no folder, or one that is not text, is one skipped
    // entry; the rest of the file still counts.
    write(
        h,
        CLAUDE_APP_FILE,
        &serde_json::json!({"byAccount": {"acct": {"presets": [
            {"name": "No folder", "target": {"kind": "local"}},
            {"name": "Number", "target": {"kind": "local", "cwd": 7}},
            {"name": "Kept", "target": {"kind": "local", "cwd": text(&kept)}}
        ]}}})
        .to_string(),
    );
    cursor(
        h,
        "remote",
        r#"{"folder": "vscode-remote://ssh-remote+box/home/u/x"}"#,
    );
    cursor(
        h,
        "remote-host",
        r#"{"folder": "file://otherhost/share/x"}"#,
    );
    cursor(
        h,
        "multi",
        r#"{"workspace": "file:///Users/u/a.code-workspace"}"#,
    );
    cursor(h, "broken", "{nope");
    cursor(h, "neither", r#"{"other": 1}"#);
    cursor(h, "bad-escape", r#"{"folder": "file:///a/%ZZ"}"#);
    cursor(h, "bad-utf8", r#"{"folder": "file:///a/%FF"}"#);
    cursor(h, "array", r#"["file:///a"]"#);
    let listed = list_saved_projects(h);
    assert_eq!(
        listed.skipped,
        SkipCounts {
            invalid_path: 5,
            missing_folder: 2,
            temporary: 3,
            app_library: 1,
            app_internal: 2,
            codex_task: 1,
            remote: 2,
            multi_root_workspace: 1,
            unreadable_entry: 3,
            home_folder: 1,
            container_folder: 1,
        }
    );
    assert_eq!(paths(&listed), vec![member.clone(), kept.clone()]);
    // A source's count is every entry it named, including the ones it
    // skipped itself: every Cursor window with a `workspace.json` here, and
    // all three presets.
    assert_eq!(
        status(&listed, ProjectSource::Cursor),
        SourceStatus::Read { count: 8 }
    );
    assert_eq!(
        status(&listed, ProjectSource::ClaudeApp),
        SourceStatus::Read { count: 3 }
    );
    assert_eq!(project(&listed, &kept).name, "Kept");
}

#[test]
fn worktrees_fold_into_their_project() {
    let home = home();
    let h = home.path();
    let project_a = mkdir(&h.join("code/app"));
    let claude_tree = mkdir(&h.join("code/app/.claude/worktrees/fix-1"));
    let plain_tree = mkdir(&h.join("code/app/worktrees/feature"));
    // A worktree of a worktree lands in the first project.
    let nested_tree = mkdir(&h.join("code/app/worktrees/feature/.claude/worktrees/deep"));
    claude_code(h, &[&project_a, &claude_tree, &nested_tree]);
    codex_cli(h, &[&plain_tree]);
    claude_app(h, &[("Worktree preset", &claude_tree)]);
    let listed = list_saved_projects(h);
    assert_eq!(listed.projects.len(), 1, "{:?}", listed.projects);
    let app = &listed.projects[0];
    // No worktree is listed on its own.
    assert_eq!(app.path, project_a);
    // A worktree's sources count for its project; its name never names it.
    assert_eq!(app.name, "app");
    assert_eq!(
        app.sources,
        vec![
            ProjectSource::ClaudeCode,
            ProjectSource::ClaudeApp,
            ProjectSource::CodexCli
        ]
    );
}

#[test]
fn a_worktree_whose_project_is_not_listed_stays_its_own_project() {
    let home = home();
    let h = home.path();
    let tree = mkdir(&h.join("code/app/worktrees/feature"));
    claude_code(h, &[&tree]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![tree.clone()]);
    assert_eq!(project(&listed, &tree).name, "feature");
}

#[test]
fn a_nested_folder_stays_separate_even_with_a_deeper_worktrees_segment() {
    let home = home();
    let h = home.path();
    let outer = mkdir(&h.join("xtrace-code"));
    // A repository of its own, so it is not hidden as a container.
    git_entry(&outer, false);
    let inner = mkdir(&h.join("xtrace-code/agent-plugins"));
    // `worktrees` is not directly under `outer` (or its `.claude`), and the
    // folder it is under is not listed: this is its own project.
    let deeper = mkdir(&h.join("xtrace-code/xtrace-desktop/worktrees/one"));
    let deeper_claude = mkdir(&h.join("xtrace-code/tool/.claude/worktrees/two"));
    claude_code(h, &[&outer, &inner, &deeper, &deeper_claude]);
    let listed = list_saved_projects(h);
    let mut expected = vec![outer, inner, deeper, deeper_claude];
    expected.sort();
    assert_eq!(paths(&listed), expected);
    // Nothing was folded into the outer folder.
    for project in &listed.projects {
        assert_eq!(project.sources, vec![ProjectSource::ClaudeCode]);
    }
}

#[test]
fn folders_inside_a_worktree_fold_into_the_project() {
    let home = home();
    let h = home.path();
    let app = mkdir(&h.join("app"));
    let tree = mkdir(&h.join("app/worktrees/w"));
    let inside = mkdir(&h.join("app/worktrees/w/crates"));
    // A folder inside a worktree folds in even when the worktree itself is
    // not listed.
    let only_inside = mkdir(&h.join("app/.claude/worktrees/v/src"));
    claude_code(h, &[&app]);
    codex_cli(h, &[&tree, &inside]);
    cursor(
        h,
        "w",
        &format!(r#"{{"folder": "file://{}"}}"#, text(&only_inside)),
    );
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![app.clone()]);
    assert_eq!(
        project(&listed, &app).sources,
        vec![
            ProjectSource::ClaudeCode,
            ProjectSource::CodexCli,
            ProjectSource::Cursor
        ]
    );
}
#[test]
fn two_worktrees_both_fold_into_the_project() {
    let home = home();
    let h = home.path();
    let app = mkdir(&h.join("app"));
    let one = mkdir(&h.join("app/worktrees/one"));
    let two = mkdir(&h.join("app/.claude/worktrees/two"));
    claude_code(h, &[&app]);
    claude_app(h, &[("One", &one)]);
    codex_cli(h, &[&two]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![app.clone()]);
    let project = project(&listed, &app);
    assert_eq!(project.name, "app");
    assert_eq!(
        project.sources,
        vec![
            ProjectSource::ClaudeCode,
            ProjectSource::ClaudeApp,
            ProjectSource::CodexCli
        ]
    );
}
#[test]
fn a_bare_worktrees_container_folds_into_the_project() {
    let home = home();
    let h = home.path();
    let app = mkdir(&h.join("code/app"));
    let container = mkdir(&h.join("code/app/worktrees"));
    let claude_container = mkdir(&h.join("code/app/.claude/worktrees"));
    let tree = mkdir(&h.join("code/app/worktrees/x"));
    claude_code(h, &[&app, &tree]);
    codex_cli(h, &[&container]);
    cursor(
        h,
        "w",
        &format!(r#"{{"folder": "file://{}"}}"#, text(&claude_container)),
    );
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![app.clone()]);
    assert_eq!(
        project(&listed, &app).sources,
        vec![
            ProjectSource::ClaudeCode,
            ProjectSource::CodexCli,
            ProjectSource::Cursor
        ]
    );
}

#[test]
fn a_folder_above_the_home_folder_never_absorbs_anything() {
    let root = home();
    let above = real(root.path());
    git_entry(&above, false);
    let h = mkdir(&root.path().join("home"));
    let tree = mkdir(&above.join("worktrees/x"));
    claude_code(&h, &[&above]);
    codex_cli(&h, &[&tree]);
    let listed = list_saved_projects(&h);
    assert_eq!(paths(&listed), vec![above.clone(), tree.clone()]);
    assert_eq!(
        project(&listed, &above).sources,
        vec![ProjectSource::ClaudeCode]
    );
}
#[test]
fn the_deepest_containing_project_takes_the_worktree() {
    let home = home();
    let h = home.path();
    let outer = mkdir(&h.join("code"));
    git_entry(&outer, true);
    let inner = mkdir(&h.join("code/app"));
    let tree = mkdir(&h.join("code/app/worktrees/x"));
    claude_code(h, &[&outer, &inner]);
    codex_cli(h, &[&tree]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![outer.clone(), inner.clone()]);
    assert_eq!(
        project(&listed, &inner).sources,
        vec![ProjectSource::ClaudeCode, ProjectSource::CodexCli]
    );
    assert_eq!(
        project(&listed, &outer).sources,
        vec![ProjectSource::ClaudeCode]
    );
}

#[test]
fn the_home_folder_is_never_listed_and_never_absorbs_anything() {
    let home = home();
    let h = home.path();
    // Even as a repository of its own, the home folder is not a project.
    git_entry(h, false);
    let tree = mkdir(&h.join("worktrees/x"));
    let claude_tree = mkdir(&h.join("proj/.claude/worktrees/y"));
    claude_code(h, &[h]);
    codex_cli(h, &[&tree, &claude_tree]);
    let listed = list_saved_projects(h);
    let mut expected = vec![tree.clone(), claude_tree.clone()];
    expected.sort();
    assert_eq!(paths(&listed), expected);
    // Nothing of the home folder's went into another project.
    for path in [&tree, &claude_tree] {
        assert_eq!(
            project(&listed, path).sources,
            vec![ProjectSource::CodexCli]
        );
    }
    assert_eq!(
        listed.skipped,
        SkipCounts {
            home_folder: 1,
            ..SkipCounts::default()
        }
    );
}

#[cfg(unix)]
#[test]
fn the_home_folder_alone_lists_nothing_in_any_spelling() {
    let root = home();
    let h = mkdir(&root.path().join("home"));
    let alias = root.path().join("home-link");
    std::os::unix::fs::symlink(&h, &alias).unwrap();
    let trailing = format!("{}/", text(&h));
    claude_code(&h, &[&h, Path::new(&trailing)]);
    codex_cli(&h, &[&alias]);
    // Read through the link: the real path and the link's own spelling are
    // both the home folder. Like every skip decided from the path alone, it
    // counts once per spelling (`h` and `h/` are one spelling).
    let listed = list_saved_projects(&alias);
    assert!(listed.projects.is_empty(), "{:?}", listed.projects);
    assert_eq!(
        listed.skipped,
        SkipCounts {
            home_folder: 2,
            ..SkipCounts::default()
        }
    );
}

#[test]
fn a_folder_that_only_holds_other_projects_is_hidden() {
    let home = home();
    let h = home.path();
    let group = mkdir(&h.join("xtrace-code"));
    let one = mkdir(&h.join("xtrace-code/one"));
    let two = mkdir(&h.join("xtrace-code/two"));
    git_entry(&one, false);
    claude_code(h, &[&group, &one]);
    claude_app(h, &[("Group", &group)]);
    codex_app(h, &[("Two", &[&two])]);
    cursor(
        h,
        "w",
        &format!(r#"{{"folder": "file://{}"}}"#, text(&group)),
    );
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![one.clone(), two.clone()]);
    // The container's sources and name went nowhere.
    assert_eq!(
        project(&listed, &one).sources,
        vec![ProjectSource::ClaudeCode]
    );
    assert_eq!(project(&listed, &one).name, "one");
    assert_eq!(
        project(&listed, &two).sources,
        vec![ProjectSource::CodexApp]
    );
    assert_eq!(
        listed.skipped,
        SkipCounts {
            container_folder: 1,
            ..SkipCounts::default()
        }
    );
    // Its sources still read and counted what they named.
    assert_eq!(
        status(&listed, ProjectSource::Cursor),
        SourceStatus::Read { count: 1 }
    );
}

#[test]
fn a_folder_with_its_own_git_entry_stays_even_with_projects_inside() {
    for as_file in [false, true] {
        let home = home();
        let h = home.path();
        let outer = mkdir(&h.join("repo"));
        git_entry(&outer, as_file);
        let inner = mkdir(&h.join("repo/vendor/lib"));
        claude_code(h, &[&outer, &inner]);
        let listed = list_saved_projects(h);
        assert_eq!(
            paths(&listed),
            vec![outer.clone(), inner.clone()],
            "{as_file}"
        );
        assert_eq!(listed.skipped, SkipCounts::default());
    }
}

#[test]
fn a_folder_whose_only_nested_folders_are_its_worktrees_stays() {
    let home = home();
    let h = home.path();
    // No `.git` entry: the worktrees fold into it first, so it holds no other
    // listed project.
    let app = mkdir(&h.join("code/app"));
    let tree = mkdir(&h.join("code/app/worktrees/x"));
    let claude_tree = mkdir(&h.join("code/app/.claude/worktrees/y"));
    claude_code(h, &[&app]);
    codex_cli(h, &[&tree, &claude_tree]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![app.clone()]);
    assert_eq!(
        project(&listed, &app).sources,
        vec![ProjectSource::ClaudeCode, ProjectSource::CodexCli]
    );
    assert_eq!(listed.skipped, SkipCounts::default());
}

#[test]
fn a_folder_that_absorbed_a_worktree_is_never_a_container() {
    let home = home();
    let h = home.path();
    let ws = mkdir(&h.join("ws"));
    let repo = mkdir(&h.join("ws/repo"));
    git_entry(&repo, false);
    let tree = mkdir(&h.join("ws/worktrees/w"));
    git_entry(&tree, true);
    claude_code(h, &[&ws, &repo]);
    codex_cli(h, &[&tree]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![ws.clone(), repo.clone()]);
    // The worktree folded into `ws`: not listed on its own, its source on `ws`.
    assert_eq!(
        project(&listed, &ws).sources,
        vec![ProjectSource::ClaudeCode, ProjectSource::CodexCli]
    );
    assert_eq!(
        project(&listed, &repo).sources,
        vec![ProjectSource::ClaudeCode]
    );
    assert_eq!(listed.skipped, SkipCounts::default());
}

#[test]
fn a_folder_that_absorbed_a_bare_worktrees_container_is_never_a_container() {
    let home = home();
    let h = home.path();
    let ws = mkdir(&h.join("ws"));
    let repo = mkdir(&h.join("ws/repo"));
    let bare = mkdir(&h.join("ws/.claude/worktrees"));
    claude_code(h, &[&ws, &repo]);
    codex_cli(h, &[&bare]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![ws.clone(), repo.clone()]);
    assert_eq!(
        project(&listed, &ws).sources,
        vec![ProjectSource::ClaudeCode, ProjectSource::CodexCli]
    );
    assert_eq!(listed.skipped.container_folder, 0);
}

#[test]
fn a_container_with_a_worktree_folded_project_inside_is_still_hidden() {
    let home = home();
    let h = home.path();
    let group = mkdir(&h.join("group"));
    let app = mkdir(&h.join("group/app"));
    let tree = mkdir(&h.join("group/app/worktrees/x"));
    claude_code(h, &[&group, &app]);
    codex_cli(h, &[&tree]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![app.clone()]);
    assert_eq!(
        project(&listed, &app).sources,
        vec![ProjectSource::ClaudeCode, ProjectSource::CodexCli]
    );
    assert_eq!(listed.skipped.container_folder, 1);
}

#[test]
fn containers_inside_containers_are_all_hidden() {
    let home = home();
    let h = home.path();
    let a = mkdir(&h.join("a"));
    let b = mkdir(&h.join("a/b"));
    let c = mkdir(&h.join("a/b/c"));
    git_entry(&c, false);
    // A folder with nothing listed inside it is a project, `.git` or not.
    let lone = mkdir(&h.join("a/lone"));
    claude_code(h, &[&a, &b, &c]);
    codex_cli(h, &[&lone]);
    let listed = list_saved_projects(h);
    assert_eq!(paths(&listed), vec![c.clone(), lone.clone()]);
    assert_eq!(
        listed.skipped,
        SkipCounts {
            container_folder: 2,
            ..SkipCounts::default()
        }
    );
}

#[test]
fn names_prefer_the_claude_app_then_the_codex_app_then_the_folder() {
    let home = home();
    let h = home.path();
    let both = mkdir(&h.join("both"));
    let codex_only = mkdir(&h.join("codex-only"));
    let neither = mkdir(&h.join("neither"));
    claude_app(h, &[("Claude name", &both)]);
    codex_app(
        h,
        &[
            ("Codex name", &[&both]),
            ("Codex only", &[&codex_only]),
            ("", &[&neither]),
        ],
    );
    let listed = list_saved_projects(h);
    assert_eq!(project(&listed, &both).name, "Claude name");
    assert_eq!(project(&listed, &codex_only).name, "Codex only");
    assert_eq!(project(&listed, &neither).name, "neither");
}

#[test]
fn projects_are_ordered_by_name_ignoring_case_then_path() {
    let home = home();
    let h = home.path();
    let zed = mkdir(&h.join("a/zed"));
    let apple = mkdir(&h.join("b/Apple"));
    let banana = mkdir(&h.join("c/banana"));
    let same_1 = mkdir(&h.join("d/same"));
    let same_2 = mkdir(&h.join("e/Same"));
    claude_code(h, &[&zed, &apple, &banana, &same_2, &same_1]);
    let listed = list_saved_projects(h);
    let order: Vec<&Path> = listed.projects.iter().map(|p| p.path.as_path()).collect();
    assert_eq!(
        order,
        vec![
            apple.as_path(),
            banana.as_path(),
            same_1.as_path(),
            same_2.as_path(),
            zed.as_path()
        ]
    );
}

#[test]
fn cursor_uris_are_percent_decoded_and_localhost_is_local() {
    let home = home();
    let h = home.path();
    let spaced = mkdir(&h.join("My Project (1)"));
    let accented = mkdir(&h.join("café"));
    let encoded = text(&spaced)
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29");
    cursor(h, "w1", &format!(r#"{{"folder": "file://{encoded}"}}"#));
    cursor(
        h,
        "w2",
        &format!(
            r#"{{"folder": "FILE://localhost{}"}}"#,
            text(&accented).replace('é', "%C3%A9")
        ),
    );
    let listed = list_saved_projects(h);
    assert_eq!(project(&listed, &spaced).name, "My Project (1)");
    assert_eq!(project(&listed, &accented).name, "café");
    assert_eq!(listed.skipped, SkipCounts::default());
}

#[test]
fn the_pure_rules_hold_without_a_file_system() {
    assert_eq!(lexical_path("relative"), None);
    assert_eq!(lexical_path(""), None);
    assert_eq!(lexical_path("/a//b/./c/"), Some(PathBuf::from("/a/b/c")));
    assert_eq!(lexical_path("/a/b/../c"), Some(PathBuf::from("/a/c")));
    assert_eq!(file_uri_path("file:///a%20b"), Ok("/a b".into()));
    assert_eq!(file_uri_path("file://localhost/a"), Ok("/a".into()));
    assert_eq!(
        file_uri_path("file://localhostx/a"),
        Err(SkipReason::Remote)
    );
    assert_eq!(file_uri_path("file:///a%2"), Err(SkipReason::InvalidPath));
    assert_eq!(file_uri_path("https://x/a"), Err(SkipReason::Remote));
    assert_eq!(file_uri_path("file:"), Err(SkipReason::Remote));
    let homes = Homes {
        given: PathBuf::from("/Users/u"),
        canonical: None,
    };
    for (path, reason) in [
        ("/tmp", Some(SkipReason::Temporary)),
        ("/private/tmp/x", Some(SkipReason::Temporary)),
        ("/var/folders/ab/T/x", Some(SkipReason::Temporary)),
        ("/private/var/folders/ab", Some(SkipReason::Temporary)),
        ("/tmpfoo", None),
        ("/Users/u/Library/x", Some(SkipReason::AppLibrary)),
        ("/Users/u/.claude", Some(SkipReason::AppInternal)),
        ("/Users/u/.codex/x", Some(SkipReason::AppInternal)),
        ("/Users/u/.claude-plugins", None),
        ("/Users/u/Documents/Codex/t", Some(SkipReason::CodexTask)),
        ("/Users/u/Documents/Codexes", None),
        ("/Users/u", Some(SkipReason::HomeFolder)),
        ("/Users", None),
        ("/Users/u/code", None),
    ] {
        assert_eq!(skip_reason(Path::new(path), &homes), reason, "{path}");
    }
    let listed = [PathBuf::from("/Users/u"), PathBuf::from("/Users/u/a")];
    assert_eq!(
        fold_target(Path::new("/Users/u/a/worktrees/x"), &listed, &homes),
        Some(&listed[1])
    );
    assert_eq!(
        fold_target(Path::new("/Users/u/worktrees/x"), &listed, &homes),
        None
    );
    // A segment merely named like one does not count.
    assert_eq!(
        fold_target(Path::new("/Users/u/a/my-worktrees/x"), &listed, &homes),
        None
    );
    assert_eq!(file_uri_path("file:///a%+7"), Err(SkipReason::InvalidPath));
    assert_eq!(file_uri_path("file:///a%7"), Err(SkipReason::InvalidPath));
    assert_eq!(file_uri_path("file:///a%2f%2F"), Ok("/a//".into()));
    // `/` and every folder above home are above it; a sibling is not.
    let listed = [
        PathBuf::from("/"),
        PathBuf::from("/Users"),
        PathBuf::from("/Users/v"),
    ];
    assert_eq!(
        fold_target(Path::new("/worktrees/x"), &listed, &homes),
        None
    );
    assert_eq!(
        fold_target(Path::new("/Users/worktrees/x"), &listed, &homes),
        None
    );
    assert_eq!(
        fold_target(Path::new("/Users/v/worktrees/x"), &listed, &homes),
        Some(&listed[2])
    );
    for (inside, folds) in [
        ("worktrees", true),
        (".claude/worktrees", true),
        ("worktrees/x", true),
        ("worktrees/x/y", true),
        (".claude/worktrees/x", true),
        (".claude/worktrees/x/y", true),
        (".claude", false),
        ("src/worktrees/x", false),
        ("a/.claude/worktrees/x", false),
        (".claude/x/worktrees/y", false),
    ] {
        assert_eq!(is_worktree_path(Path::new(inside)), folds, "{inside}");
    }
}
