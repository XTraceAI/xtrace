//! The grading service against a fake `claude`.
//!
//! Every case builds a synthetic world under this workspace's `target/tmp`:
//! a project folder, a home folder, a temporary folder for staging, an app
//! data folder whose path contains a space, and a fake `claude` (a small
//! Python program) named by `XTRACE_CLAUDE` in a fixed environment. The real
//! Claude Code CLI is never run (an override that is not executable is an
//! error, never a fallback), and neither the real home nor XTrace's live
//! data folders are read.
use super::*;
use crate::{
    grading_dto::{
        GradingInstalledRubric, GradingInstalledSkill, GradingRubricAction, GradingRubricProblem,
    },
    projects_dto::{ProjectEntry, ProjectListLoaded},
    state::{AppState, StartupOptions},
};
use std::os::unix::fs::PermissionsExt;
use xt_store::grades::GradeStatus;

const GOOD: &str = r#"{"protocol": 1, "score": 75, "summary": "One dead reference.",
  "insights": [{"severity": "critical", "title": "Dead path", "fix": "Remove it."},
               {"severity": "warning", "title": "Long"}],
  "criteria": [{"name": "Health of the memory files", "max": 100, "awarded": 75}],
  "report": "report.html", "complete": true}"#;

/// Writes a finished grade folder: report, card, the skill's own extras and
/// a work folder holding a conversation excerpt, then the result.
const WRITE_GOOD: &str = "write('report.html', '<p>report</p>')\n\
     write('card.svg', '<svg/>')\n\
     write('report.json', '{}')\n\
     os.makedirs(os.path.join(G, 'work'), exist_ok=True)\n\
     write('work/excerpts.jsonl', 'conversation text')\n\
     write('result.json', GOOD)";

/// Waits, recording its pid and a child's, until it is ended.
const HANG: &str = "child = subprocess.Popen(['/bin/sleep', '600'])\n\
     with open(PIDS, 'w') as f:\n    f.write('%d %d' % (os.getpid(), child.pid))\n\
     open(READY, 'w').close()\n\
     time.sleep(600)";

#[derive(Default)]
struct Seen {
    changed: usize,
    progress: Vec<GradingProgress>,
    rubric_progress: Vec<GradingRubricProgress>,
}

struct World {
    _root: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    grades: PathBuf,
    data: PathBuf,
    claude: PathBuf,
    record: PathBuf,
    pids: PathBuf,
    ready: PathBuf,
    state: Arc<AppState>,
    listed: Arc<Mutex<Vec<String>>>,
    seen: Arc<Mutex<Seen>>,
    /// Every file handed to the system to open; nothing is really opened.
    opened: Arc<Mutex<Vec<PathBuf>>>,
}

fn python3() -> PathBuf {
    xt_probes::executable::resolve_within(
        "python3",
        None,
        std::env::var_os("PATH").as_deref(),
        &[Path::new("/usr/bin"), Path::new("/opt/homebrew/bin")],
    )
    .expect("python3 is required for these tests")
}

impl World {
    fn new(body: &str) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("grading-")
            .tempdir_in(&parent)
            .unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let project = root.join("project");
        let temp = root.join("tmp");
        let data = root.join("app data");
        let home = root.join("home");
        let bin = root.join("bin");
        for folder in [&project, &temp, &data, &home, &bin] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let (record, pids, ready) = (
            root.join("record.json"),
            root.join("pids"),
            root.join("ready"),
        );
        let claude = bin.join("claude");
        let script = format!(
            "#!{python} -I\nimport json, os, signal, subprocess, sys, time\n\
             RECORD = {record:?}\nPIDS = {pids:?}\nREADY = {ready:?}\nMARKS = {marks:?}\n\
             if sys.argv[1:] == ['--version']:\n    print('2.1.296 (Claude Code)')\n    sys.exit(0)\n\
             with open(RECORD, 'w') as f:\n    json.dump({{'argv': sys.argv, 'cwd': os.getcwd(), 'grade': os.environ['XTRACE_GRADE_DIR']}}, f)\n\
             G = os.environ['XTRACE_GRADE_DIR']\n\
             def write(name, text):\n    with open(os.path.join(G, name), 'w') as f:\n        f.write(text)\n\
             GOOD = {good:?}\n{body}\n",
            python = python3().display(),
            record = record.to_str().unwrap(),
            pids = pids.to_str().unwrap(),
            ready = ready.to_str().unwrap(),
            marks = root.to_str().unwrap(),
            good = GOOD,
        );
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = Arc::new(
            AppState::build(
                StartupOptions {
                    data_dir: Some(data.clone()),
                    native_home: Some(home.clone()),
                    ..Default::default()
                },
                || panic!("the default data folder is never resolved"),
                || panic!("the default home is never resolved"),
            )
            .unwrap(),
        );
        Self {
            _root: dir,
            listed: Arc::new(Mutex::new(vec![project.to_str().unwrap().to_owned()])),
            grades: data.join(GRADES_DIR),
            data,
            root,
            project,
            home,
            temp,
            claude,
            record,
            pids,
            ready,
            state,
            seen: Arc::new(Mutex::new(Seen::default())),
            opened: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn environment(&self, claude: &Path) -> Vec<(OsString, OsString)> {
        [
            ("HOME", self.root.join("home").to_str().unwrap()),
            ("PATH", "/usr/bin:/bin"),
            ("TMPDIR", self.temp.to_str().unwrap()),
            ("XTRACE_CLAUDE", claude.to_str().unwrap()),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    fn config(&self) -> GradingConfig {
        let listed = Arc::clone(&self.listed);
        GradingConfig {
            skills_root: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor"),
            home: self.home.clone(),
            temp_dir: self.temp.clone(),
            grades_dir: self.grades.clone(),
            environment: RunEnvironment::Fixed(self.environment(&self.claude)),
            projects: Arc::new(move || {
                ProjectList::Loaded(ProjectListLoaded {
                    projects: lock(&listed)
                        .iter()
                        .map(|path| ProjectEntry {
                            name: "project".into(),
                            path: path.clone(),
                            sources: vec![],
                        })
                        .collect(),
                    sources: vec![],
                    skipped: Default::default(),
                })
            }),
            limits: Limits {
                deadline: Duration::from_secs(60),
                exit_grace: Duration::from_secs(1),
                ..Limits::default()
            },
            maker_dir: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../crates/xt-judge/rubric-maker"),
            rubric_store: self.data.join(xt_judge::rubric_store::STORE_DIR),
            refusals: self.data.join(xt_judge::refusals::REFUSALS_FILE),
            maker_key: xt_judge::maker::data_key(&self.data),
            maker_limits: Limits {
                deadline: Duration::from_secs(60),
                exit_grace: Duration::from_secs(1),
                ..Limits::default()
            },
            keep_folders: KEEP_FOLDERS,
            progress_every: Duration::from_millis(100),
            shutdown_bound: SHUTDOWN_BOUND,
            clock: Arc::new(now_ms),
            open_file: {
                let opened = Arc::clone(&self.opened);
                Arc::new(move |path: &Path| {
                    lock(&opened).push(path.to_path_buf());
                    true
                })
            },
            before_staging: None,
            before_save: None,
        }
    }

    fn service_with(&self, config: GradingConfig) -> GradingService {
        let (changed, progress, rubric) = (
            Arc::clone(&self.seen),
            Arc::clone(&self.seen),
            Arc::clone(&self.seen),
        );
        GradingService::new(
            config,
            Arc::new(Arc::clone(&self.state)),
            Arc::new(move || lock(&changed).changed += 1),
            Arc::new(move |report: &GradingProgress| lock(&progress).progress.push(report.clone())),
            Arc::new(move |report: &GradingRubricProgress| {
                lock(&rubric).rubric_progress.push(report.clone());
            }),
        )
    }

    fn service(&self) -> GradingService {
        self.service_with(self.config())
    }

    fn project_path(&self) -> &str {
        self.project.to_str().unwrap()
    }

    fn row(&self, run_id: &str) -> GradeRow {
        self.state
            .with_store(|store| Ok(store.grade(run_id)?))
            .unwrap()
            .unwrap()
    }

    fn rows(&self) -> usize {
        self.state
            .with_store(|store| Ok(store.grades(500)?))
            .unwrap()
            .len()
    }

    /// Waits until the run is no longer running and the slot is free.
    fn settle(&self, service: &GradingService, run_id: &str) -> GradeRow {
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            if service.active_run().is_none() {
                return self.row(run_id);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the run did not finish");
    }

    fn wait_ready(&self) {
        let until = Instant::now() + Duration::from_secs(30);
        while !self.ready.exists() {
            assert!(Instant::now() < until, "the fake never got ready");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn record(&self) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(&self.record).unwrap()).unwrap()
    }

    fn staging_root(&self, run_id: &str) -> PathBuf {
        self.temp.join(xt_judge::staging::STAGING_DIR).join(run_id)
    }

    fn pids(&self) -> Vec<String> {
        std::fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
}

fn started(start: GradingStart) -> String {
    match start {
        GradingStart::Started { run_id } => run_id,
        other => panic!("not started: {other:?}"),
    }
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn all_gone(pids: &[String]) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if pids.iter().all(|pid| !alive(pid)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_good_run_is_saved_kept_without_work_and_its_staging_removed() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Succeeded);
    assert_eq!(row.score, Some(75));
    assert_eq!(row.complete, Some(true));
    assert_eq!(row.summary.as_deref(), Some("One dead reference."));
    assert_eq!(row.report_file.as_deref(), Some("report.html"));
    assert_eq!(row.project_path, world.project_path());
    assert_eq!(row.skill_version, bundled_version());
    assert!(row.skill_version.starts_with("0.5.1+rubric.1 #"));
    // Only the result, the report and the card beside it are kept; never
    // work/, nor the skill's own files. The data folder has a space.
    let kept = world.grades.join(&run_id);
    assert!(kept.to_str().unwrap().contains(' '));
    assert_eq!(
        files_under(&kept),
        ["card.svg", "report.html", "result.json"]
    );
    // The run happened in a staging folder without whitespace, now removed.
    let record = world.record();
    let grade = record["grade"].as_str().unwrap();
    assert_eq!(
        Path::new(grade),
        world.staging_root(&run_id).join("grade").as_path()
    );
    let argv: Vec<&str> = record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    let add_dir = argv.iter().position(|arg| *arg == "--add-dir").unwrap();
    for path in &argv[add_dir + 1..] {
        assert!(!path.contains(char::is_whitespace), "{path}");
        assert!(path.starts_with(world.staging_root(&run_id).to_str().unwrap()));
    }
    assert_eq!(record["cwd"], world.project_path());
    assert!(!world.staging_root(&run_id).exists());
    // The DTO carries the stored insights back exactly.
    let GradingRuns::Loaded { runs } = service.runs() else {
        panic!("runs not loaded");
    };
    assert_eq!(runs.len(), 1);
    let insights = runs[0].insights.as_ref().unwrap();
    assert_eq!(insights.len(), 2);
    assert_eq!(insights[0].fix.as_deref(), Some("Remove it."));
    assert_eq!(runs[0].failure, None);
    // Committed changes: the running row, then the finish.
    let seen = lock(&world.seen);
    assert_eq!(seen.changed, 2);
    let first = seen.progress.first().unwrap();
    assert_eq!(
        (first.state, first.elapsed_seconds),
        (GradingProgressState::Running, 0)
    );
    let last = seen.progress.last().unwrap();
    assert_eq!(last.state, GradingProgressState::Finished);
    assert!(seen.progress.iter().all(|report| report.run_id == run_id));
}

#[test]
fn a_folder_not_in_a_fresh_project_list_is_refused_and_nothing_is_saved() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let other = world.root.join("other");
    std::fs::create_dir(&other).unwrap();
    assert_eq!(
        service.start(other.to_str().unwrap(), "claude-md-doctor"),
        GradingStart::NotAProject
    );
    // The list is read fresh: a project that left it is refused too.
    lock(&world.listed).clear();
    assert_eq!(
        service.start(world.project_path(), "claude-md-doctor"),
        GradingStart::NotAProject
    );
    assert_eq!(world.rows(), 0);
    assert!(!world.record.exists(), "the fake never ran");
}

#[test]
fn link_and_path_tricks_are_refused() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let link = world.root.join("link");
    std::os::unix::fs::symlink(&world.project, &link).unwrap();
    let file = world.root.join("file");
    std::fs::write(&file, "").unwrap();
    let tricks = [
        format!("{}/", world.project_path()),
        format!("{}/../project", world.project_path()),
        format!("{}/./", world.project_path()),
        link.to_str().unwrap().to_owned(),
        file.to_str().unwrap().to_owned(),
        "project".to_owned(),
    ];
    // Even when the list itself names the trick, it is not the resolved folder.
    lock(&world.listed).extend(tricks.iter().cloned());
    for trick in &tricks {
        assert_eq!(
            service.start(trick, "claude-md-doctor"),
            GradingStart::NotAProject,
            "{trick}"
        );
    }
    assert_eq!(world.rows(), 0);
}

#[test]
fn a_project_that_leaves_the_list_before_launch_fails_without_running() {
    let world = World::new(WRITE_GOOD);
    let mut config = world.config();
    let calls = Arc::new(Mutex::new(0_u32));
    let path = world.project_path().to_owned();
    let counted = Arc::clone(&calls);
    config.projects = Arc::new(move || {
        let mut calls = lock(&counted);
        *calls += 1;
        let projects = if *calls == 1 {
            vec![ProjectEntry {
                name: "project".into(),
                path: path.clone(),
                sources: vec![],
            }]
        } else {
            vec![]
        };
        ProjectList::Loaded(ProjectListLoaded {
            projects,
            sources: vec![],
            skipped: Default::default(),
        })
    });
    let service = world.service_with(config);
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("project_changed"));
    assert_eq!(*lock(&calls), 2);
    assert!(!world.record.exists(), "the fake never ran");
    assert!(!world.staging_root(&run_id).exists());
}

#[test]
fn one_run_at_a_time_and_cancel_stops_it_with_progress_meanwhile() {
    let world = World::new(HANG);
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    assert_eq!(service.active_run().as_deref(), Some(run_id.as_str()));
    assert_eq!(
        service.start(world.project_path(), "claude-md-doctor"),
        GradingStart::Busy
    );
    world.wait_ready();
    std::thread::sleep(Duration::from_millis(350));
    assert!(!service.cancel("not-this-run"));
    assert!(service.cancel(&run_id));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Cancelled);
    assert_eq!(row.failure_reason, None);
    assert!(all_gone(&world.pids()));
    assert!(!world.staging_root(&run_id).exists());
    assert!(!world.grades.join(&run_id).exists());
    assert!(!service.cancel(&run_id), "nothing is running any more");
    let seen = lock(&world.seen);
    assert_eq!(seen.changed, 2);
    let running = seen
        .progress
        .iter()
        .filter(|report| report.state == GradingProgressState::Running)
        .count();
    assert!(running >= 3, "{running} progress reports while running");
    assert_eq!(
        seen.progress.last().unwrap().state,
        GradingProgressState::Finished
    );
}

#[test]
fn unknown_skills_a_missing_claude_and_a_failed_run_have_typed_answers() {
    let world = World::new("sys.exit(3)");
    let service = world.service();
    assert_eq!(
        service.start(world.project_path(), "other-skill"),
        GradingStart::UnknownSkill
    );
    let mut config = world.config();
    config.environment = RunEnvironment::Fixed(world.environment(&world.root.join("bin/none")));
    assert_eq!(
        world
            .service_with(config)
            .start(world.project_path(), "claude-md-doctor"),
        GradingStart::ClaudeUnavailable
    );
    let mut config = world.config();
    config.skills_root = world.root.join("no-skills");
    let missing = world.service_with(config);
    assert_eq!(
        missing.start(world.project_path(), "claude-md-doctor"),
        GradingStart::SkillUnavailable
    );
    assert_eq!(missing.skills(), GradingSkills::Failed);
    assert_eq!(world.rows(), 0);
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("claude_failed"));
    assert!(!world.staging_root(&run_id).exists());
}

