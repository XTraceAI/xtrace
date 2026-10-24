//! The run details rule over seeded disposable stores and grade folders,
//! and the synthetic answers the renderer's tests and acceptance pages read
//! (`ui/src/app/scores/details.synthetic.json`, byte-checked here).
use super::*;
use crate::grading_dto::{GradingInsight, GradingRunDetails, GradingSeverity};

/// A kept run's content; panics on deleted content.
fn kept(details: &GradingRunDetailsLoaded) -> &GradingRunKept {
    match &details.content {
        GradingRunContent::Kept(kept) => kept,
        GradingRunContent::Deleted => panic!("content was deleted"),
    }
}

/// Listed findings; panics on unreadable ones.
fn listed(kept: &GradingRunKept) -> &[GradingInsight] {
    match &kept.findings {
        GradingRunFindings::Listed { findings } => findings,
        GradingRunFindings::Unreadable => panic!("findings are unreadable"),
    }
}
use serde::Serialize;
use xt_store::{
    Store,
    grades::{GradeFinish, NewGradeRun},
};

const SKILL: &str = "claude-md-doctor";
/// 2026-10-05 09:00 UTC, as the Scores page's synthetic answers.
const T0: i64 = 1_791_190_800_000;
const MINUTE: i64 = 60_000;

/// A disposable store, its grades folder, and a counter that names each run
/// as the Scores page's synthetic answers do.
struct Seed {
    _dir: tempfile::TempDir,
    root: PathBuf,
    grades: PathBuf,
    store: Store,
    runs: u32,
}

/// One seeded succeeded run.
struct Run<'a> {
    project: &'a str,
    score: u8,
    complete: bool,
    summary: &'a str,
    insights: &'a str,
    /// The `criteria` the kept `result.json` reports, if any.
    criteria: Option<&'a str>,
    /// Whether the run wrote a report.
    report: bool,
}

impl Seed {
    fn new() -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("details-")
            .tempdir_in(&parent)
            .unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let grades = root.join("app data").join("grades");
        std::fs::create_dir_all(&grades).unwrap();
        let store = Store::open(root.join("grades.sqlite")).unwrap();
        Self {
            _dir: dir,
            root,
            grades,
            store,
            runs: 0,
        }
    }

    /// Store `run` as succeeded at minute `at` and keep its folder as a run
    /// keeps it: the validated `result.json` and the report.
    fn succeeded(&mut self, at: i64, run: &Run<'_>) -> String {
        self.runs += 1;
        let run_id = format!("00000000-0000-4000-8000-{:012x}", self.runs);
        let started_at_ms = T0 + at * MINUTE;
        self.store
            .grade_insert_running(&NewGradeRun {
                run_id: &run_id,
                project_path: run.project,
                skill_id: SKILL,
                skill_version: "0.5.1+rubric.1 #3fa2b9c1d0e4",
                started_at_ms,
            })
            .unwrap();
        let report = run.report.then_some("report.html");
        assert!(
            self.store
                .grade_finish(
                    &run_id,
                    &GradeFinish::Succeeded {
                        finished_at_ms: started_at_ms + MINUTE,
                        score: run.score,
                        complete: run.complete,
                        summary: run.summary,
                        insights_json: run.insights,
                        report_file: report,
                    },
                )
                .unwrap()
        );
        let folder = self.grades.join(&run_id);
        std::fs::create_dir_all(&folder).unwrap();
        if run.report {
            std::fs::write(folder.join("report.html"), "<p>report</p>").unwrap();
        }
        let mut result = serde_json::json!({
            "protocol": 1,
            "score": run.score,
            "summary": run.summary,
            "insights": serde_json::from_str::<serde_json::Value>(run.insights).unwrap(),
            "report": report,
            "complete": run.complete,
        });
        if let Some(criteria) = run.criteria {
            result["criteria"] = serde_json::from_str(criteria).unwrap();
        }
        std::fs::write(
            folder.join(protocol::RESULT_FILE),
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
        run_id
    }

    /// Store findings that do not read for a run whose kept result is
    /// otherwise good: its kept `result.json` lists no findings.
    fn odd_findings(&mut self, at: i64, run: Run<'_>, stored: &str) -> String {
        let run_id = self.succeeded(
            at,
            &Run {
                insights: stored,
                ..run
            },
        );
        let path = self.grades.join(&run_id).join(protocol::RESULT_FILE);
        let mut result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        result["insights"] = serde_json::json!([]);
        std::fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
        run_id
    }

    fn row(&self, run_id: &str) -> GradeRow {
        self.store.grade(run_id).unwrap().unwrap()
    }

    fn kept(&self, run_id: &str) -> Option<KeptFolder> {
        read_kept(&self.grades, &self.row(run_id))
    }

    fn details(&self, run_id: &str) -> GradingRunDetailsLoaded {
        details(&self.row(run_id), self.kept(run_id).as_ref()).unwrap()
    }

    fn purge(&mut self) {
        self.store
            .purge_content(&xt_store::retention::ContentRegistry::default())
            .unwrap();
    }
}

