//! `xtrace-core grade` end to end, against a fake `claude` named by
//! `XTRACE_CLAUDE`.
#![cfg(unix)]
//!
//! The command runs with a cleared environment: a synthetic home folder under
//! Cargo's target temp folder, a short `PATH` and the override. The real
//! Claude Code CLI never runs and the real home folder is never read.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn python3() -> PathBuf {
    let search = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&search)
        .chain([PathBuf::from("/usr/bin")])
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_absolute() && path.is_file())
        .expect("python3 is required for this test")
}

struct Case {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Case {
    fn new(fake_body: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("grade-cli-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for folder in ["project", "home", "bin"] {
            fs::create_dir(root.join(folder)).unwrap();
        }
        let fake = root.join("bin/claude");
        fs::write(
            &fake,
            format!(
                "#!{} -I\nimport json, os, sys\n\
                 if sys.argv[1:] == ['--version']:\n    print('2.1.296 (Claude Code)')\n    sys.exit(0)\n\
                 G = os.environ['XTRACE_GRADE_DIR']\n\
                 json.dump(sorted(os.environ), open(os.path.join(os.path.dirname(G), 'env.json'), 'w'))\n{fake_body}\n",
                python3().display()
            ),
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        Self { _dir: dir, root }
    }

    fn grade(&self, out: &Path, claude: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtrace-core"))
            .args(["grade", "--project"])
            .arg(self.root.join("project"))
            .args(["--skill", "claude-md-doctor", "--out"])
            .arg(out)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("XTRACE_CLAUDE", claude)
            .output()
            .unwrap()
    }
}

const GOOD: &str = r#"open(os.path.join(G, 'result.json'), 'w').write(json.dumps({
    'protocol': 1, 'score': 95, 'summary': 'One warning.',
    'insights': [{'severity': 'warning', 'title': 'Long file', 'fix': 'Trim it.'}],
    'criteria': [{'name': 'Health of the memory files', 'max': 100, 'awarded': 95}],
    'report': None, 'complete': True}))"#;

#[test]
fn prints_the_validated_result() {
    let case = Case::new(GOOD);
    let out = case.root.join("grades/one");
    let output = case.grade(&out, &case.root.join("bin/claude"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "score": 95, "summary": "One warning.",
            "insights": [{"severity": "warning", "title": "Long file", "fix": "Trim it."}],
            "report": null, "complete": true,
            "criteria": [{"name": "Health of the memory files", "max": 100, "awarded": 95}]
        })
    );
    // The override names the executable; it is not passed on to it.
    let env: Vec<String> =
        serde_json::from_slice(&fs::read(case.root.join("grades/env.json")).unwrap()).unwrap();
    assert!(!env.contains(&"XTRACE_CLAUDE".to_owned()));
    assert!(env.contains(&"XTRACE_GRADE_DIR".to_owned()));
}

#[test]
fn a_run_without_a_result_exits_2_with_a_reason() {
    let case = Case::new("pass");
    let output = case.grade(&case.root.join("grades/two"), &case.root.join("bin/claude"));
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "the grading run wrote no result file"
    );
}

#[test]
fn usage_errors_exit_1_without_running_anything() {
    let case = Case::new(GOOD);
    let out = case.root.join("grades/three");
    let relative = case.grade(&out, Path::new("claude"));
    assert_eq!(relative.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&relative.stderr).contains("XTRACE_CLAUDE"));
    assert!(!out.exists());

    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("old.json"), "{}").unwrap();
    let busy = case.grade(&out, &case.root.join("bin/claude"));
    assert_eq!(busy.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&busy.stderr).trim(),
        "the grade folder must be empty"
    );
    assert!(!case.root.join("grades/env.json").exists());
}

#[test]
fn overlapping_folders_are_refused_before_any_folder_is_created() {
    let case = Case::new(GOOD);
    let claude = case.root.join("bin/claude");
    let overlap = "the grade folder and the project folder must not contain each other";

    let inside = case.root.join("project/grades/deep/one");
    let output = case.grade(&inside, &claude);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), overlap);
    assert!(!case.root.join("project/grades").exists());

    // Through a link: the nearest existing ancestor is resolved.
    std::os::unix::fs::symlink(case.root.join("project"), case.root.join("alias")).unwrap();
    let output = case.grade(&case.root.join("alias/new"), &claude);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), overlap);
    assert!(!case.root.join("project/new").exists());

    // The project inside the grade folder.
    let output = case.grade(&case.root, &claude);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), overlap);

    // A grade folder path with whitespace.
    let spaced = case.root.join("my grades/one");
    let output = case.grade(&spaced, &claude);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "the grade folder's path contains whitespace"
    );
    assert!(!case.root.join("my grades").exists());

    assert_eq!(
        fs::read_dir(case.root.join("project")).unwrap().count(),
        0,
        "nothing was created in the project"
    );
    assert!(!case.root.join("env.json").exists(), "the fake never ran");
}