/// The app bundles exactly each built-in skill's folder and its license,
/// at the place `skills_root = <resources>/grading-skills` resolves.
#[test]
fn the_bundle_holds_each_skill_folder_and_license_where_the_app_looks() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest.join("tauri.conf.json")).unwrap()).unwrap();
    let grading: Vec<(String, String)> = config["bundle"]["resources"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(from, to)| (from.clone(), to.as_str().unwrap().to_owned()))
        .filter(|(_, to)| to.starts_with(&format!("{BUNDLE_RESOURCE}/")))
        .collect();
    let mut expected = Vec::new();
    for id in BUILTIN {
        let skill = skill::bundled_dir(Path::new(BUNDLE_RESOURCE), id).unwrap();
        let license = skill.with_file_name("LICENSE");
        expected.push((
            format!("../../../vendor/{id}/skill"),
            skill.to_str().unwrap().to_owned(),
        ));
        expected.push((
            format!("../../../vendor/{id}/LICENSE"),
            license.to_str().unwrap().to_owned(),
        ));
    }
    // The rubric maker, where `GradingConfig::new` looks for it.
    expected.push((
        "../../../crates/xt-judge/rubric-maker".to_owned(),
        format!("{BUNDLE_RESOURCE}/{}", xt_judge::maker::HELPER_ID),
    ));
    let mut grading = grading;
    grading.sort();
    expected.sort();
    assert_eq!(grading, expected);
    for (from, _) in &grading {
        assert!(manifest.join(from).exists(), "{from}");
    }
    for id in BUILTIN {
        GradingSkill::open(&manifest.join(format!("../../../vendor/{id}/skill"))).unwrap();
    }
    let config = GradingConfig::new(
        PathBuf::from("/resources").join(BUNDLE_RESOURCE),
        PathBuf::from("/home"),
        Path::new("/data"),
        Arc::new(|| ProjectList::Failed),
    );
    assert_eq!(
        config.maker_dir,
        Path::new("/resources")
            .join(BUNDLE_RESOURCE)
            .join(xt_judge::maker::HELPER_ID)
    );
    assert_eq!(config.rubric_store, Path::new("/data/rubrics"));
    assert_eq!(
        config.maker_limits.deadline,
        xt_judge::maker::DEFAULT_DEADLINE
    );
    xt_judge::maker::Helper::open(&manifest.join("../../../crates/xt-judge/rubric-maker")).unwrap();
}

#[test]
fn skills_come_from_the_bundled_rubric() {
    let world = World::new(WRITE_GOOD);
    let GradingSkills::Loaded { skills } = world.service().skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "claude-md-doctor");
    assert_eq!(skills[0].name, "CLAUDE.md Doctor");
    assert_eq!(skills[0].version, bundled_version());
    assert!(skills[0].description.contains("CLAUDE.md and AGENTS.md"));
}

/// A copy of the bundled skills whose claude-md-doctor rubric is `rubric`.
fn skills_with_rubric(world: &World, rubric: Option<&str>) -> PathBuf {
    let root = world.root.join("skills-copy");
    let skill = root.join("claude-md-doctor/skill");
    std::fs::create_dir_all(&skill).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/claude-md-doctor/skill");
    std::fs::copy(source.join("SKILL.md"), skill.join("SKILL.md")).unwrap();
    if let Some(rubric) = rubric {
        std::fs::write(skill.join(xt_judge::rubric::RUBRIC_FILE), rubric).unwrap();
    }
    root
}

#[test]
fn a_missing_or_invalid_rubric_runs_nothing() {
    let world = World::new(WRITE_GOOD);
    let other = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../vendor/claude-md-doctor/skill/xtrace-rubric.md"),
    )
    .unwrap()
    .replace("id: claude-md-doctor", "id: someone-else");
    for rubric in [
        None,
        Some("no header"),
        Some("---\nprotocol: 1\n---\n"),
        Some(other.as_str()),
    ] {
        let mut config = world.config();
        config.skills_root = skills_with_rubric(&world, rubric);
        let service = world.service_with(config);
        assert_eq!(service.skills(), GradingSkills::Failed, "{rubric:?}");
        assert_eq!(
            service.start(world.project_path(), "claude-md-doctor"),
            GradingStart::SkillUnavailable,
            "{rubric:?}"
        );
        std::fs::remove_dir_all(world.root.join("skills-copy")).unwrap();
    }
    assert_eq!(world.rows(), 0);
    assert!(!world.record.exists(), "the fake never ran");
}

#[test]
fn a_missing_sandbox_fails_the_run_with_its_own_reason() {
    let world = World::new(
        "sys.stderr.write('\\nError: sandbox required but unavailable: no backend\\n  \
         sandbox.failIfUnavailable is set \\u2014 refusing to start without a working \
         sandbox.\\n')\nsys.exit(1)",
    );
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("sandbox_unavailable"));
    assert!(!world.grades.join(&run_id).exists());
    assert!(!world.staging_root(&run_id).exists());
}

#[test]
fn an_old_claude_code_fails_the_run_with_its_own_reason() {
    let world = World::new(WRITE_GOOD);
    let script = std::fs::read_to_string(&world.claude).unwrap();
    std::fs::write(
        &world.claude,
        script.replace("2.1.296 (Claude Code)", "2.1.200 (Claude Code)"),
    )
    .unwrap();
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("claude_too_old"));
    assert!(!world.record.exists(), "the grading run never started");
    assert!(!world.staging_root(&run_id).exists());
}

#[test]
fn a_result_without_the_rubric_criteria_fails_as_invalid() {
    let world = World::new(
        "r = json.loads(GOOD)\ndel r['criteria']\nwrite('report.html', 'x')\n\
         write('result.json', json.dumps(r))",
    );
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("result_invalid"));
}

#[test]
fn a_report_name_the_store_would_refuse_fails_as_invalid_before_anything_is_kept() {
    // The protocol and the grades table agree on report names: two dots in
    // a row are refused by both, so the run says its result was invalid
    // instead of archiving it and then losing it as an internal error.
    let world = World::new(
        "r = json.loads(GOOD)\nr['report'] = 'audit..report.html'\n\
         write('audit..report.html', '<p>report</p>')\n\
         write('result.json', json.dumps(r))",
    );
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("result_invalid"));
    assert!(!world.grades.join(&run_id).exists());
    assert!(!world.staging_root(&run_id).exists());
}

fn bundled_version() -> String {
    GradingSkill::open_builtin(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/claude-md-doctor/skill"),
        "claude-md-doctor",
    )
    .unwrap()
    .version()
}

#[test]
fn an_existing_work_folder_must_stay_exactly_as_it_was() {
    let unchanged = World::new(WRITE_GOOD);
    let folder = unchanged.project.join(".claude-md-doctor");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("mine.txt"), "the user's own").unwrap();
    let service = unchanged.service();
    let run_id = started(service.start(unchanged.project_path(), "claude-md-doctor"));
    assert_eq!(
        unchanged.settle(&service, &run_id).status,
        GradeStatus::Succeeded
    );

    let changed = World::new(&format!(
        "open(os.path.join(os.getcwd(), '.claude-md-doctor', 'new.txt'), 'w').close()\n{WRITE_GOOD}"
    ));
    let folder = changed.project.join(".claude-md-doctor");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("mine.txt"), "the user's own").unwrap();
    let service = changed.service();
    let run_id = started(service.start(changed.project_path(), "claude-md-doctor"));
    let row = changed.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("project_written"));
    assert!(folder.join("mine.txt").is_file(), "nothing is deleted");
    assert!(!changed.grades.join(&run_id).exists());
}

#[test]
fn quitting_sends_sigterm_and_saves_the_run_interrupted() {
    let world = World::new(&format!(
        "def on_term(*_):\n    open(os.path.join(MARKS, 'term'), 'w').close()\n    sys.exit(0)\n\
         signal.signal(signal.SIGTERM, on_term)\n{HANG}"
    ));
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    let begun = Instant::now();
    assert!(service.shutdown());
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "SIGTERM was honoured"
    );
    assert!(world.root.join("term").exists());
    let row = world.row(&run_id);
    assert_eq!(row.status, GradeStatus::Interrupted);
    assert_eq!(row.failure_reason.as_deref(), Some("app_closed"));
    assert!(all_gone(&world.pids()));
    assert!(!world.staging_root(&run_id).exists());
    assert_eq!(
        service.start(world.project_path(), "claude-md-doctor"),
        GradingStart::Stopped
    );
}

#[test]
fn quitting_ends_a_group_that_ignores_sigterm_with_sigkill_after_the_grace() {
    let world = World::new(&format!(
        "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n{HANG}"
    ));
    let mut config = world.config();
    config.limits.term_grace = Duration::from_millis(700);
    let service = world.service_with(config);
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    let begun = Instant::now();
    assert!(service.shutdown());
    assert!(begun.elapsed() >= Duration::from_millis(700));
    assert!(all_gone(&world.pids()));
    let row = world.row(&run_id);
    assert_eq!(row.status, GradeStatus::Interrupted);
    assert!(!world.staging_root(&run_id).exists());
}

