//! `xtrace-core make-rubric` end to end, against a fake `claude` named by
//! `XTRACE_CLAUDE`.
#![cfg(unix)]
//!
//! The command runs with a cleared environment: a synthetic home and
//! temporary folder under Cargo's target temp folder, a short `PATH` and the
//! override. The real Claude Code CLI never runs and the real home folder is
//! never read.

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

/// A valid rubric the fake writes; XTrace sets its version.
const MADE: &str = "---\nprotocol: 1\nid: readme-check\nname: README Check\nversion: 9\n\
     description: How well the README explains the project.\ntarget: project\n---\n\
     ## Install steps (60 points)\nAward 60 when ok, else 0.\n\n\
     ## Usage (40 points)\nAward 40 when ok, else 0.\n";

struct Case {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Case {
    /// A skill folder `skill/` and a fake that records each try's prompt in
    /// `prompts.jsonl`, then runs `body` (Python; `OUT` is the try's output
    /// folder, `TRY` its name).
    fn new(body: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("make-rubric-cli-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for folder in ["skill/scripts", "home", "bin", "tmp"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(
            root.join("skill/SKILL.md"),
            "---\nname: readme\ndescription: Checks READMEs.\n---\nRead the README.\n",
        )
        .unwrap();
        fs::write(root.join("skill/scripts/check.py"), "print('ok')\n").unwrap();
        let fake = root.join("bin/claude");
        fs::write(
            &fake,
            format!(
                "#!{python} -I\nimport json, os, sys\n\
                 if sys.argv[1:] == ['--version']:\n    print('2.1.296 (Claude Code)')\n    sys.exit(0)\n\
                 OUT = os.environ['XTRACE_GRADE_DIR']\nTRY = os.path.basename(OUT)\n\
                 with open({prompts:?}, 'a') as f:\n    f.write(json.dumps({{'prompt': sys.argv[2], 'cwd': os.getcwd()}}) + '\\n')\n\
                 def write(name, text):\n    open(os.path.join(OUT, name), 'w').write(text)\n\
                 MADE = {made:?}\n{body}\n",
                python = python3().display(),
                prompts = root.join("prompts.jsonl").to_str().unwrap(),
                made = MADE,
            ),
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        Self { _dir: dir, root }
    }

    fn make(&self, out: &Path, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtrace-core"))
            .args(["make-rubric", "--skill"])
            .arg(self.root.join("skill"))
            .arg("--out")
            .arg(out)
            .args(extra)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("PATH", "/usr/bin:/bin")
            .env("XTRACE_CLAUDE", self.root.join("bin/claude"))
            .output()
            .unwrap()
    }

    fn prompts(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.root.join("prompts.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn report(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn a_made_rubric_is_copied_to_out_with_xtraces_version_and_nothing_is_saved() {
    let case = Case::new("write('xtrace-rubric.md', MADE)");
    let out = case.root.join("made");
    let output = case.make(&out, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        report(&output),
        serde_json::json!({
            "outcome": "made", "reason": null, "id": "readme-check", "version": "1",
            "criteria": [{"name": "Install steps", "points": 60}, {"name": "Usage", "points": 40}],
            "tries": 1,
        })
    );
    assert_eq!(
        fs::read_to_string(out.join("xtrace-rubric.md")).unwrap(),
        MADE.replace("version: 9\n", "version: 1\n")
    );
    // Nothing next to the skill; the run used a staged copy, since removed.
    assert!(!case.root.join("skill/xtrace-rubric.md").exists());
    let prompts = case.prompts();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("XTrace rubric-making run (headless).")
    );
    let cwd = prompts[0]["cwd"].as_str().unwrap();
    assert!(
        cwd.starts_with(
            case.root
                .join("tmp/xtrace-grading/maker-")
                .to_str()
                .unwrap()
        )
    );
    assert!(!Path::new(cwd).exists(), "the staging folder is removed");
}

#[test]
fn a_remake_follows_the_refused_rubrics_version() {
    let case = Case::new("write('xtrace-rubric.md', MADE)");
    fs::write(
        case.root.join("skill/xtrace-rubric.md"),
        "---\nversion: 3\n---\nno points",
    )
    .unwrap();
    // Without --remake a refused rubric is a usage error.
    let refused = case.make(&case.root.join("first"), &[]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--remake"));
    assert!(!case.root.join("first").exists());
    let out = case.root.join("again");
    let output = case.make(&out, &["--remake"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report(&output)["version"], "4");
    assert!(
        case.prompts()[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("(a copy is in TARGET) is refused by XTrace")
    );
    // The skill's own broken rubric is untouched.
    assert_eq!(
        fs::read_to_string(case.root.join("skill/xtrace-rubric.md")).unwrap(),
        "---\nversion: 3\n---\nno points"
    );
}

#[test]
fn a_refusal_writes_its_reason_and_is_not_retried() {
    let case =
        Case::new("write('xtrace-refusal.md', 'It publishes releases; it reviews nothing.')");
    let out = case.root.join("refused");
    let output = case.make(&out, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        report(&output),
        serde_json::json!({
            "outcome": "refused", "reason": "It publishes releases; it reviews nothing.",
            "id": null, "version": null, "criteria": [], "tries": 1,
        })
    );
    assert_eq!(
        fs::read_to_string(out.join("refusal.txt")).unwrap(),
        "It publishes releases; it reviews nothing.\n"
    );
    assert!(!out.join("xtrace-rubric.md").exists());
    assert_eq!(case.prompts().len(), 1);
}

#[test]
fn a_job_without_a_rubric_fails_with_exit_2_after_its_retry() {
    let case = Case::new("pass");
    let out = case.root.join("failed");
    let output = case.make(&out, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        report(&output),
        serde_json::json!({
            "outcome": "failed",
            "reason": "Claude wrote neither a rubric nor a refusal, on the retry either",
            "id": null, "version": null, "criteria": [], "tries": null,
        })
    );
    assert_eq!(case.prompts().len(), 2);
    assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
}

#[test]
fn an_out_folder_inside_the_skill_is_refused_before_anything_is_created() {
    let case = Case::new("write('xtrace-rubric.md', MADE)");
    let inside = case.root.join("skill/rubrics/deep/one");
    let output = case.make(&inside, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "the output folder and the skill folder must not contain each other"
    );
    assert!(!case.root.join("skill/rubrics").exists());
    // A folder holding the skill is refused too, and so is a non-empty one.
    let output = case.make(&case.root, &[]);
    assert_eq!(output.status.code(), Some(1));
    let busy = case.root.join("busy");
    fs::create_dir(&busy).unwrap();
    fs::write(busy.join("old"), "x").unwrap();
    let output = case.make(&busy, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "the output folder must be empty"
    );
    assert!(case.prompts().is_empty(), "Claude Code never ran");
    assert!(!case.root.join("tmp/xtrace-grading").exists());
}