const ATLAS: &str = "/work/atlas";
const BEACON: &str = "/work/beacon";
const CEDAR: &str = "/work/cedar";

/// Five findings in the order the skill wrote them, not by weight.
const ATLAS_FINDINGS: &str = r#"[
  {"severity": "warning", "title": "CLAUDE.md is longer than 200 lines",
   "detail": "CLAUDE.md has 342 lines. Claude Code loads all of it into every conversation, so the rules near the end get less attention.",
   "fix": "Move the deployment and release sections into docs/ and link them from CLAUDE.md."},
  {"severity": "pass", "title": "Commands run",
   "detail": "Every build and test command in CLAUDE.md ran in this project."},
  {"severity": "critical", "title": "AGENTS.md is not loaded",
   "detail": "AGENTS.md holds the review rules, but nothing imports it and Claude Code does not read it by itself.",
   "fix": "Add @AGENTS.md to CLAUDE.md, or move its rules into CLAUDE.md."},
  {"severity": "info", "title": "No hooks are set up"},
  {"severity": "critical", "title": "A named script does not exist",
   "detail": "CLAUDE.md says to run scripts/check.sh before a commit; the project has no such file.",
   "fix": "Name the command the project uses now: pnpm check."}
]"#;

/// Three criteria worth 100 points, awarded 75.
const ATLAS_CRITERIA: &str = r#"[
  {"name": "Instructions reach the agent", "max": 30, "awarded": 15},
  {"name": "Instructions are accurate", "max": 40, "awarded": 35},
  {"name": "Instructions are short and clear", "max": 30, "awarded": 25}
]"#;

fn atlas() -> Run<'static> {
    Run {
        project: ATLAS,
        score: 75,
        complete: true,
        summary: "The instructions are mostly accurate, but AGENTS.md never reaches the agent and one named script is missing.",
        insights: ATLAS_FINDINGS,
        criteria: Some(ATLAS_CRITERIA),
        report: true,
    }
}

/// A result written before rubrics: no criteria.
fn beacon() -> Run<'static> {
    Run {
        project: BEACON,
        score: 90,
        complete: true,
        summary: "Short, accurate instructions.",
        insights: r#"[{"severity": "warning", "title": "CLAUDE.md is longer than 200 lines", "fix": "Trim the examples."},
                      {"severity": "info", "title": "No hooks are set up"}]"#,
        criteria: None,
        report: true,
    }
}

fn cedar() -> Run<'static> {
    Run {
        project: CEDAR,
        score: 61,
        complete: false,
        summary: "Some files could not be read, so this score may miss something.",
        insights: r#"[{"severity": "warning", "title": "CLAUDE.md is longer than 200 lines"},
                      {"severity": "warning", "title": "Two rules disagree", "detail": "CLAUDE.md says to use npm; AGENTS.md says pnpm."}]"#,
        criteria: Some(
            r#"[{"name": "Instructions reach the agent", "max": 50, "awarded": 31},
                {"name": "Instructions are accurate", "max": 50, "awarded": 30}]"#,
        ),
        report: true,
    }
}

fn titles(findings: &[GradingInsight]) -> Vec<(GradingSeverity, &str)> {
    findings
        .iter()
        .map(|finding| (finding.severity, finding.title.as_str()))
        .collect()
}