#[test]
fn startup_marks_leftover_runs_interrupted_and_removes_only_their_staging() {
    let world = World::new(WRITE_GOOD);
    let leftover = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let unknown = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    world
        .state
        .with_store(|store| {
            Ok(store.grade_insert_running(&NewGradeRun {
                run_id: leftover,
                project_path: world.project_path(),
                skill_id: "claude-md-doctor",
                skill_version: "1",
                started_at_ms: 1,
            })?)
        })
        .unwrap();
    let parent = world.temp.join(xt_judge::staging::STAGING_DIR);
    // As staging makes it: private to this user.
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    for name in [leftover, unknown, "not-a-run"] {
        std::fs::create_dir_all(parent.join(name).join("grade/work")).unwrap();
    }
    let service = world.service();
    assert_eq!(service.recover(), 1);
    let row = world.row(leftover);
    assert_eq!(row.status, GradeStatus::Interrupted);
    assert_eq!(row.failure_reason.as_deref(), Some("app_closed"));
    assert!(!parent.join(leftover).exists());
    // Another copy of the app's run, and anything not named like a run, stay.
    assert!(parent.join(unknown).is_dir());
    assert!(parent.join("not-a-run").is_dir());
    assert_eq!(lock(&world.seen).changed, 1);
    let GradingRuns::Loaded { runs } = service.runs() else {
        panic!("runs not loaded");
    };
    assert_eq!(runs[0].failure, Some(GradingFailure::AppClosed));
}

/// The process an earlier start of the app recorded for a run.
fn recorded_leader(world: &World, run_id: &str) -> xt_probes::leader::GroupLeader {
    let path = world
        .staging_root(run_id)
        .join(xt_judge::staging::LEADER_RECORD);
    xt_probes::leader::GroupLeader::parse(&std::fs::read(path).unwrap()).expect("a leader record")
}

/// A Claude Code that outlived the app: the app that started it is gone (its
/// service is left behind, as a crash leaves it, and knows nothing of the
/// next start), and the next start's service recovers over the same database.
#[test]
fn startup_stops_a_claude_that_outlived_the_app_then_removes_its_staging() {
    for ignores_sigterm in [false, true] {
        let world = World::new(&if ignores_sigterm {
            format!("signal.signal(signal.SIGTERM, signal.SIG_IGN)\n{HANG}")
        } else {
            format!(
                "def on_term(*_):\n    open(os.path.join(MARKS, 'term'), 'w').close()\n    sys.exit(0)\n\
                 signal.signal(signal.SIGTERM, on_term)\n{HANG}"
            )
        });
        let crashed = world.service();
        let run_id = started(crashed.start(world.project_path(), "claude-md-doctor"));
        world.wait_ready();
        let leader = recorded_leader(&world, &run_id);
        assert_eq!(world.pids()[0], leader.pid.to_string());
        assert!(leader.is_running());
        let mut config = world.config();
        config.limits.term_grace = Duration::from_millis(700);
        let next = world.service_with(config);
        let begun = Instant::now();
        assert_eq!(next.recover(), 1);
        if ignores_sigterm {
            assert!(begun.elapsed() >= Duration::from_millis(700));
        } else {
            assert!(begun.elapsed() < Duration::from_millis(700));
            assert!(world.root.join("term").exists(), "SIGTERM came first");
        }
        assert!(all_gone(&world.pids()), "the whole group is gone");
        assert!(!world.staging_root(&run_id).exists());
        let row = world.row(&run_id);
        assert_eq!(row.status, GradeStatus::Interrupted);
        assert_eq!(row.failure_reason.as_deref(), Some("app_closed"));
        // The left-behind service sees its child end; the saved row stays.
        world.settle(&crashed, &run_id);
        assert_eq!(world.row(&run_id).status, GradeStatus::Interrupted);
    }
}

/// A recorded process whose start time differs (its ID was reused) is never
/// signalled, and neither is one another copy of the app's run recorded.
#[test]
fn startup_signals_no_process_but_the_exact_recorded_one_of_its_own_run() {
    use xt_probes::leader::{GroupLeader, ProcessStart};
    let world = World::new(WRITE_GOOD);
    let leftover = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let unknown = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    world
        .state
        .with_store(|store| {
            Ok(store.grade_insert_running(&NewGradeRun {
                run_id: leftover,
                project_path: world.project_path(),
                skill_id: "claude-md-doctor",
                skill_version: "1",
                started_at_ms: 1,
            })?)
        })
        .unwrap();
    let parent = world.temp.join(xt_judge::staging::STAGING_DIR);
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("60").stdin(std::process::Stdio::null());
    let mut child = xt_probes::process::spawn_grouped(&mut command).unwrap();
    let leader = GroupLeader::observe(child.id()).expect("a group leader");
    let reused = GroupLeader {
        start: ProcessStart {
            seconds: leader.start.seconds + 1,
            ..leader.start
        },
        ..leader
    };
    for (name, record) in [(leftover, reused), (unknown, leader)] {
        std::fs::create_dir_all(parent.join(name).join("grade")).unwrap();
        std::fs::write(
            parent.join(name).join(xt_judge::staging::LEADER_RECORD),
            record.record(),
        )
        .unwrap();
    }
    assert_eq!(world.service().recover(), 1);
    assert!(!parent.join(leftover).exists());
    assert!(parent.join(unknown).is_dir());
    assert!(leader.is_running(), "nothing was signalled");
    assert_eq!(child.try_wait().unwrap(), None);
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn only_the_newest_succeeded_runs_keep_their_folders() {
    let world = World::new(WRITE_GOOD);
    let mut config = world.config();
    config.keep_folders = 2;
    let service = world.service_with(config);
    std::fs::create_dir_all(world.grades.join("11111111-2222-4333-8444-555555555555")).unwrap();
    std::fs::create_dir_all(world.grades.join("not-a-run")).unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
        world.settle(&service, &run_id);
        ids.push(run_id);
        // Distinct start times order the runs.
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!world.grades.join(&ids[0]).exists());
    assert!(world.grades.join(&ids[1]).is_dir());
    assert!(world.grades.join(&ids[2]).is_dir());
    assert!(
        !world
            .grades
            .join("11111111-2222-4333-8444-555555555555")
            .exists()
    );
    assert!(world.grades.join("not-a-run").is_dir());
    assert_eq!(world.rows(), 3, "rows stay");
    // Each run publishes its start and its finish; a prune that removed a
    // folder publishes once more (the first run's removed the stray folder,
    // the third's the first run's), one that removed nothing does not.
    assert_eq!(lock(&world.seen).changed, 2 + 1 + 2 + 2 + 1);
}

#[test]
fn a_content_purge_clears_results_and_removes_every_kept_folder() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.settle(&service, &run_id);
    assert!(world.grades.join(&run_id).is_dir());
    let purged = service
        .purge_with_folders(|| world.state.purge_stored_content(|| ()))
        .unwrap();
    assert!(purged.invalidate_content);
    assert!(!world.grades.join(&run_id).exists());
    let row = world.row(&run_id);
    assert_eq!(row.status, GradeStatus::Succeeded);
    assert_eq!(row.score, Some(75));
    assert_eq!(
        (row.summary, row.insights_json, row.report_file),
        (None, None, None)
    );
    // A failed purge removes nothing.
    std::fs::create_dir_all(world.grades.join(&run_id)).unwrap();
    let failed: Result<(), ()> = service.purge_with_folders(|| Err(()));
    assert!(failed.is_err());
    assert!(world.grades.join(&run_id).is_dir());
}

#[test]
fn a_kept_run_shows_its_details_and_opens_only_its_own_report() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.settle(&service, &run_id);
    let GradingRunDetails::Loaded(details) = service.run_details(&run_id) else {
        panic!("details not loaded");
    };
    assert_eq!((details.score, details.complete), (75, true));
    let crate::grading_dto::GradingRunContent::Kept(kept) = &details.content else {
        panic!("content kept");
    };
    assert_eq!(kept.summary, "One dead reference.");
    let crate::grading_dto::GradingRunFindings::Listed { findings } = &kept.findings else {
        panic!("findings listed");
    };
    assert_eq!(findings.len(), 2);
    assert_eq!(
        kept.criteria,
        crate::grading_dto::GradingRunCriteria::Reported {
            criteria: vec![crate::grading_dto::GradingCriterionScore {
                name: "Health of the memory files".into(),
                max: 100,
                awarded: 75,
            }]
        }
    );
    assert_eq!(kept.report, crate::grading_dto::GradingRunReport::Kept);
    assert_eq!(service.open_report(&run_id), GradingReportOpen::Opened);
    assert_eq!(
        *lock(&world.opened),
        [std::fs::canonicalize(world.grades.join(&run_id).join("report.html")).unwrap()]
    );
    // An ID no run has, a path, and a run that did not succeed open nothing.
    for unknown in [
        "11111111-2222-4333-8444-555555555555",
        "../report.html",
        world
            .grades
            .join(&run_id)
            .join("report.html")
            .to_str()
            .unwrap(),
    ] {
        assert_eq!(service.run_details(unknown), GradingRunDetails::NotFound);
        assert_eq!(service.open_report(unknown), GradingReportOpen::NotFound);
    }
    let failed = World::new("sys.exit(3)");
    let failed_service = failed.service();
    let failed_id = started(failed_service.start(failed.project_path(), "claude-md-doctor"));
    failed.settle(&failed_service, &failed_id);
    assert_eq!(
        failed_service.run_details(&failed_id),
        GradingRunDetails::NotFound
    );
    assert_eq!(
        failed_service.open_report(&failed_id),
        GradingReportOpen::NotFound
    );
    assert!(lock(&failed.opened).is_empty());
    // A system that cannot open it is a failure, not an opened report.
    let mut config = world.config();
    config.open_file = Arc::new(|_: &Path| false);
    assert_eq!(
        world.service_with(config).open_report(&run_id),
        GradingReportOpen::Failed
    );
    // Deleting stored content leaves the score alone and opens nothing.
    service
        .purge_with_folders(|| world.state.purge_stored_content(|| ()))
        .unwrap();
    let GradingRunDetails::Loaded(details) = service.run_details(&run_id) else {
        panic!("details not loaded after a purge");
    };
    assert_eq!(details.score, 75);
    assert_eq!(
        details.content,
        crate::grading_dto::GradingRunContent::Deleted
    );
    assert_eq!(
        service.open_report(&run_id),
        GradingReportOpen::ContentDeleted
    );
    assert_eq!(lock(&world.opened).len(), 1);
}

#[test]
fn a_clock_moved_back_during_a_run_still_finishes_it_and_frees_the_next_start() {
    const START: i64 = 1_790_000_000_000;
    for body in [WRITE_GOOD, "sys.exit(3)"] {
        let world = World::new(body);
        let now = Arc::new(std::sync::atomic::AtomicI64::new(START));
        let mut config = world.config();
        let clock = Arc::clone(&now);
        config.clock = Arc::new(move || clock.load(Ordering::SeqCst));
        let service = world.service_with(config);
        let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
        // The clock steps back an hour before the run finishes.
        now.store(START - 3_600_000, Ordering::SeqCst);
        let row = world.settle(&service, &run_id);
        assert_ne!(row.status, GradeStatus::Running, "{body}");
        assert_eq!(row.finished_at_ms, Some(START), "{body}");
        let next = started(service.start(world.project_path(), "claude-md-doctor"));
        assert_ne!(world.settle(&service, &next).status, GradeStatus::Running);
    }
}

/// A trigger in the disposable database refuses one kind of finish.
fn refuse_finish(world: &World, status: &str) {
    rusqlite::Connection::open(world.state.database_path().unwrap())
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER refuse_finish BEFORE UPDATE ON grades WHEN NEW.status='{status}' \
             BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;"
        ))
        .unwrap();
}

#[test]
fn a_finish_that_fails_falls_back_to_failed_and_the_next_start_works() {
    // A succeeded finish refused: no folder is kept, the run is failed.
    let world = World::new(WRITE_GOOD);
    refuse_finish(&world, "succeeded");
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("internal_error"));
    assert!(!world.grades.join(&run_id).exists());
    assert!(!world.staging_root(&run_id).exists());

    // A cancelled finish refused: the fallback still ends the row.
    let world = World::new(HANG);
    refuse_finish(&world, "cancelled");
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    assert!(service.cancel(&run_id));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("internal_error"));
    assert!(all_gone(&world.pids()));
    // No row is left running, so the next run can start.
    std::fs::remove_file(&world.ready).unwrap();
    let next = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    assert!(service.cancel(&next));
    assert_ne!(world.settle(&service, &next).status, GradeStatus::Running);
}

#[test]
fn the_quit_bound_covers_the_term_grace_the_drain_and_a_margin() {
    assert!(
        SHUTDOWN_BOUND
            >= runner::DEFAULT_TERM_GRACE
                + xt_probes::process::DRAIN_GRACE
                + Duration::from_secs(2)
    );
    // Waited for on its own thread, so quitting can stop other services.
    let world = World::new(HANG);
    let service = world.service();
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    let waiting = service.shutdown_in_background().unwrap();
    assert!(waiting.join().unwrap());
    assert_eq!(world.row(&run_id).status, GradeStatus::Interrupted);
}

#[test]
fn startup_removes_nothing_under_a_staging_parent_it_cannot_trust() {
    let leftover = "0f8fad5b-d9cb-469f-a165-70867728950e";
    for unsafe_parent in ["open to others", "a link"] {
        let world = World::new(WRITE_GOOD);
        world
            .state
            .with_store(|store| {
                Ok(store.grade_insert_running(&NewGradeRun {
                    run_id: leftover,
                    project_path: world.project_path(),
                    skill_id: "claude-md-doctor",
                    skill_version: "1",
                    started_at_ms: 1,
                })?)
            })
            .unwrap();
        let parent = world.temp.join(xt_judge::staging::STAGING_DIR);
        let real = if unsafe_parent == "a link" {
            let real = world.root.join("real-staging");
            std::fs::create_dir(&real).unwrap();
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::os::unix::fs::symlink(&real, &parent).unwrap();
            real
        } else {
            std::fs::create_dir(&parent).unwrap();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
            parent.clone()
        };
        std::fs::create_dir_all(real.join(leftover).join("grade")).unwrap();
        let service = world.service();
        assert_eq!(service.recover(), 0, "{unsafe_parent}");
        assert!(real.join(leftover).is_dir(), "{unsafe_parent}");
        // The row is still saved interrupted.
        assert_eq!(world.row(leftover).status, GradeStatus::Interrupted);
    }
}

/// The service's overview reads a fresh project list, the bundled skill and
/// the stored runs: a run going is live from its stored row, and a finished
/// one's score is shown while its project stays listed.
#[test]
fn the_overview_reads_the_list_the_skill_and_the_stored_runs_fresh() {
    let world = World::new(&format!(
        "if os.path.exists(os.path.join(MARKS, 'hang')):\n    exec({HANG:?})\n{WRITE_GOOD}"
    ));
    let service = world.service();
    let loaded = |overview: GradingOverview| match overview {
        GradingOverview::Loaded(loaded) => loaded,
        other => panic!("not loaded: {other:?}"),
    };
    let before = loaded(service.overview());
    assert_eq!(before.projects.len(), 1);
    assert_eq!(before.skills[0].skill.id, "claude-md-doctor");
    assert_eq!(before.skills[0].rows[0].shown, None);
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.settle(&service, &run_id);
    let after = loaded(service.overview());
    let skill = &after.skills[0];
    assert_eq!((skill.average, skill.graded, skill.total), (Some(75), 1, 1));
    assert_eq!(skill.rows[0].shown.as_ref().unwrap().run_id, run_id);
    assert_eq!(skill.top_insights[0].title, "Dead path");
    std::fs::write(world.root.join("hang"), "").unwrap();
    let live = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    let during = loaded(service.overview());
    assert_eq!(during.running.as_ref().unwrap().run_id, live);
    assert!(during.skills[0].rows[0].running);
    assert!(service.cancel(&live));
    world.settle(&service, &live);
    // The project left the list: its score no longer counts.
    lock(&world.listed).clear();
    let gone = loaded(service.overview());
    assert_eq!((gone.skills[0].average, gone.skills[0].total), (None, 0));
    assert_eq!(gone.running, None);
}

#[test]
fn fixture_mode_answers_not_configured_and_runs_nothing() {
    let service = GradingService::not_configured();
    assert_eq!(service.skills(), GradingSkills::NotConfigured);
    assert_eq!(service.overview(), GradingOverview::NotConfigured);
    assert_eq!(service.runs(), GradingRuns::NotConfigured);
    let run = "0f8fad5b-d9cb-469f-a165-70867728950e";
    assert_eq!(service.run_details(run), GradingRunDetails::NotConfigured);
    assert_eq!(service.open_report(run), GradingReportOpen::NotConfigured);
    assert_eq!(
        service.start("/anywhere", "claude-md-doctor"),
        GradingStart::NotConfigured
    );
    assert_eq!(service.installed(), GradingInstalledSkills::NotConfigured);
    assert_eq!(
        service.add_skill("/anywhere"),
        GradingSkillAdd::NotConfigured
    );
    assert_eq!(
        service.remove_skill("readme-check"),
        GradingSkillRemove::NotConfigured
    );
    assert!(!service.cancel("0f8fad5b-d9cb-469f-a165-70867728950e"));
    assert_eq!(service.recover(), 0);
    assert!(service.shutdown());
    assert_eq!(service.purge_with_folders(|| Ok::<_, ()>(7)), Ok(7));
}

/// The rubric of an installed test skill: one criterion the fake's GOOD
/// result fills.
fn installed_rubric(id: &str) -> String {
    format!(
        "---\nprotocol: 1\nid: {id}\nname: {id} grader\nversion: 2.0\n\
         description: Grades with {id}.\ntarget: project\n---\n\
         ## Health of the memory files (100 points)\nAward it all.\n"
    )
}

/// An installed skill under the world's home at `.claude/skills/<folder>`,
/// with a rubric of id `id` when given.
fn install(world: &World, folder: &str, id: Option<&str>) -> String {
    let dir = world.home.join(".claude/skills").join(folder);
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {folder}\ndescription: The {folder} skill.\n---\nRun it.\n"),
    )
    .unwrap();
    std::fs::write(dir.join("scripts/run.py"), "print('hi')\n").unwrap();
    if let Some(id) = id {
        std::fs::write(
            dir.join(xt_judge::rubric::RUBRIC_FILE),
            installed_rubric(id),
        )
        .unwrap();
    }
    dir.to_str().unwrap().to_owned()
}

fn installed_rows(service: &GradingService) -> Vec<(String, GradingInstalledScores)> {
    let GradingInstalledSkills::Loaded(loaded) = service.installed() else {
        panic!("installed skills not loaded");
    };
    loaded
        .skills
        .into_iter()
        .map(|row| (row.name, row.scores))
        .collect()
}

#[test]
fn installed_skills_are_listed_and_an_addable_one_becomes_a_card() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    install(&world, "claude-md-doctor", None);
    install(&world, "plain", None);
    assert_eq!(
        installed_rows(&service),
        [
            (
                "readme-check grader".to_owned(),
                GradingInstalledScores::Addable
            ),
            ("plain".to_owned(), GradingInstalledScores::NotReady),
            (
                "claude-md-doctor".to_owned(),
                GradingInstalledScores::BuiltIn
            ),
        ]
    );
    let changed = lock(&world.seen).changed;
    assert_eq!(
        service.add_skill(&readme),
        GradingSkillAdd::Added {
            skill_id: "readme-check".into()
        }
    );
    assert_eq!(lock(&world.seen).changed, changed + 1);
    assert!(installed_rows(&service).contains(&(
        "readme-check grader".to_owned(),
        GradingInstalledScores::OnScores
    )));
    // Adding it again is refused.
    assert_eq!(service.add_skill(&readme), GradingSkillAdd::AlreadyOnScores);
    let GradingSkills::Loaded { skills } = service.skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(
        skills
            .iter()
            .map(|skill| skill.id.as_str())
            .collect::<Vec<_>>(),
        ["claude-md-doctor", "readme-check"]
    );
    assert_eq!(
        skills[1].origin,
        GradingSkillOrigin::Added {
            folder: Some(readme.clone()),
            plugin: None,
        }
    );
    assert_eq!(skills[1].problem, None);
    assert!(skills[1].version.starts_with("2.0 #"));
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    assert_eq!(overview.skills.len(), 2);
    assert_eq!(overview.skills[1].skill.id, "readme-check");
    assert_eq!(overview.skills[1].rows.len(), 1);
    assert_eq!(overview.skills[1].average, None);
}

#[test]
fn only_a_folder_a_fresh_discovery_marks_addable_is_added() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let plain = install(&world, "plain", None);
    let broken = install(&world, "broken", None);
    std::fs::write(
        Path::new(&broken).join(xt_judge::rubric::RUBRIC_FILE),
        "no header",
    )
    .unwrap();
    let first = install(&world, "a-first", Some("same-check"));
    let second = install(&world, "b-second", Some("same-check"));
    let doctor = install(&world, "doctor", Some("claude-md-doctor"));
    for (folder, answer) in [
        (plain.as_str(), GradingSkillAdd::NotReady),
        (broken.as_str(), GradingSkillAdd::NotReady),
        (second.as_str(), GradingSkillAdd::IdClash),
        (doctor.as_str(), GradingSkillAdd::AlreadyOnScores),
        ("/not/a/skill", GradingSkillAdd::NotFound),
        // The same folder written another way is not what was listed.
        (&format!("{first}/"), GradingSkillAdd::NotFound),
    ] {
        assert_eq!(service.add_skill(folder), answer, "{folder}");
    }
    // A folder listed a moment ago and gone now is refused.
    let stale = install(&world, "stale", Some("stale-check"));
    std::fs::remove_dir_all(&stale).unwrap();
    assert_eq!(service.add_skill(&stale), GradingSkillAdd::NotFound);
    // A folder whose rubric broke after it was listed is refused.
    std::fs::write(Path::new(&first).join(xt_judge::rubric::RUBRIC_FILE), "x").unwrap();
    assert_eq!(service.add_skill(&first), GradingSkillAdd::NotReady);
    assert_eq!(lock(&world.seen).changed, 0);
    let GradingSkills::Loaded { skills } = service.skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(skills.len(), 1);
}

#[test]
fn removing_an_added_skill_keeps_its_grades_and_a_built_in_one_stays() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    assert!(matches!(
        service.add_skill(&readme),
        GradingSkillAdd::Added { .. }
    ));
    let run_id = started(service.start(world.project_path(), "readme-check"));
    assert_eq!(world.settle(&service, &run_id).score, Some(75));
    assert_eq!(
        service.remove_skill("claude-md-doctor"),
        GradingSkillRemove::BuiltIn
    );
    let changed = lock(&world.seen).changed;
    assert_eq!(
        service.remove_skill("readme-check"),
        GradingSkillRemove::Removed
    );
    assert_eq!(lock(&world.seen).changed, changed + 1);
    assert_eq!(
        service.remove_skill("readme-check"),
        GradingSkillRemove::NotFound
    );
    // Its grade and its files stay; it is off Scores and cannot start.
    assert_eq!(world.row(&run_id).score, Some(75));
    assert!(Path::new(&readme).join("SKILL.md").is_file());
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    assert_eq!(overview.skills.len(), 1);
    assert_eq!(
        service.start(world.project_path(), "readme-check"),
        GradingStart::UnknownSkill
    );
    // Added again, its stored grade shows again.
    assert!(matches!(
        service.add_skill(&readme),
        GradingSkillAdd::Added { .. }
    ));
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    assert_eq!(overview.skills[1].average, Some(75));
}

#[test]
fn an_added_skill_is_graded_end_to_end_from_a_staged_copy() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    service.add_skill(&readme);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(
        row.status,
        GradeStatus::Succeeded,
        "{:?}",
        row.failure_reason
    );
    assert_eq!(row.skill_id, "readme-check");
    assert!(row.skill_version.starts_with("2.0 #"));
    assert_eq!(
        files_under(&world.grades.join(&run_id)),
        ["card.svg", "report.html", "result.json"]
    );
    // Claude Code was given the staged copy, never the installed folder.
    let record = world.record();
    let argv: Vec<&str> = record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    let prompt = argv[argv.iter().position(|arg| *arg == "-p").unwrap() + 1];
    let staged = world.staging_root(&run_id).join("skill");
    assert!(prompt.contains(&format!(
        "Read the rubric at {}",
        staged.join("xtrace-rubric.md").display()
    )));
    assert!(prompt.contains("Run it."));
    assert!(!argv.iter().any(|arg| arg.contains(&readme)));
    assert!(!world.staging_root(&run_id).exists());
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    let card = &overview.skills[1];
    assert_eq!((card.average, card.graded, card.total), (Some(75), 1, 1));
    assert_eq!(card.top_insights[0].title, "Dead path");
}