#[test]
fn a_kept_run_shows_every_finding_most_severe_first_its_criteria_and_its_report() {
    let mut seed = Seed::new();
    let run_id = seed.succeeded(0, &atlas());
    let details = seed.details(&run_id);
    assert_eq!(details.run_id, run_id);
    assert_eq!((details.score, details.complete), (75, true));
    assert_eq!(details.graded_at_ms, T0 + MINUTE);
    let got = kept(&details);
    assert_eq!(got.summary, atlas().summary);
    let findings = listed(got);
    // All five, by weight; each weight keeps the run's own order.
    assert_eq!(
        titles(findings),
        [
            (GradingSeverity::Critical, "AGENTS.md is not loaded"),
            (GradingSeverity::Critical, "A named script does not exist"),
            (
                GradingSeverity::Warning,
                "CLAUDE.md is longer than 200 lines"
            ),
            (GradingSeverity::Info, "No hooks are set up"),
            (GradingSeverity::Pass, "Commands run"),
        ]
    );
    assert_eq!(
        findings[0].fix.as_deref(),
        Some("Add @AGENTS.md to CLAUDE.md, or move its rules into CLAUDE.md.")
    );
    assert_eq!(findings[3].detail, None);
    let GradingRunCriteria::Reported { criteria } = &got.criteria else {
        panic!("criteria are reported: {:?}", got.criteria);
    };
    assert_eq!(
        criteria
            .iter()
            .map(|criterion| (criterion.name.as_str(), criterion.awarded, criterion.max))
            .collect::<Vec<_>>(),
        [
            ("Instructions reach the agent", 15, 30),
            ("Instructions are accurate", 35, 40),
            ("Instructions are short and clear", 25, 30),
        ]
    );
    assert_eq!(got.report, GradingRunReport::Kept);
    // The file opened is the checked report inside the run's own folder.
    let row = seed.row(&run_id);
    let folder = seed.kept(&run_id).unwrap();
    assert_eq!(
        report_target(&row, &folder),
        Ok(seed.grades.join(&run_id).join("report.html").as_path())
    );
}

#[test]
fn a_result_without_criteria_or_without_a_report_says_it_did_not_report_them() {
    let mut seed = Seed::new();
    let old = seed.succeeded(0, &beacon());
    let details = seed.details(&old);
    let got = kept(&details);
    assert_eq!(
        got.criteria,
        GradingRunCriteria::Unavailable {
            reason: GradingRunGap::NotReported
        }
    );
    assert_eq!(got.report, GradingRunReport::Kept);
    let bare = seed.succeeded(
        1,
        &Run {
            report: false,
            ..atlas()
        },
    );
    let details = seed.details(&bare);
    let got = kept(&details);
    assert!(matches!(got.criteria, GradingRunCriteria::Reported { .. }));
    assert_eq!(
        got.report,
        GradingRunReport::Unavailable {
            reason: GradingRunGap::NotReported
        }
    );
    // A run with no report says so even when its folder is gone.
    std::fs::remove_dir_all(seed.grades.join(&bare)).unwrap();
    let details = seed.details(&bare);
    let got = kept(&details);
    assert_eq!(
        (got.criteria.clone(), got.report),
        (
            GradingRunCriteria::Unavailable {
                reason: GradingRunGap::FolderNotKept
            },
            GradingRunReport::Unavailable {
                reason: GradingRunGap::NotReported
            }
        )
    );
}

#[test]
fn a_pruned_folder_keeps_the_stored_summary_and_findings() {
    let mut seed = Seed::new();
    let run_id = seed.succeeded(0, &cedar());
    std::fs::remove_dir_all(seed.grades.join(&run_id)).unwrap();
    assert_eq!(seed.kept(&run_id), Some(KeptFolder::Missing));
    let details = seed.details(&run_id);
    assert!(!details.complete);
    let got = kept(&details);
    assert_eq!(got.summary, cedar().summary);
    assert_eq!(listed(got).len(), 2);
    let gone = GradingRunGap::FolderNotKept;
    assert_eq!(
        got.criteria,
        GradingRunCriteria::Unavailable { reason: gone }
    );
    assert_eq!(got.report, GradingRunReport::Unavailable { reason: gone });
}

#[test]
fn deleted_content_shows_the_score_alone_and_reads_no_folder() {
    let mut seed = Seed::new();
    let run_id = seed.succeeded(0, &atlas());
    seed.purge();
    // The folder is still there (the service removes it after the purge);
    // nothing in it is read.
    assert!(seed.grades.join(&run_id).is_dir());
    assert_eq!(seed.kept(&run_id), None);
    let got = seed.details(&run_id);
    assert_eq!((got.score, got.complete), (75, true));
    assert_eq!(got.content, GradingRunContent::Deleted);
    assert_eq!(
        serde_json::to_value(&got.content).unwrap(),
        serde_json::json!({"state": "deleted"})
    );
}