#[test]
fn an_added_skill_whose_folder_or_rubric_goes_keeps_its_card_and_starts_nothing() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    service.add_skill(&readme);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    world.settle(&service, &run_id);
    let rubric = Path::new(&readme).join(xt_judge::rubric::RUBRIC_FILE);
    let moved = world.root.join("readme-moved");
    let cases: [(&dyn Fn(), GradingSkillProblem); 4] = [
        (
            &|| std::fs::write(&rubric, "no header").unwrap(),
            GradingSkillProblem::RubricInvalid {
                rubric: crate::grading_dto::GradingRubricProblem::NoHeader,
            },
        ),
        (
            &|| std::fs::write(&rubric, installed_rubric("other-check")).unwrap(),
            GradingSkillProblem::RubricIdChanged,
        ),
        (
            &|| std::fs::remove_file(&rubric).unwrap(),
            GradingSkillProblem::RubricMissing,
        ),
        (
            &|| {
                std::fs::write(&rubric, installed_rubric("readme-check")).unwrap();
                std::fs::rename(&readme, &moved).unwrap();
                std::os::unix::fs::symlink(&moved, &readme).unwrap();
            },
            GradingSkillProblem::FolderMissing,
        ),
    ];
    for (change, problem) in cases {
        change();
        let GradingOverview::Loaded(overview) = service.overview() else {
            panic!("overview not loaded");
        };
        let card = &overview.skills[1];
        assert_eq!(card.skill.problem, Some(problem));
        assert_eq!(card.skill.name, "readme-check grader");
        assert_eq!(card.average, Some(75), "{problem:?}");
        let rows = world.rows();
        assert_eq!(
            service.start(world.project_path(), "readme-check"),
            GradingStart::SkillUnavailable,
            "{problem:?}"
        );
        assert_eq!(world.rows(), rows);
    }
    std::fs::remove_file(&readme).unwrap();
    let GradingSkills::Loaded { skills } = service.skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(skills[1].problem, Some(GradingSkillProblem::FolderMissing));
}

#[test]
fn an_added_skill_past_the_copy_limits_or_with_an_outside_link_fails_with_its_reason() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    service.add_skill(&readme);
    let outside = world.root.join("outside.txt");
    std::fs::write(&outside, "x").unwrap();
    std::os::unix::fs::symlink(&outside, Path::new(&readme).join("notes.txt")).unwrap();
    let run_id = started(service.start(world.project_path(), "readme-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.failure_reason.as_deref(), Some("skill_link_refused"));
    std::fs::remove_file(Path::new(&readme).join("notes.txt")).unwrap();
    let many = Path::new(&readme).join("many");
    std::fs::create_dir(&many).unwrap();
    for index in 0..xt_judge::staging::MAX_INSTALLED_SKILL_FILES {
        std::fs::write(many.join(format!("{index}.md")), "").unwrap();
    }
    let run_id = started(service.start(world.project_path(), "readme-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.failure_reason.as_deref(), Some("skill_too_large"));
    assert!(!world.record.exists(), "the fake never ran");
    assert!(!world.staging_root(&run_id).exists());
}

/// A Claude Code plugin `tools@market`, installed for the user at
/// `cache/market/tools/<version>` with one skill `lint` whose rubric has the
/// id `lint-check`, and turned on (or off).
fn install_plugin(world: &World, version: &str, enabled: bool) -> PathBuf {
    let plugins = world.home.join(".claude/plugins");
    let install = plugins.join("cache/market/tools").join(version);
    let skill = install.join("skills/lint");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: lint\ndescription: Lints.\n---\nRun it.\n",
    )
    .unwrap();
    std::fs::write(
        skill.join(xt_judge::rubric::RUBRIC_FILE),
        installed_rubric("lint-check"),
    )
    .unwrap();
    std::fs::write(
        plugins.join("installed_plugins.json"),
        format!(
            r#"{{"version": 2, "plugins": {{"tools@market": [{{"scope": "user", "installPath": "{}", "version": "{version}"}}]}}}}"#,
            install.display()
        ),
    )
    .unwrap();
    std::fs::write(
        world.home.join(".claude/settings.json"),
        format!(r#"{{"enabledPlugins": {{"tools@market": {enabled}}}}}"#),
    )
    .unwrap();
    skill
}

fn added_card(service: &GradingService) -> GradingSkillInfo {
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    overview.skills[1].skill.clone()
}

#[test]
fn a_plugin_skill_is_added_by_its_plugin_and_keeps_working_after_an_update() {
    let world = World::new(WRITE_GOOD);
    let service = world.service();
    let first = install_plugin(&world, "1.0.0", true);
    let GradingInstalledSkills::Loaded(loaded) = service.installed() else {
        panic!("installed skills not loaded");
    };
    assert_eq!(loaded.skills[0].location, "plugin:tools@market/lint");
    assert_eq!(loaded.skills[0].folder, first.to_str().unwrap());
    // Adding by the versioned folder is not what was listed.
    assert_eq!(
        service.add_skill(first.to_str().unwrap()),
        GradingSkillAdd::NotFound
    );
    assert_eq!(
        service.add_skill("plugin:tools@market/lint"),
        GradingSkillAdd::Added {
            skill_id: "lint-check".into()
        }
    );
    let card = added_card(&service);
    assert_eq!(
        (card.origin, card.problem),
        (
            GradingSkillOrigin::Added {
                folder: Some(first.to_str().unwrap().to_owned()),
                plugin: Some("tools".into()),
            },
            None
        )
    );
    // The plugin updates into a new folder and the old one goes.
    let second = install_plugin(&world, "1.1.0", true);
    std::fs::remove_dir_all(world.home.join(".claude/plugins/cache/market/tools/1.0.0")).unwrap();
    let card = added_card(&service);
    assert_eq!(card.problem, None);
    assert_eq!(
        card.origin,
        GradingSkillOrigin::Added {
            folder: Some(second.to_str().unwrap().to_owned()),
            plugin: Some("tools".into()),
        }
    );
    assert_eq!(
        installed_rows(&service),
        [(
            "lint-check grader".to_owned(),
            GradingInstalledScores::OnScores
        )]
    );
    let run_id = started(service.start(world.project_path(), "lint-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(
        row.status,
        GradeStatus::Succeeded,
        "{:?}",
        row.failure_reason
    );
    // Turned off, then uninstalled: the card stays with its grade and says
    // why; Grade starts nothing.
    install_plugin(&world, "1.1.0", false);
    for step in ["off", "uninstalled"] {
        if step == "uninstalled" {
            std::fs::write(
                world.home.join(".claude/plugins/installed_plugins.json"),
                r#"{"version": 2, "plugins": {}}"#,
            )
            .unwrap();
        }
        let card = added_card(&service);
        assert_eq!(
            card.problem,
            Some(GradingSkillProblem::PluginUnavailable),
            "{step}"
        );
        assert_eq!(
            card.origin,
            GradingSkillOrigin::Added {
                folder: None,
                plugin: Some("tools".into()),
            },
            "{step}"
        );
        assert_eq!(card.name, "lint-check grader");
        let GradingOverview::Loaded(overview) = service.overview() else {
            panic!("overview not loaded");
        };
        assert_eq!(overview.skills[1].average, Some(75), "{step}");
        let rows = world.rows();
        assert_eq!(
            service.start(world.project_path(), "lint-check"),
            GradingStart::SkillUnavailable,
            "{step}"
        );
        assert_eq!(world.rows(), rows);
    }
}

#[test]
fn a_rubric_changed_between_the_start_and_the_copy_fails_as_skill_changed() {
    let world = World::new(WRITE_GOOD);
    let readme = install(&world, "readme", Some("readme-check"));
    let rubric = Path::new(&readme).join(xt_judge::rubric::RUBRIC_FILE);
    let mut config = world.config();
    let edit = rubric.clone();
    config.before_staging = Some(Arc::new(move || {
        let text = std::fs::read_to_string(&edit).unwrap();
        std::fs::write(&edit, text.replace("Award it all.", "Award all of it.")).unwrap();
    }));
    let service = world.service_with(config);
    service.add_skill(&readme);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Failed);
    assert_eq!(row.failure_reason.as_deref(), Some("skill_changed"));
    assert!(!world.record.exists(), "the fake never ran");
    assert!(!world.staging_root(&run_id).exists());
    assert!(
        std::fs::read_to_string(&rubric)
            .unwrap()
            .contains("Award all of it.")
    );
    assert_eq!(
        GradingFailure::from_code("skill_changed"),
        GradingFailure::SkillChanged
    );
}

#[test]
fn an_added_skill_cannot_be_removed_while_it_grades() {
    let world = World::new(HANG);
    let service = world.service();
    let readme = install(&world, "readme", Some("readme-check"));
    service.add_skill(&readme);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    world.wait_ready();
    let changed = lock(&world.seen).changed;
    assert_eq!(
        service.remove_skill("readme-check"),
        GradingSkillRemove::Running
    );
    assert_eq!(lock(&world.seen).changed, changed);
    let GradingSkills::Loaded { skills } = service.skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(skills.len(), 2);
    assert!(service.cancel(&run_id));
    world.settle(&service, &run_id);
    assert_eq!(
        service.remove_skill("readme-check"),
        GradingSkillRemove::Removed
    );
}

/// Holds the app's store from another thread until `release` is sent, as
/// indexing can; returns once it holds it.
fn hold_store(world: &World) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (release, held) = mpsc::channel::<()>();
    let (holding, holds) = mpsc::channel::<()>();
    let state = Arc::clone(&world.state);
    let holder = std::thread::spawn(move || {
        state
            .with_store(|_| {
                holding.send(()).unwrap();
                let _ = held.recv();
                Ok(())
            })
            .unwrap();
    });
    holds.recv().unwrap();
    (release, holder)
}

#[test]
fn cancel_never_waits_on_the_store_while_a_removal_or_start_does() {
    let world = World::new(HANG);
    let service = Arc::new(world.service());
    let readme = install(&world, "readme", Some("readme-check"));
    let other = install(&world, "other", Some("other-check"));
    service.add_skill(&readme);
    service.add_skill(&other);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    world.wait_ready();
    let (release, holder) = hold_store(&world);
    // A removal of another skill and a start each wait for the store now.
    let removing = std::thread::spawn({
        let service = Arc::clone(&service);
        move || service.remove_skill("other-check")
    });
    let starting = std::thread::spawn({
        let service = Arc::clone(&service);
        let project = world.project_path().to_owned();
        move || service.start(&project, "readme-check")
    });
    std::thread::sleep(Duration::from_millis(200));
    // Cancel answers at once all the same.
    let begun = Instant::now();
    assert!(service.cancel(&run_id));
    assert!(
        begun.elapsed() < Duration::from_millis(500),
        "cancel waited {:?}",
        begun.elapsed()
    );
    assert!(service.active_run().is_some());
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(removing.join().unwrap(), GradingSkillRemove::Removed);
    let row = world.settle(&service, &run_id);
    assert_eq!(row.status, GradeStatus::Cancelled);
    // The waiting start found the slot taken, or free once the run ended.
    match starting.join().unwrap() {
        GradingStart::Busy => {}
        GradingStart::Started { run_id } => {
            world.wait_ready();
            assert!(service.cancel(&run_id));
            world.settle(&service, &run_id);
        }
        other => panic!("unexpected start: {other:?}"),
    }
}

#[test]
fn a_skill_removed_while_its_start_waits_never_starts() {
    let world = World::new(WRITE_GOOD);
    let service = Arc::new(world.service());
    let readme = install(&world, "readme", Some("readme-check"));
    service.add_skill(&readme);
    // The removal holds `membership` across its delete while the store is
    // held; a start of that skill meanwhile must not run it.
    let (release, holder) = hold_store(&world);
    let removing = std::thread::spawn({
        let service = Arc::clone(&service);
        move || service.remove_skill("readme-check")
    });
    std::thread::sleep(Duration::from_millis(100));
    let starting = std::thread::spawn({
        let service = Arc::clone(&service);
        let project = world.project_path().to_owned();
        move || service.start(&project, "readme-check")
    });
    std::thread::sleep(Duration::from_millis(100));
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(removing.join().unwrap(), GradingSkillRemove::Removed);
    assert_eq!(starting.join().unwrap(), GradingStart::UnknownSkill);
    assert_eq!(world.rows(), 0);
    assert!(service.active_run().is_none());
}

// ---- Making a rubric -------------------------------------------------------

/// A rubric the fake's rubric-making run writes: one criterion the fake's
/// GOOD grading result fills.
const MADE: &str = "---\nprotocol: 1\nid: readme-check\nname: README Check\nversion: 1\n\
     description: How well the README explains the project.\ntarget: project\n---\n\
     ## Health of the memory files (100 points)\nAward it all.\n";

/// The fake: a rubric-making run appends its prompt and its folders to
/// `<root>/making`, then runs `maker` (Python, `OUT` is its output folder,
/// `TRY` that folder's name, `TARGET` the staged copy); a grading run does
/// `grading`.
fn maker_world(maker: &str, grading: &str) -> World {
    let indent = |body: &str| {
        body.lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    World::new(&format!(
        "MADE = {MADE:?}\n\
         if sys.argv[2].startswith('XTrace rubric-making run'):\n\
         \x20   OUT = G\n    TRY = os.path.basename(OUT)\n    TARGET = os.getcwd()\n\
         \x20   with open(os.path.join(MARKS, 'making'), 'a') as f:\n\
         \x20       f.write(json.dumps({{'prompt': sys.argv[2], 'out': OUT, 'cwd': TARGET, 'argv': sys.argv}}) + '\\n')\n\
         {maker}\n\
         else:\n\
         \x20   SKILL_DIR = sys.argv[-1]\n\
         \x20   open(os.path.join(MARKS, 'staged-rubric'), 'w').write(open(os.path.join(SKILL_DIR, 'xtrace-rubric.md')).read() if os.path.exists(os.path.join(SKILL_DIR, 'xtrace-rubric.md')) else '')\n\
         {grading}\n",
        maker = indent(maker),
        grading = indent(grading),
    ))
}

impl World {
    /// Each rubric-making try's record, in order.
    fn makings(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.root.join("making"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Waits until no job holds the slot and returns the location's row.
    fn settle_job(&self, service: &GradingService, location: &str) -> GradingInstalledSkill {
        let until = Instant::now() + Duration::from_secs(60);
        while service.active_run().is_some() {
            assert!(Instant::now() < until, "the job did not finish");
            std::thread::sleep(Duration::from_millis(20));
        }
        installed_row(service, location)
    }
}

fn installed_row(service: &GradingService, location: &str) -> GradingInstalledSkill {
    let GradingInstalledSkills::Loaded(loaded) = service.installed() else {
        panic!("installed skills not loaded");
    };
    loaded
        .skills
        .into_iter()
        .find(|row| row.location == location)
        .unwrap_or_else(|| panic!("{location} is not listed"))
}

fn job_started(answer: GradingRubricMake) -> String {
    match answer {
        GradingRubricMake::Started { job_id } => job_id,
        other => panic!("not started: {other:?}"),
    }
}

fn made_criteria(row: &GradingInstalledSkill) -> (Vec<(String, u8)>, bool) {
    match &row.rubric_job {
        Some(GradingRubricJob::Made {
            criteria,
            kept_by_xtrace,
            ..
        }) => (
            criteria
                .iter()
                .map(|criterion| (criterion.name.clone(), criterion.points))
                .collect(),
            *kept_by_xtrace,
        ),
        other => panic!("not made: {other:?}"),
    }
}

fn job_failure(row: &GradingInstalledSkill) -> GradingRubricFailure {
    match &row.rubric_job {
        Some(GradingRubricJob::Failed { failure, .. }) => *failure,
        other => panic!("not failed: {other:?}"),
    }
}

fn rubric_at(folder: &Path) -> PathBuf {
    folder.join(xt_judge::rubric::RUBRIC_FILE)
}

#[test]
fn a_made_rubric_is_saved_next_to_the_skill_and_the_skill_becomes_ready() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    let row = installed_row(&service, &readme);
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Make));
    assert_eq!(row.rubric_job, None);
    let changed = lock(&world.seen).changed;
    let job_id = job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    // Saved next to SKILL.md with exactly the made bytes, readable like the
    // skill's other files; nothing else was written there.
    let saved = rubric_at(Path::new(&readme));
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), MADE);
    assert_eq!(
        std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let mut names: Vec<String> = std::fs::read_dir(&readme)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["SKILL.md", "scripts", "xtrace-rubric.md"]);
    // The row is ready to add now, with a preview of its criteria.
    assert_eq!(
        row.rubric,
        GradingInstalledRubric::Ready {
            id: "readme-check".into()
        }
    );
    assert_eq!(row.scores, GradingInstalledScores::Addable);
    assert_eq!(row.rubric_action, None);
    assert!(!row.rubric_kept_by_xtrace);
    assert_eq!(
        made_criteria(&row),
        (vec![("Health of the memory files".to_owned(), 100)], false)
    );
    // The job's staging folder is gone; its start, the save and its end
    // were published, and its progress reported.
    assert!(
        !xt_judge::maker::maker_parent(&world.temp, &xt_judge::maker::data_key(&world.data))
            .join(&job_id)
            .exists()
    );
    assert!(lock(&world.seen).changed >= changed + 3);
    let seen = lock(&world.seen);
    let first = seen.rubric_progress.first().unwrap();
    assert_eq!(
        (first.state, first.location.as_str()),
        (GradingProgressState::Running, readme.as_str())
    );
    assert_eq!(
        seen.rubric_progress.last().unwrap().state,
        GradingProgressState::Finished
    );
    assert!(
        seen.rubric_progress
            .iter()
            .all(|report| report.job_id == job_id)
    );
    drop(seen);
    // It can be added and grades end to end with the made rubric.
    assert!(matches!(
        service.add_skill(&readme),
        GradingSkillAdd::Added { .. }
    ));
    let run_id = started(service.start(world.project_path(), "readme-check"));
    assert_eq!(
        world.settle(&service, &run_id).status,
        GradeStatus::Succeeded
    );
    // A skill that is ready is not made again.
    assert_eq!(
        service.make_rubric(&readme),
        GradingRubricMake::AlreadyReady
    );
}

/// The rubric-making run is contained like a grading: the staged copy of the
/// skill is its read-only working folder, the output folder the only
/// writable one, the staged helper the skill folder, and the prompt is the
/// XTrace header then the helper's own instructions.
#[test]
fn the_rubric_making_run_has_the_gradings_command_line_over_its_own_folders() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    let job_id = job_started(service.make_rubric(&readme));
    world.settle_job(&service, &readme);
    let making = world.makings();
    assert_eq!(making.len(), 1);
    let argv: Vec<String> = making[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_owned())
        .collect();
    let root = std::fs::canonicalize(xt_judge::maker::maker_parent(
        &world.temp,
        &xt_judge::maker::data_key(&world.data),
    ))
    .unwrap()
    .join(&job_id);
    let (target, out, helper) = (root.join("target"), root.join("out"), root.join("helper"));
    let (target, out, helper) = (
        target.to_str().unwrap(),
        out.to_str().unwrap(),
        helper.to_str().unwrap(),
    );
    assert_eq!(making[0]["cwd"], target);
    assert_eq!(making[0]["out"], out);
    let settings: serde_json::Value = serde_json::from_str(&argv[10]).unwrap();
    assert_eq!(
        settings["sandbox"]["filesystem"],
        serde_json::json!({"denyWrite": [target], "allowWrite": [out]})
    );
    assert_eq!(
        settings["sandbox"]["network"]["allowedDomains"],
        serde_json::json!([])
    );
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
    assert_eq!(settings["disableAllHooks"], true);
    assert_eq!(
        argv[3..],
        [
            "--tools",
            "Read,Write,Edit,Bash,Glob,Grep",
            "--permission-mode",
            "dontAsk",
            "--setting-sources",
            "",
            "--settings",
            argv[10].as_str(),
            "--strict-mcp-config",
            "--no-session-persistence",
            "--allowedTools",
            &format!("Edit(/{out}/**)"),
            "--disallowedTools",
            &format!("Edit(/{target}/**)"),
            "--add-dir",
            out,
            helper,
        ]
    );
    let prompt = &argv[2];
    assert!(prompt.starts_with(&format!(
        "XTrace rubric-making run (headless). TARGET = {target}. OUT = {out}."
    )));
    assert!(prompt.contains("Write exactly one file"));
    assert!(prompt.contains("# XTrace rubric maker"));
    // The real skill folder was never the run's folder.
    assert!(!argv.iter().any(|arg| arg.contains(&readme)));
}

#[test]
fn an_invalid_rubric_is_retried_once_with_the_reason_and_a_second_failure_is_reported() {
    let short = "---\\nprotocol: 1\\nid: readme-check\\nname: R\\nversion: 1\\n\
                 description: d\\ntarget: project\\n---\\n## All (90 points)\\nAll.\\n";
    let world = maker_world(&format!("write('xtrace-rubric.md', '{short}')"), WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(
        job_failure(&row),
        GradingRubricFailure::RubricInvalid {
            rubric: GradingRubricProblem::Total
        }
    );
    let making = world.makings();
    assert_eq!(making.len(), 2);
    let retry = making[1]["prompt"].as_str().unwrap();
    assert!(
        retry.contains("the rubric's points do not add up to 100"),
        "{retry}"
    );
    assert!(making[1]["out"].as_str().unwrap().ends_with("/out-2"));
    // Nothing was saved; the skill still needs a rubric and can try again.
    assert!(!rubric_at(Path::new(&readme)).exists());
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Make));

    // A first try that is refused and a second that is valid: saved.
    let world = maker_world(
        &format!("write('xtrace-rubric.md', '{short}' if TRY == 'out' else MADE)"),
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    world.settle_job(&service, &readme);
    assert_eq!(world.makings().len(), 2);
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&readme))).unwrap(),
        MADE
    );
}

#[test]
fn remaking_a_rubric_with_a_problem_keeps_the_old_one_as_a_backup() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    std::fs::write(rubric_at(Path::new(&readme)), "no header").unwrap();
    std::fs::write(Path::new(&readme).join("xtrace-rubric.md.bak"), "older").unwrap();
    let row = installed_row(&service, &readme);
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Remake));
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert!(matches!(
        row.rubric_job,
        Some(GradingRubricJob::Made { .. })
    ));
    // The old rubric states no version: the remade one is version 2.
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&readme))).unwrap(),
        MADE.replace("version: 1\n", "version: 2\n")
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&readme).join("xtrace-rubric.md.bak")).unwrap(),
        "no header"
    );
    // The prompt said why the old one was refused, and a copy of it was in
    // the staged skill.
    let prompt = world.makings()[0]["prompt"].as_str().unwrap().to_owned();
    assert!(prompt.contains(
        "(a copy is in TARGET) is refused by XTrace: the rubric does not start with a header"
    ));
}

#[test]
fn a_rubric_that_appears_while_the_first_one_is_made_is_never_overwritten() {
    let world = maker_world(
        "open(os.path.join(MARKS, 'home/.claude/skills/readme/xtrace-rubric.md'), 'w').write('mine')\n\
         write('xtrace-rubric.md', MADE)",
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(job_failure(&row), GradingRubricFailure::SkillChanged);
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&readme))).unwrap(),
        "mine"
    );
    assert!(!Path::new(&readme).join("xtrace-rubric.md.bak").exists());
}

#[test]
fn a_folder_that_cannot_be_written_gets_its_rubric_kept_by_xtrace() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    std::fs::set_permissions(&readme, std::fs::Permissions::from_mode(0o555)).unwrap();
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    std::fs::set_permissions(&readme, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(made_criteria(&row).1);
    assert!(row.rubric_kept_by_xtrace);
    assert_eq!(row.scores, GradingInstalledScores::Addable);
    assert!(!rubric_at(Path::new(&readme)).exists());
    let kept = xt_judge::rubric_store::RubricStore::new(world.data.join("rubrics"))
        .file(&xt_judge::installed::Location::Folder(PathBuf::from(
            &readme,
        )))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), MADE);
    assert_eq!(
        std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // A folder that cannot be written and has its own refused rubric cannot
    // be remade: its own one would still win.
    let broken = install(&world, "broken", None);
    std::fs::write(rubric_at(Path::new(&broken)), "no header").unwrap();
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o555)).unwrap();
    job_started(service.make_rubric(&broken));
    let row = world.settle_job(&service, &broken);
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(job_failure(&row), GradingRubricFailure::FolderNotWritable);
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&broken))).unwrap(),
        "no header"
    );
}

/// The plugin `tools@market` without a rubric, as `install_plugin` makes it.
fn install_bare_plugin(world: &World, version: &str) -> PathBuf {
    let skill = install_plugin(world, version, true);
    std::fs::remove_file(rubric_at(&skill)).unwrap();
    skill
}