#[test]
fn stored_findings_that_do_not_read_are_unreadable_not_none() {
    let mut seed = Seed::new();
    // Valid JSON the store accepts, but not findings.
    let run_id = seed.odd_findings(0, atlas(), "[1]");
    let details = seed.details(&run_id);
    let got = kept(&details);
    assert_eq!(got.findings, GradingRunFindings::Unreadable);
    // The rest of the run shows as usual.
    assert_eq!(got.summary, atlas().summary);
    assert_eq!(got.report, GradingRunReport::Kept);
    // A run that stored no findings lists none.
    let none = seed.succeeded(
        1,
        &Run {
            insights: "[]",
            ..beacon()
        },
    );
    assert!(listed(kept(&seed.details(&none))).is_empty());
}

#[cfg(unix)]
#[test]
fn a_folder_that_does_not_read_or_match_the_stored_run_is_unreadable() {
    let mut seed = Seed::new();
    let unreadable = |seed: &Seed, run_id: &str| {
        assert_eq!(seed.kept(run_id), Some(KeptFolder::Unreadable), "{run_id}");
        let details = seed.details(run_id);
        let got = kept(&details);
        let reason = GradingRunGap::Unreadable;
        assert_eq!(got.criteria, GradingRunCriteria::Unavailable { reason });
        assert_eq!(got.report, GradingRunReport::Unavailable { reason });
        // The stored summary and findings still show.
        assert!(!got.summary.is_empty() && !listed(got).is_empty());
    };
    // The run's folder is a link to a good copy elsewhere.
    let linked = seed.succeeded(0, &atlas());
    let elsewhere = seed.root.join("elsewhere");
    std::fs::rename(seed.grades.join(&linked), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, seed.grades.join(&linked)).unwrap();
    unreadable(&seed, &linked);
    // Not JSON.
    let broken = seed.succeeded(1, &atlas());
    std::fs::write(seed.grades.join(&broken).join("result.json"), "{").unwrap();
    unreadable(&seed, &broken);
    // Another score than the stored one.
    let other = seed.succeeded(2, &atlas());
    let path = seed.grades.join(&other).join("result.json");
    let mut result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    result["score"] = 80.into();
    result["criteria"][0]["awarded"] = 20.into();
    std::fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
    unreadable(&seed, &other);
    // The report replaced by a link out of the folder.
    let outside = seed.succeeded(3, &atlas());
    let report = seed.grades.join(&outside).join("report.html");
    std::fs::remove_file(&report).unwrap();
    std::fs::write(seed.root.join("outside.html"), "<p>").unwrap();
    std::os::unix::fs::symlink(seed.root.join("outside.html"), &report).unwrap();
    unreadable(&seed, &outside);
    // The report gone from a kept folder.
    let missing = seed.succeeded(4, &atlas());
    std::fs::remove_file(seed.grades.join(&missing).join("report.html")).unwrap();
    unreadable(&seed, &missing);
}

#[test]
fn a_run_that_did_not_succeed_has_no_details() {
    let mut seed = Seed::new();
    let run_id = "00000000-0000-4000-8000-0000000000f1";
    seed.store
        .grade_insert_running(&NewGradeRun {
            run_id,
            project_path: ATLAS,
            skill_id: SKILL,
            skill_version: "1",
            started_at_ms: T0,
        })
        .unwrap();
    seed.store
        .grade_finish(
            run_id,
            &GradeFinish::Failed {
                finished_at_ms: T0 + MINUTE,
                reason: "timeout",
            },
        )
        .unwrap();
    let row = seed.row(run_id);
    assert_eq!(details(&row, read_kept(&seed.grades, &row).as_ref()), None);
}

/// The answers the renderer's tests and acceptance pages read, one per
/// dialog state. Run IDs, projects and scores are those of the Scores page's
/// `graded` answer (`scores.synthetic.json`); the texts are made up.
#[derive(Serialize)]
struct Export {
    full: GradingRunDetails,
    no_criteria: GradingRunDetails,
    folder_not_kept: GradingRunDetails,
    content_deleted: GradingRunDetails,
    unreadable: GradingRunDetails,
    findings_unreadable: GradingRunDetails,
    not_found: GradingRunDetails,
    failed: GradingRunDetails,
    not_configured: GradingRunDetails,
}