#[test]
fn a_plugin_skills_rubric_is_kept_by_xtrace_survives_an_update_and_is_staged_for_grading() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let first = install_bare_plugin(&world, "1.0.0");
    let location = "plugin:tools@market/lint";
    job_started(service.make_rubric(location));
    let row = world.settle_job(&service, location);
    assert!(made_criteria(&row).1);
    assert!(row.rubric_kept_by_xtrace);
    // The plugin's own folder is untouched.
    assert!(!rubric_at(&first).exists());
    let kept = world
        .data
        .join("rubrics")
        .join(xt_judge::rubric_store::store_key(location))
        .join("xtrace-rubric.md");
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), MADE);
    // An update installs into a new folder: the kept rubric still applies.
    let second = install_bare_plugin(&world, "2.0.0");
    std::fs::remove_dir_all(world.home.join(".claude/plugins/cache/market/tools/1.0.0")).unwrap();
    let row = installed_row(&service, location);
    assert_eq!(row.folder, second.to_str().unwrap());
    assert_eq!(row.scores, GradingInstalledScores::Addable);
    assert!(row.rubric_kept_by_xtrace);
    // Added and graded: the run's staged copy carries the kept rubric.
    assert!(matches!(
        service.add_skill(location),
        GradingSkillAdd::Added { .. }
    ));
    let GradingSkills::Loaded { skills } = service.skills() else {
        panic!("skills not loaded");
    };
    assert_eq!(skills[1].problem, None);
    let run_id = started(service.start(world.project_path(), "readme-check"));
    let row = world.settle(&service, &run_id);
    assert_eq!(
        row.status,
        GradeStatus::Succeeded,
        "{:?}",
        row.failure_reason
    );
    assert_eq!(
        std::fs::read_to_string(world.root.join("staged-rubric")).unwrap(),
        MADE
    );
    assert!(row.skill_version.starts_with("1 #"));
    assert!(!rubric_at(&second).exists());
}

#[test]
fn a_made_id_another_skill_has_gets_a_free_suffix_and_an_added_skill_keeps_its_own() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    // Another installed skill is ready with the id the maker writes.
    install(&world, "other", Some("readme-check"));
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(
        row.rubric,
        GradingInstalledRubric::Ready {
            id: "readme-check-2".into()
        }
    );
    assert_eq!(row.scores, GradingInstalledScores::Addable);
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&readme))).unwrap(),
        MADE.replace("id: readme-check\n", "id: readme-check-2\n")
    );
    // A built-in skill's id is taken too.
    let world = maker_world(
        "write('xtrace-rubric.md', MADE.replace('id: readme-check', 'id: claude-md-doctor'))",
        WRITE_GOOD,
    );
    let service = world.service();
    let doctor = install(&world, "my-doctor", None);
    job_started(service.make_rubric(&doctor));
    assert_eq!(
        world.settle_job(&service, &doctor).rubric,
        GradingInstalledRubric::Ready {
            id: "claude-md-doctor-2".into()
        }
    );
    // An added skill whose rubric broke is made again with its card's id.
    let world = maker_world(
        "write('xtrace-rubric.md', MADE.replace('id: readme-check', 'id: something-else'))",
        WRITE_GOOD,
    );
    let service = world.service();
    let lint = install(&world, "lint", Some("lint-check"));
    assert!(matches!(
        service.add_skill(&lint),
        GradingSkillAdd::Added { .. }
    ));
    std::fs::write(rubric_at(Path::new(&lint)), "no header").unwrap();
    assert!(added_card(&service).problem.is_some());
    job_started(service.make_rubric(&lint));
    let row = world.settle_job(&service, &lint);
    assert_eq!(
        row.rubric,
        GradingInstalledRubric::Ready {
            id: "lint-check".into()
        }
    );
    assert_eq!(row.scores, GradingInstalledScores::OnScores);
    assert_eq!(added_card(&service).problem, None);
}

#[test]
fn a_skill_reached_through_a_link_gets_its_rubric_in_the_real_folder() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    // `~/.claude` is a link to a folder elsewhere.
    let real = world.root.join("real-claude");
    std::fs::create_dir_all(real.join("skills/readme")).unwrap();
    std::fs::write(
        real.join("skills/readme/SKILL.md"),
        "---\nname: readme\ndescription: Checks READMEs.\n---\nRun it.\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, world.home.join(".claude")).unwrap();
    let listed = world.home.join(".claude/skills/readme");
    let location = listed.to_str().unwrap();
    job_started(service.make_rubric(location));
    let row = world.settle_job(&service, location);
    assert!(matches!(
        row.rubric_job,
        Some(GradingRubricJob::Made { .. })
    ));
    assert!(
        std::fs::symlink_metadata(real.join("skills/readme/xtrace-rubric.md"))
            .unwrap()
            .file_type()
            .is_file()
    );
    // The staged copy came from the real folder.
    assert!(
        world.makings()[0]["cwd"]
            .as_str()
            .unwrap()
            .ends_with("/target")
    );
}

#[test]
fn cancel_and_quitting_end_a_rubric_job_and_save_nothing() {
    let world = maker_world(HANG, HANG);
    let service = world.service();
    let readme = install(&world, "readme", None);
    let job_id = job_started(service.make_rubric(&readme));
    world.wait_ready();
    let pids = world.pids();
    assert!(service.cancel(&job_id));
    let row = world.settle_job(&service, &readme);
    assert_eq!(
        row.rubric_job,
        Some(GradingRubricJob::Cancelled {
            job_id: job_id.clone()
        })
    );
    assert!(all_gone(&pids), "{pids:?}");
    assert!(!rubric_at(Path::new(&readme)).exists());
    assert_eq!(world.makings().len(), 1, "a cancelled job is not retried");
    // Quitting cancels a job the same way, within the bound.
    std::fs::remove_file(&world.ready).unwrap();
    let job_id = job_started(service.make_rubric(&readme));
    world.wait_ready();
    let pids = world.pids();
    assert!(service.shutdown());
    assert!(service.active_run().is_none());
    assert!(all_gone(&pids), "{pids:?}");
    assert!(
        !xt_judge::maker::maker_parent(&world.temp, &xt_judge::maker::data_key(&world.data))
            .join(&job_id)
            .exists()
    );
    assert_eq!(service.make_rubric(&readme), GradingRubricMake::Stopped);
}

#[test]
fn a_rubric_job_and_a_grading_never_run_together() {
    let world = maker_world(HANG, HANG);
    let service = world.service();
    let readme = install(&world, "readme", None);
    // A grading is going: no rubric job starts.
    let run_id = started(service.start(world.project_path(), "claude-md-doctor"));
    world.wait_ready();
    assert_eq!(service.make_rubric(&readme), GradingRubricMake::Busy);
    assert!(service.cancel(&run_id));
    world.settle(&service, &run_id);
    // A rubric job is going: no grading starts, and the Scores page's answer
    // names the job.
    std::fs::remove_file(&world.ready).unwrap();
    let job_id = job_started(service.make_rubric(&readme));
    world.wait_ready();
    let rows = world.rows();
    assert_eq!(
        service.start(world.project_path(), "claude-md-doctor"),
        GradingStart::Busy
    );
    assert_eq!(world.rows(), rows, "nothing was saved for a refused start");
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    let making = overview.making_rubric.unwrap();
    assert_eq!(
        (
            making.job_id.as_str(),
            making.location.as_str(),
            making.name.as_str()
        ),
        (job_id.as_str(), readme.as_str(), "readme")
    );
    assert_eq!(overview.running, None);
    assert_eq!(service.make_rubric(&readme), GradingRubricMake::Busy);
    assert!(matches!(
        installed_row(&service, &readme).rubric_job,
        Some(GradingRubricJob::Running { .. })
    ));
    assert!(service.cancel(&job_id));
    world.settle_job(&service, &readme);
    let GradingOverview::Loaded(overview) = service.overview() else {
        panic!("overview not loaded");
    };
    assert_eq!(overview.making_rubric, None);
}

#[test]
fn rubric_requests_that_cannot_start_have_typed_answers() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    assert_eq!(
        service.make_rubric("/not/a/skill"),
        GradingRubricMake::NotFound
    );
    let readme = install(&world, "readme", None);
    let mut config = world.config();
    config.maker_dir = world.root.join("no-helper");
    assert_eq!(
        world.service_with(config).make_rubric(&readme),
        GradingRubricMake::HelperUnavailable
    );
    let mut config = world.config();
    config.environment = RunEnvironment::Fixed(world.environment(&world.root.join("no-claude")));
    assert_eq!(
        world.service_with(config).make_rubric(&readme),
        GradingRubricMake::ClaudeUnavailable
    );
    assert_eq!(
        GradingService::not_configured().make_rubric(&readme),
        GradingRubricMake::NotConfigured
    );
    assert!(world.makings().is_empty());
}

#[test]
fn startup_stops_and_removes_this_apps_leftover_rubric_jobs() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let key = xt_judge::maker::data_key(&world.data);
    let readme = install(&world, "readme", None);
    let staging = xt_judge::maker::MakerStaging::create(
        &world.temp,
        &key,
        "0f8fad5b-d9cb-469f-a165-70867728950e",
        Path::new(&readme),
        &world.config().maker_dir,
        None,
    )
    .unwrap();
    let root = staging.root().to_path_buf();
    // The app crashed: the folder and its process stay behind.
    std::mem::forget(staging);
    let mut sleeper = std::process::Command::new("/bin/sleep");
    sleeper.arg("600").stdin(std::process::Stdio::null());
    let mut child = xt_probes::process::spawn_grouped(&mut sleeper).unwrap();
    let leader = xt_probes::leader::GroupLeader::observe(child.id()).unwrap();
    xt_judge::staging::write_private(&root.join("leader"), leader.record().as_bytes()).unwrap();
    assert_eq!(world.service().recover(), 1);
    assert!(!root.exists());
    child.wait().unwrap();
    assert!(!alive(&child.id().to_string()));
}

#[test]
fn no_rubric_is_made_for_a_skill_scores_already_has_under_another_one() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    // A copy of the built-in skill, by its name.
    let doctor = install(&world, "claude-md-doctor", None);
    let row = installed_row(&service, &doctor);
    assert_eq!(row.scores, GradingInstalledScores::BuiltIn);
    assert_eq!(row.rubric_action, None);
    assert_eq!(service.make_rubric(&doctor), GradingRubricMake::NotOffered);
    // A skill named like an added skill's id, from elsewhere; its rubric
    // has a problem.
    let lint = install(&world, "lint", Some("lint-check"));
    assert!(matches!(
        service.add_skill(&lint),
        GradingSkillAdd::Added { .. }
    ));
    let copy = install(&world, "lint-check", None);
    std::fs::write(rubric_at(Path::new(&copy)), "no header").unwrap();
    let row = installed_row(&service, &copy);
    assert!(matches!(
        row.scores,
        GradingInstalledScores::IdOnScores { .. }
    ));
    assert_eq!(row.rubric_action, None);
    assert_eq!(service.make_rubric(&copy), GradingRubricMake::NotOffered);
    // The added skill itself, once its rubric breaks, is offered a remake.
    std::fs::write(rubric_at(Path::new(&lint)), "no header").unwrap();
    let row = installed_row(&service, &lint);
    assert_eq!(row.scores, GradingInstalledScores::OnScores);
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Remake));
    assert!(world.makings().is_empty());
    assert!(!rubric_at(Path::new(&doctor)).exists());
}

#[test]
fn a_refusal_saves_nothing_is_not_retried_and_shows_its_reason() {
    let world = maker_world(
        "write('xtrace-refusal.md', 'This skill publishes releases; it does not review a project.')",
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    let job_id = job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(
        row.rubric_job,
        Some(GradingRubricJob::Refused {
            job_id,
            reason: "This skill publishes releases; it does not review a project.".into(),
        })
    );
    assert_eq!(world.makings().len(), 1, "a refusal is not retried");
    assert!(!rubric_at(Path::new(&readme)).exists());
    assert!(
        !world.data.join("rubrics").exists(),
        "nothing is kept by XTrace either"
    );
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Make));
    // Once the skill has a valid rubric of its own, the refusal is not shown.
    std::fs::write(rubric_at(Path::new(&readme)), MADE).unwrap();
    let row = installed_row(&service, &readme);
    assert_eq!(row.rubric_job, None);

    // Both files, twice: a failure with its own reason.
    let world = maker_world(
        "write('xtrace-rubric.md', MADE)\nwrite('xtrace-refusal.md', 'No.')",
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(job_failure(&row), GradingRubricFailure::BothWritten);
    assert_eq!(world.makings().len(), 2);
    assert!(!rubric_at(Path::new(&readme)).exists());
}

#[test]
fn a_finished_outcome_shows_only_while_it_still_matches_the_rubric() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert!(matches!(
        row.rubric_job,
        Some(GradingRubricJob::Made { .. })
    ));
    // The made rubric is gone again: its outcome is not shown.
    std::fs::remove_file(rubric_at(Path::new(&readme))).unwrap();
    let row = installed_row(&service, &readme);
    assert_eq!(row.rubric, GradingInstalledRubric::Missing);
    assert_eq!(row.rubric_job, None);

    // A failure is shown until the skill gets a valid rubric.
    let world = maker_world("pass", WRITE_GOOD);
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(job_failure(&row), GradingRubricFailure::RubricMissing);
    std::fs::write(rubric_at(Path::new(&readme)), MADE).unwrap();
    assert_eq!(installed_row(&service, &readme).rubric_job, None);
}

/// A rubric file that is a link out of the skill's folder, or to nothing,
/// is left out of the staged copy and remade; the link itself becomes the
/// backup.
#[test]
fn a_rubric_that_is_a_link_out_of_the_folder_or_to_nothing_can_be_remade() {
    let world = maker_world(
        "open(os.path.join(MARKS, 'target-rubric'), 'w').write(str(os.path.lexists(os.path.join(TARGET, 'xtrace-rubric.md'))))\n\
         write('xtrace-rubric.md', MADE)",
        WRITE_GOOD,
    );
    let service = world.service();
    let outside = world.root.join("outside.md");
    std::fs::write(&outside, installed_rubric("outside")).unwrap();
    for (name, link_to) in [
        ("linked", outside.clone()),
        ("dangling", world.root.join("gone")),
    ] {
        let skill = install(&world, name, None);
        std::os::unix::fs::symlink(&link_to, rubric_at(Path::new(&skill))).unwrap();
        let row = installed_row(&service, &skill);
        assert_eq!(
            row.rubric_action,
            Some(GradingRubricAction::Remake),
            "{name}"
        );
        job_started(service.make_rubric(&skill));
        let row = world.settle_job(&service, &skill);
        assert!(
            matches!(row.rubric_job, Some(GradingRubricJob::Made { .. })),
            "{name}: {:?}",
            row.rubric_job
        );
        // The staged copy had no rubric entry, and the prompt said so.
        assert_eq!(
            std::fs::read_to_string(world.root.join("target-rubric")).unwrap(),
            "False"
        );
        let prompt = world.makings().last().unwrap()["prompt"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(prompt.contains("It is not in TARGET."), "{name}");
        let saved = std::fs::symlink_metadata(rubric_at(Path::new(&skill))).unwrap();
        assert!(saved.file_type().is_file(), "{name}");
        let backup = Path::new(&skill).join("xtrace-rubric.md.bak");
        assert!(
            std::fs::symlink_metadata(&backup)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
    // What the link pointed at is untouched.
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        installed_rubric("outside")
    );
}

/// A broken rubric XTrace keeps for a plugin's skill is placed in the staged
/// copy, so the prompt's "a copy is in TARGET" is true, and the remade one
/// replaces it in the store.
#[test]
fn a_kept_broken_rubric_is_in_the_staged_copy_when_it_is_remade() {
    let world = maker_world(
        "open(os.path.join(MARKS, 'target-rubric'), 'w').write(open(os.path.join(TARGET, 'xtrace-rubric.md')).read())\n\
         write('xtrace-rubric.md', MADE)",
        WRITE_GOOD,
    );
    let service = world.service();
    install_bare_plugin(&world, "1.0.0");
    let location = "plugin:tools@market/lint";
    let store = xt_judge::rubric_store::RubricStore::new(world.data.join("rubrics"));
    let kept = store
        .file(&xt_judge::installed::Location::parse(location).unwrap())
        .unwrap();
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, "---\nversion: 7\n---\nbroken").unwrap();
    let row = installed_row(&service, location);
    assert!(row.rubric_kept_by_xtrace);
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Remake));
    job_started(service.make_rubric(location));
    let row = world.settle_job(&service, location);
    assert!(matches!(
        row.rubric_job,
        Some(GradingRubricJob::Made { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(world.root.join("target-rubric")).unwrap(),
        "---\nversion: 7\n---\nbroken"
    );
    assert!(
        world.makings()[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("(a copy is in TARGET) is refused by XTrace")
    );
    // Remade in the store, one version on.
    assert_eq!(
        std::fs::read_to_string(&kept).unwrap(),
        MADE.replace("version: 1\n", "version: 8\n")
    );
}

/// A remade rubric gets the next version, so the card and the grades made
/// with it show a changed version; the retry's prompt keeps both reasons.
#[test]
fn a_remade_rubric_gets_the_next_version_and_grades_show_it() {
    let short = "---\\nprotocol: 1\\nid: readme-check\\nname: R\\nversion: 1\\n\
                 description: d\\ntarget: project\\n---\\n## All (90 points)\\nAll.\\n";
    let world = maker_world(
        &format!("write('xtrace-rubric.md', '{short}' if TRY == 'out' else MADE)"),
        WRITE_GOOD,
    );
    let service = world.service();
    let lint = install(&world, "lint", Some("lint-check"));
    assert!(matches!(
        service.add_skill(&lint),
        GradingSkillAdd::Added { .. }
    ));
    let run_id = started(service.start(world.project_path(), "lint-check"));
    let before = world.settle(&service, &run_id).skill_version;
    assert!(before.starts_with("2.0 #"), "{before}");
    // The rubric breaks but still says version 3.
    std::fs::write(
        rubric_at(Path::new(&lint)),
        installed_rubric("lint-check")
            .replace("version: 2.0", "version: 3")
            .replace("(100 points)", "(90 points)"),
    )
    .unwrap();
    job_started(service.make_rubric(&lint));
    world.settle_job(&service, &lint);
    let making = world.makings();
    assert_eq!(making.len(), 2);
    let retry = making[1]["prompt"].as_str().unwrap();
    assert!(
        retry.contains(
            "The skill's current xtrace-rubric.md (a copy is in TARGET) is refused by XTrace: \
             the rubric's points do not add up to 100."
        ),
        "{retry}"
    );
    assert!(retry.contains(
        "A previous try of this run wrote a rubric XTrace refused: the rubric's points do not \
         add up to 100."
    ));
    assert_eq!(
        std::fs::read_to_string(rubric_at(Path::new(&lint))).unwrap(),
        MADE.replace("id: readme-check\n", "id: lint-check\n")
            .replace("version: 1\n", "version: 4\n")
    );
    assert!(added_card(&service).version.starts_with("4 #"));
    let run_id = started(service.start(world.project_path(), "lint-check"));
    let after = world.settle(&service, &run_id);
    assert_eq!(after.status, GradeStatus::Succeeded);
    assert!(
        after.skill_version.starts_with("4 #"),
        "{}",
        after.skill_version
    );
    assert_ne!(after.skill_version, before);
    // Not a plain whole number: version 2.
    std::fs::write(rubric_at(Path::new(&lint)), "---\nversion: 4.1\n---\n").unwrap();
    job_started(service.make_rubric(&lint));
    world.settle_job(&service, &lint);
    assert!(
        std::fs::read_to_string(rubric_at(Path::new(&lint)))
            .unwrap()
            .contains("\nversion: 2\n")
    );
}

/// A cancel that arrives after the run but before the save wins: nothing is
/// saved.
#[test]
fn a_job_cancelled_right_before_its_save_saves_nothing() {
    let world = maker_world("write('xtrace-rubric.md', MADE)", WRITE_GOOD);
    let (reached, at_save) = mpsc::channel::<()>();
    let (go, wait) = mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let mut config = world.config();
    config.before_save = Some(Arc::new(move || {
        reached.send(()).unwrap();
        lock(&wait).recv().unwrap();
    }));
    let service = world.service_with(config);
    let readme = install(&world, "readme", None);
    let job_id = job_started(service.make_rubric(&readme));
    at_save.recv_timeout(Duration::from_secs(60)).unwrap();
    assert!(service.cancel(&job_id));
    go.send(()).unwrap();
    let row = world.settle_job(&service, &readme);
    assert_eq!(row.rubric_job, Some(GradingRubricJob::Cancelled { job_id }));
    assert!(!rubric_at(Path::new(&readme)).exists());
}

/// A refusal is kept across a restart, in `<data>/rubric-refusals.json`
/// with the skill's fingerprint; it is listed under "can't be scored" only
/// while the skill is unchanged, and Try again drops it for good.
#[test]
fn a_refusal_survives_a_restart_until_the_skill_changes_or_it_is_tried_again() {
    use crate::grading_dto::GradingInstalledGroup;
    // The first try refuses; every later one writes nothing.
    let world = maker_world(
        "if not os.path.exists(os.path.join(MARKS, 'refused-once')):\n\
         \x20   open(os.path.join(MARKS, 'refused-once'), 'w').write('')\n\
         \x20   write('xtrace-refusal.md', 'This skill publishes releases; it does not review a project.')",
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    let job_id = job_started(service.make_rubric(&readme));
    let refused = GradingRubricJob::Refused {
        job_id: job_id.clone(),
        reason: "This skill publishes releases; it does not review a project.".into(),
    };
    let row = world.settle_job(&service, &readme);
    assert_eq!(row.rubric_job.as_ref(), Some(&refused));
    assert_eq!(row.group, GradingInstalledGroup::CantScore);
    assert_eq!(row.rubric_action, Some(GradingRubricAction::Make));
    let file = world.data.join(xt_judge::refusals::REFUSALS_FILE);
    let kept = xt_judge::refusals::RefusalStore::new(file.clone()).read();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (kept[0].location.as_str(), kept[0].job_id.as_str()),
        (readme.as_str(), job_id.as_str())
    );
    assert_eq!(
        Some(kept[0].fingerprint.as_str()),
        xt_judge::refusals::fingerprint(Path::new(&readme)).as_deref()
    );
    drop(service);

    // After a restart: the same refusal, read from the file.
    let service = world.service();
    let row = installed_row(&service, &readme);
    assert_eq!(row.rubric_job.as_ref(), Some(&refused));
    assert_eq!(row.group, GradingInstalledGroup::CantScore);
    let GradingInstalledSkills::Loaded(loaded) = service.installed() else {
        panic!("installed skills not loaded");
    };
    assert_eq!((loaded.cant_score, loaded.yours), (1, 0));

    // The skill was edited: the refusal no longer applies (it stays in the
    // file); put back as it was, it applies again.
    let skill_md = Path::new(&readme).join("SKILL.md");
    let original = std::fs::read_to_string(&skill_md).unwrap();
    std::fs::write(&skill_md, original.replace("Run it.", "Run it twice.")).unwrap();
    let row = installed_row(&service, &readme);
    assert_eq!(
        (row.rubric_job, row.group),
        (None, GradingInstalledGroup::Yours)
    );
    std::fs::write(&skill_md, &original).unwrap();
    assert_eq!(
        installed_row(&service, &readme).group,
        GradingInstalledGroup::CantScore
    );
    // A new file in the skill changes it too.
    std::fs::write(Path::new(&readme).join("scripts/more.py"), "").unwrap();
    assert_eq!(
        installed_row(&service, &readme).group,
        GradingInstalledGroup::Yours
    );
    std::fs::remove_file(Path::new(&readme).join("scripts/more.py")).unwrap();
    assert_eq!(
        installed_row(&service, &readme).group,
        GradingInstalledGroup::CantScore
    );

    // Try again: the kept refusal is dropped as the job starts; this try
    // fails, which shows among "your skills".
    job_started(service.make_rubric(&readme));
    assert!(
        xt_judge::refusals::RefusalStore::new(file.clone())
            .read()
            .is_empty()
    );
    let row = world.settle_job(&service, &readme);
    assert_eq!(job_failure(&row), GradingRubricFailure::RubricMissing);
    assert_eq!(row.group, GradingInstalledGroup::Yours);
    drop(service);
    // After another restart nothing about it is kept.
    let service = world.service();
    let row = installed_row(&service, &readme);
    assert_eq!(
        (row.rubric_job, row.group),
        (None, GradingInstalledGroup::Yours)
    );
}

/// "Delete stored content" also removes the kept refusals: in memory, from
/// their file, and so after a restart; a failed purge keeps them.
#[test]
fn a_content_purge_removes_the_kept_refusals() {
    use crate::grading_dto::GradingInstalledGroup;
    let world = maker_world(
        "write('xtrace-refusal.md', 'This skill publishes releases; it does not review a project.')",
        WRITE_GOOD,
    );
    let service = world.service();
    let readme = install(&world, "readme", None);
    job_started(service.make_rubric(&readme));
    let row = world.settle_job(&service, &readme);
    assert_eq!(row.group, GradingInstalledGroup::CantScore);
    let file = world.data.join(xt_judge::refusals::REFUSALS_FILE);
    assert!(file.is_file());
    // A failed purge keeps them.
    let failed: Result<(), ()> = service.purge_with_folders(|| Err(()));
    assert!(failed.is_err());
    assert!(file.is_file());
    assert_eq!(
        installed_row(&service, &readme).group,
        GradingInstalledGroup::CantScore
    );
    service
        .purge_with_folders(|| world.state.purge_stored_content(|| ()))
        .unwrap();
    assert!(!file.exists());
    let row = installed_row(&service, &readme);
    assert_eq!(
        (row.rubric_job, row.group),
        (None, GradingInstalledGroup::Yours)
    );
    drop(service);
    let service = world.service();
    assert_eq!(
        installed_row(&service, &readme).group,
        GradingInstalledGroup::Yours
    );
}