const EXPORT: &str = "../ui/src/app/scores/details.synthetic.json";

fn export() -> Export {
    let loaded = |seed: &Seed, run_id: &str| GradingRunDetails::Loaded(seed.details(run_id));
    // atlas 75, beacon 90 (from before rubrics), cedar 61 (partial, pruned).
    let mut seed = Seed::new();
    let atlas_id = seed.succeeded(0, &atlas());
    let beacon_id = seed.succeeded(10, &beacon());
    let cedar_id = seed.succeeded(20, &cedar());
    std::fs::remove_dir_all(seed.grades.join(&cedar_id)).unwrap();
    let full = loaded(&seed, &atlas_id);
    let no_criteria = loaded(&seed, &beacon_id);
    let folder_not_kept = loaded(&seed, &cedar_id);
    let mut broken = Seed::new();
    let broken_id = broken.succeeded(0, &atlas());
    std::fs::write(broken.grades.join(&broken_id).join("result.json"), "{").unwrap();
    let unreadable = loaded(&broken, &broken_id);
    let odd_id = broken.odd_findings(
        10,
        beacon(),
        r#"[{"severity": "urgent", "title": "Not a finding"}]"#,
    );
    let findings_unreadable = loaded(&broken, &odd_id);
    seed.purge();
    let content_deleted = loaded(&seed, &atlas_id);
    Export {
        full,
        no_criteria,
        folder_not_kept,
        content_deleted,
        unreadable,
        findings_unreadable,
        not_found: GradingRunDetails::NotFound,
        failed: GradingRunDetails::Failed,
        not_configured: GradingRunDetails::NotConfigured,
    }
}

/// The committed export is exactly what the rule gives now. Regenerate it
/// with `XTRACE_WRITE_SCORES_SYNTHETIC=1`.
#[test]
fn the_synthetic_ui_export_is_what_the_rule_gives() {
    let export = export();
    let json = serde_json::to_string_pretty(&export).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPORT);
    if std::env::var_os("XTRACE_WRITE_SCORES_SYNTHETIC").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &json).unwrap();
    }
    assert_eq!(
        json,
        std::fs::read_to_string(&path).unwrap_or_default(),
        "regenerate with XTRACE_WRITE_SCORES_SYNTHETIC=1"
    );
    // What the renderer's acceptance relies on, stated here so a changed
    // seed cannot quietly remove a case.
    let loaded = |answer: &GradingRunDetails| match answer {
        GradingRunDetails::Loaded(loaded) => loaded.clone(),
        other => panic!("not loaded: {other:?}"),
    };
    let full = loaded(&export.full);
    assert_eq!(
        (full.run_id.as_str(), full.score),
        ("00000000-0000-4000-8000-000000000001", 75)
    );
    assert_eq!(listed(kept(&full)).len(), 5);
    assert!(matches!(
        kept(&full).criteria,
        GradingRunCriteria::Reported { .. }
    ));
    assert_eq!(kept(&full).report, GradingRunReport::Kept);
    let no_criteria = loaded(&export.no_criteria);
    assert_eq!(no_criteria.run_id, "00000000-0000-4000-8000-000000000002");
    assert_eq!(
        kept(&no_criteria).criteria,
        GradingRunCriteria::Unavailable {
            reason: GradingRunGap::NotReported
        }
    );
    let pruned = loaded(&export.folder_not_kept);
    assert_eq!(
        (pruned.run_id.as_str(), pruned.complete),
        ("00000000-0000-4000-8000-000000000003", false)
    );
    assert_eq!(
        kept(&pruned).report,
        GradingRunReport::Unavailable {
            reason: GradingRunGap::FolderNotKept
        }
    );
    let deleted = loaded(&export.content_deleted);
    assert_eq!(deleted.content, GradingRunContent::Deleted);
    let odd = loaded(&export.findings_unreadable);
    assert_eq!(kept(&odd).findings, GradingRunFindings::Unreadable);
    let unreadable = loaded(&export.unreadable);
    assert_eq!(
        kept(&unreadable).report,
        GradingRunReport::Unavailable {
            reason: GradingRunGap::Unreadable
        }
    );
}
