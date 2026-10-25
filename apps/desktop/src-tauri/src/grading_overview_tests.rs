//! The Scores page's rule over a seeded disposable store, and the synthetic
//! answers the renderer's tests and acceptance pages read
//! (`ui/src/app/scores/scores.synthetic.json`, byte-checked here).
use super::*;
use crate::{
    grading_dto::{GradingOverview, GradingRubricProblem, GradingSkillOrigin, GradingSkillProblem},
    projects_dto::ProjectSourceId,
};
use serde::Serialize;
use std::path::Path;
use xt_store::{
    Store,
    grades::{GradeFinish, NewGradeRun},
};

const SKILL: &str = "claude-md-doctor";
/// 2026-10-05 09:00 UTC; each seeded minute below counts from here.
const T0: i64 = 1_791_190_800_000;
const MINUTE: i64 = 60_000;

fn skills() -> Vec<GradingSkillInfo> {
    vec![GradingSkillInfo {
        id: SKILL.into(),
        name: "CLAUDE.md doctor".into(),
        version: "0.5.1+xtrace.1".into(),
        description: "Checks how well this project's CLAUDE.md and AGENTS.md guide coding agents."
            .into(),
        origin: GradingSkillOrigin::BuiltIn,
        problem: None,
    }]
}

const ADDED: &str = "readme-check";

/// The built-in skill and one added skill, which can grade or cannot.
fn with_added(problem: Option<GradingSkillProblem>) -> Vec<GradingSkillInfo> {
    let mut all = skills();
    all.push(GradingSkillInfo {
        id: ADDED.into(),
        name: "README check".into(),
        version: "1.0 #3fa2b9c1d0e4".into(),
        description: "Checks that a project's README explains how to install and run it.".into(),
        origin: GradingSkillOrigin::Added {
            folder: Some("/home/you/.claude/skills/readme-check".into()),
            plugin: None,
        },
        problem,
    });
    all
}

fn project(name: &str, sources: &[ProjectSourceId]) -> ProjectEntry {
    ProjectEntry {
        name: name.into(),
        path: format!("/work/{name}"),
        sources: sources.to_vec(),
    }
}

/// A disposable store and a counter that names each run.
struct Seed {
    _dir: tempfile::TempDir,
    store: Store,
    runs: u32,
}

/// How a seeded run ends.
enum End<'a> {
    Succeeded {
        score: u8,
        complete: bool,
        insights: &'a str,
    },
    Failed(&'a str),
    Cancelled,
    Interrupted,
    Running,
}

impl Seed {
    fn new() -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("scores-")
            .tempdir_in(&parent)
            .unwrap();
        let store = Store::open(dir.path().join("grades.sqlite")).unwrap();
        Self {
            _dir: dir,
            store,
            runs: 0,
        }
    }

    /// A run of `skill` on `path` started at minute `at`, ended a minute later.
    fn run_of(&mut self, skill: &str, path: &str, at: i64, end: End<'_>) -> String {
        self.runs += 1;
        let run_id = format!("00000000-0000-4000-8000-{:012x}", self.runs);
        let started_at_ms = T0 + at * MINUTE;
        self.store
            .grade_insert_running(&NewGradeRun {
                run_id: &run_id,
                project_path: path,
                skill_id: skill,
                skill_version: "0.5.1+xtrace.1",
                started_at_ms,
            })
            .unwrap();
        let finished_at_ms = started_at_ms + MINUTE;
        let finish = match end {
            End::Running => return run_id,
            End::Succeeded {
                score,
                complete,
                insights,
            } => GradeFinish::Succeeded {
                finished_at_ms,
                score,
                complete,
                summary: "A summary.",
                insights_json: insights,
                report_file: Some("report.html"),
            },
            End::Failed(reason) => GradeFinish::Failed {
                finished_at_ms,
                reason,
            },
            End::Cancelled => GradeFinish::Cancelled { finished_at_ms },
            End::Interrupted => GradeFinish::Interrupted {
                finished_at_ms,
                reason: "app_closed",
            },
        };
        assert!(self.store.grade_finish(&run_id, &finish).unwrap());
        run_id
    }

    fn run(&mut self, path: &str, at: i64, end: End<'_>) -> String {
        self.run_of(SKILL, path, at, end)
    }

    fn overview(&self, projects: &[ProjectEntry]) -> GradingOverviewLoaded {
        self.overview_with(projects, &skills())
    }

    fn overview_with(
        &self,
        projects: &[ProjectEntry],
        skills: &[GradingSkillInfo],
    ) -> GradingOverviewLoaded {
        self.overview_line(projects, skills, &[])
    }

    fn overview_line(
        &self,
        projects: &[ProjectEntry],
        skills: &[GradingSkillInfo],
        line: &[Waiting],
    ) -> GradingOverviewLoaded {
        let read = self.store.grade_pairs(&pairs(projects, skills)).unwrap();
        overview(projects.to_vec(), skills, &read, line).unwrap()
    }
}

/// A grading waiting in line for `skill` on `path`.
fn queued(run_id: &str, path: &str, skill: &str) -> Waiting {
    Waiting {
        run_id: run_id.into(),
        project_path: path.into(),
        skill_id: skill.into(),
        skill_version: "0.5.1+xtrace.1".into(),
    }
}

fn scored(score: u8) -> End<'static> {
    End::Succeeded {
        score,
        complete: true,
        insights: "[]",
    }
}

#[test]
fn the_average_is_the_rounded_mean_with_halves_rounded_up() {
    assert_eq!(average(&[]), None);
    assert_eq!(average(&[0]), Some(0));
    assert_eq!(average(&[100, 100]), Some(100));
    assert_eq!(average(&[70, 71]), Some(71), "70.5 rounds up");
    assert_eq!(average(&[70, 70, 71]), Some(70), "70.33 rounds down");
    assert_eq!(average(&[70, 71, 71]), Some(71), "70.67 rounds up");
    assert_eq!(average(&[0, 1]), Some(1), "0.5 rounds up");
}

#[test]
fn the_shown_score_is_the_newest_success_of_a_listed_project_only() {
    let mut seed = Seed::new();
    let (a, b, c) = (
        project("atlas", &[]),
        project("beacon", &[]),
        project("cedar", &[]),
    );
    seed.run(&a.path, 0, scored(40));
    let newest = seed.run(&a.path, 2, scored(80));
    // A newer failure keeps the success shown and is reported beside it.
    seed.run(&a.path, 4, End::Failed("timeout"));
    // A project no longer listed counts nowhere, however high its score.
    seed.run("/work/gone", 1, scored(100));
    seed.run(&b.path, 3, scored(61));
    let got = seed.overview(&[a.clone(), b.clone(), c.clone()]);
    let skill = &got.skills[0];
    let rows: Vec<_> = skill
        .rows
        .iter()
        .map(|row| {
            (
                row.path.as_str(),
                row.shown.as_ref().map(|shown| shown.score),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (a.path.as_str(), Some(80)),
            (b.path.as_str(), Some(61)),
            (c.path.as_str(), None)
        ]
    );
    let shown = skill.rows[0].shown.as_ref().unwrap();
    assert_eq!(shown.run_id, newest);
    assert_eq!(shown.graded_at_ms, T0 + 3 * MINUTE);
    assert_eq!(
        skill.rows[0].attempt,
        Some(GradingAttempt::Failed {
            run_id: "00000000-0000-4000-8000-000000000003".into(),
            finished_at_ms: T0 + 5 * MINUTE,
            reason: GradingFailure::Timeout,
        })
    );
    // (80 + 61) / 2 = 70.5, rounded up.
    assert_eq!(skill.average, Some(71));
    assert_eq!((skill.graded, skill.total), (2, 3));
    assert!(!skill.running);
    assert_eq!(got.running, None);
    assert_eq!(got.projects, [a, b, c]);
}

#[test]
fn a_partial_result_counts_and_is_marked() {
    let mut seed = Seed::new();
    let a = project("atlas", &[]);
    seed.run(
        &a.path,
        0,
        End::Succeeded {
            score: 55,
            complete: false,
            insights: "[]",
        },
    );
    let got = seed.overview(std::slice::from_ref(&a));
    let skill = &got.skills[0];
    assert_eq!(skill.average, Some(55));
    let shown = skill.rows[0].shown.as_ref().unwrap();
    assert!(shown.partial);
    assert_eq!(skill.rows[0].attempt, None);
}

#[test]
fn every_newer_status_is_an_attempt_and_only_a_stored_running_run_is_live() {
    let mut seed = Seed::new();
    let projects = [
        project("atlas", &[]),
        project("beacon", &[]),
        project("cedar", &[]),
        project("delta", &[]),
    ];
    seed.run(&projects[0].path, 0, End::Cancelled);
    seed.run(&projects[1].path, 0, scored(90));
    seed.run(&projects[1].path, 1, End::Interrupted);
    seed.run(&projects[2].path, 0, End::Failed("unheard_of_reason"));
    let live = seed.run(&projects[3].path, 5, End::Running);
    let got = seed.overview(&projects);
    let skill = &got.skills[0];
    let attempts: Vec<_> = skill.rows.iter().map(|row| row.attempt.clone()).collect();
    assert_eq!(
        attempts,
        [
            Some(GradingAttempt::Cancelled {
                run_id: "00000000-0000-4000-8000-000000000001".into(),
                finished_at_ms: T0 + MINUTE,
            }),
            Some(GradingAttempt::Interrupted {
                run_id: "00000000-0000-4000-8000-000000000003".into(),
                finished_at_ms: T0 + 2 * MINUTE,
                reason: GradingFailure::AppClosed,
            }),
            Some(GradingAttempt::Failed {
                run_id: "00000000-0000-4000-8000-000000000004".into(),
                finished_at_ms: T0 + MINUTE,
                reason: GradingFailure::Other,
            }),
            Some(GradingAttempt::Running {
                run_id: live.clone(),
                started_at_ms: T0 + 5 * MINUTE,
            }),
        ]
    );
    assert_eq!(
        skill.rows.iter().map(|row| row.running).collect::<Vec<_>>(),
        [false, false, false, true]
    );
    assert!(skill.running);
    assert_eq!(
        got.running,
        Some(GradingRunningNow {
            run_id: live,
            skill_id: SKILL.into(),
            skill_name: Some("CLAUDE.md doctor".into()),
            project_path: projects[3].path.clone(),
            project_name: Some("delta".into()),
            started_at_ms: T0 + 5 * MINUTE,
        })
    );
    // Only beacon has a shown score; the others are not graded.
    assert_eq!((skill.graded, skill.total, skill.average), (1, 4, Some(90)));
}

#[test]
fn a_run_going_on_an_unlisted_project_or_another_skill_is_named_as_it_is() {
    let mut seed = Seed::new();
    let a = project("atlas", &[]);
    seed.run_of("another-skill", "/work/gone", 0, End::Running);
    let got = seed.overview(std::slice::from_ref(&a));
    assert!(!got.skills[0].running);
    assert!(!got.skills[0].rows[0].running);
    let running = got.running.unwrap();
    assert_eq!(
        (running.skill_name, running.project_name),
        (None, None),
        "not a built-in skill, not a listed project"
    );
    assert_eq!(running.project_path, "/work/gone");
}

#[test]
fn top_insights_come_from_shown_runs_ranked_by_weight_then_reach_then_title() {
    let mut seed = Seed::new();
    let projects = [
        project("atlas", &[]),
        project("beacon", &[]),
        project("cedar", &[]),
    ];
    // An older success's findings are not shown, so they never count.
    seed.run(
        &projects[0].path,
        0,
        End::Succeeded {
            score: 10,
            complete: true,
            insights: r#"[{"severity":"critical","title":"Old finding"}]"#,
        },
    );
    seed.run(
        &projects[0].path,
        1,
        End::Succeeded {
            score: 80,
            complete: true,
            insights: r#"[{"severity":"warning","title":"Long file"},
                          {"severity":"info","title":"No hooks"},
                          {"severity":"info","title":"No hooks"},
                          {"severity":"pass","title":"Loads"}]"#,
        },
    );
    seed.run(
        &projects[1].path,
        0,
        End::Succeeded {
            score: 60,
            complete: true,
            insights: r#"[{"severity":"info","title":"Long file"},
                          {"severity":"critical","title":"Dead path"},
                          {"severity":"info","title":"No hooks"},
                          {"severity":"info","title":"A note"}]"#,
        },
    );
    // A newer failure does not hide cedar's shown run's findings.
    seed.run(
        &projects[2].path,
        0,
        End::Succeeded {
            score: 70,
            complete: true,
            insights: r#"[{"severity":"warning","title":"Long file"},
                          {"severity":"info","title":"No hooks"}]"#,
        },
    );
    seed.run(&projects[2].path, 3, End::Failed("timeout"));
    let got = seed.overview(&projects);
    let top: Vec<_> = got.skills[0]
        .top_insights
        .iter()
        .map(|insight| (insight.severity, insight.title.as_str(), insight.projects))
        .collect();
    assert_eq!(
        top,
        [
            (GradingSeverity::Critical, "Dead path", 1),
            // Its most severe weight, over all three projects.
            (GradingSeverity::Warning, "Long file", 3),
            // Listed twice by atlas, counted once per project.
            (GradingSeverity::Info, "No hooks", 3),
        ]
    );
}

#[test]
fn passes_are_listed_only_when_there_is_no_other_finding() {
    let mut seed = Seed::new();
    let projects = [project("atlas", &[]), project("beacon", &[])];
    for (index, title) in ["Loads", "Short"].into_iter().enumerate() {
        seed.run(
            &projects[index].path,
            0,
            End::Succeeded {
                score: 100,
                complete: true,
                insights: &format!(
                    r#"[{{"severity":"pass","title":"{title}"}},{{"severity":"pass","title":"Valid"}}]"#
                ),
            },
        );
    }
    let got = seed.overview(&projects);
    let top: Vec<_> = got.skills[0]
        .top_insights
        .iter()
        .map(|insight| (insight.severity, insight.title.as_str(), insight.projects))
        .collect();
    assert_eq!(
        top,
        [
            (GradingSeverity::Pass, "Valid", 2),
            (GradingSeverity::Pass, "Loads", 1),
            (GradingSeverity::Pass, "Short", 1),
        ]
    );
    // After a content purge the score stays shown and no finding is left.
    seed.store
        .purge_content(&xt_store::retention::ContentRegistry::default())
        .unwrap();
    let purged = seed.overview(&projects);
    assert_eq!(purged.skills[0].average, Some(100));
    assert!(purged.skills[0].top_insights.is_empty());
}

#[test]
fn every_signal_of_one_answer_changes_together() {
    let mut seed = Seed::new();
    let a = project("atlas", &[]);
    let before = seed.overview(std::slice::from_ref(&a));
    let skill = &before.skills[0];
    assert_eq!(
        (skill.average, skill.graded, skill.running),
        (None, 0, false)
    );
    assert_eq!(skill.rows[0].shown, None);
    let live = seed.run(&a.path, 0, End::Running);
    let during = seed.overview(std::slice::from_ref(&a));
    let skill = &during.skills[0];
    assert!(skill.running && skill.rows[0].running);
    assert_eq!(during.running.as_ref().unwrap().run_id, live);
    assert_eq!(skill.average, None);
    seed.store
        .grade_finish(
            &live,
            &GradeFinish::Succeeded {
                finished_at_ms: T0 + MINUTE,
                score: 64,
                complete: true,
                summary: "s",
                insights_json: r#"[{"severity":"warning","title":"Long file"}]"#,
                report_file: None,
            },
        )
        .unwrap();
    let after = seed.overview(std::slice::from_ref(&a));
    let skill = &after.skills[0];
    assert_eq!(
        (skill.average, skill.graded, skill.running),
        (Some(64), 1, false)
    );
    assert!(!skill.rows[0].running);
    assert_eq!(skill.rows[0].attempt, None);
    assert_eq!(skill.top_insights.len(), 1);
    assert_eq!(after.running, None);
}

#[test]
fn a_read_that_does_not_match_the_pairs_is_refused() {
    let seed = Seed::new();
    let projects = [project("atlas", &[])];
    let read = seed.store.grade_pairs(&[]).unwrap();
    assert_eq!(overview(projects.to_vec(), &skills(), &read, &[]), None);
}

#[test]
fn waiting_gradings_show_on_their_rows_count_on_their_skill_and_the_rest_are_named() {
    let mut seed = Seed::new();
    let projects = [
        project("atlas", &[]),
        project("beacon", &[]),
        project("cedar", &[]),
    ];
    let refused = seed.run(&projects[2].path, 1, End::Failed("project_changed"));
    let live = seed.run(&projects[0].path, 0, End::Running);
    let id = |n: u32| format!("00000000-0000-4000-8000-0000000000{n:02x}");
    let line = [
        // Read before the runs: these two have started since, so the read
        // shows them and the line does not.
        queued(&live, &projects[0].path, SKILL),
        queued(&refused, &projects[2].path, SKILL),
        queued(&id(0xb1), "/work/gone", SKILL),
        queued(&id(0xb2), &projects[1].path, SKILL),
        queued(&id(0xb3), &projects[1].path, ADDED),
        queued(&id(0xb4), &projects[1].path, "another-skill"),
    ];
    let got = seed.overview_line(&projects, &with_added(None), &line);
    let doctor = &got.skills[0];
    let rows: Vec<_> = doctor
        .rows
        .iter()
        .map(|row| (row.running, row.waiting.clone()))
        .collect();
    let at = |run_id: String, ahead: u32| Some(GradingRowWaiting { run_id, ahead });
    assert_eq!(
        rows,
        [(true, None), (false, at(id(0xb2), 1)), (false, None)],
        "ahead counts the whole line, other skills and unlisted projects too"
    );
    assert!(matches!(
        doctor.rows[2].attempt,
        Some(GradingAttempt::Failed { .. })
    ));
    // Its entries on any project, the unlisted one too.
    assert_eq!(doctor.waiting, 2);
    let added = &got.skills[1];
    assert_eq!(added.waiting, 1);
    assert_eq!(added.rows[1].waiting, at(id(0xb3), 2));
    assert_eq!(
        got.waiting_elsewhere,
        [
            GradingWaitingElsewhere {
                run_id: id(0xb1),
                skill_id: SKILL.into(),
                skill_name: Some("CLAUDE.md doctor".into()),
                project_path: "/work/gone".into(),
                project_name: None,
                ahead: 0,
            },
            GradingWaitingElsewhere {
                run_id: id(0xb4),
                skill_id: "another-skill".into(),
                skill_name: None,
                project_path: projects[1].path.clone(),
                project_name: Some("beacon".into()),
                ahead: 3,
            },
        ]
    );
    // An empty line shows nothing waiting anywhere.
    let none = seed.overview_with(&projects, &with_added(None));
    assert!(none.waiting_elsewhere.is_empty());
    assert!(
        none.skills
            .iter()
            .all(|skill| skill.waiting == 0 && skill.rows.iter().all(|row| row.waiting.is_none()))
    );
}

/// The answers the renderer's tests and acceptance pages read, one per
/// page state. Paths and names are made up.
#[derive(Serialize)]
struct Export {
    no_projects: GradingOverview,
    empty: GradingOverview,
    graded: GradingOverview,
    partial: GradingOverview,
    running: GradingOverview,
    running_unlisted: GradingOverview,
    waiting: GradingOverview,
    long_findings: GradingOverview,
    failed: GradingOverview,
    added: GradingOverview,
    added_problem: GradingOverview,
    making_rubric: GradingOverview,
    not_configured: GradingOverview,
    failed_read: GradingOverview,
}

const EXPORT: &str = "../ui/src/app/scores/scores.synthetic.json";

fn listed() -> Vec<ProjectEntry> {
    use ProjectSourceId::{ClaudeApp, ClaudeCode, CodexApp, CodexCli, Cursor};
    vec![
        project("atlas", &[ClaudeCode, ClaudeApp]),
        project("beacon", &[CodexApp, CodexCli]),
        project("cedar", &[Cursor]),
        project("delta", &[ClaudeCode, CodexCli, Cursor]),
    ]
}

/// atlas 75, beacon 90, cedar 61 (partial); delta not graded.
fn graded(seed: &mut Seed, projects: &[ProjectEntry]) {
    seed.run(
        &projects[0].path,
        0,
        End::Succeeded {
            score: 75,
            complete: true,
            insights: r#"[{"severity":"critical","title":"AGENTS.md is not loaded"},
                          {"severity":"warning","title":"CLAUDE.md is longer than 200 lines"},
                          {"severity":"pass","title":"Commands run"}]"#,
        },
    );
    seed.run(
        &projects[1].path,
        10,
        End::Succeeded {
            score: 90,
            complete: true,
            insights: r#"[{"severity":"warning","title":"CLAUDE.md is longer than 200 lines"},
                          {"severity":"info","title":"No hooks are set up"}]"#,
        },
    );
    seed.run(
        &projects[2].path,
        20,
        End::Succeeded {
            score: 61,
            complete: false,
            insights: r#"[{"severity":"warning","title":"CLAUDE.md is longer than 200 lines"},
                          {"severity":"warning","title":"Two rules disagree"}]"#,
        },
    );
}

fn export() -> Export {
    let projects = listed();
    let loaded = |seed: &Seed, projects: &[ProjectEntry]| {
        GradingOverview::Loaded(Box::new(seed.overview(projects)))
    };
    let no_projects = loaded(&Seed::new(), &[]);
    let empty = loaded(&Seed::new(), &projects);
    let mut seed = Seed::new();
    graded(&mut seed, &projects);
    let graded_answer = loaded(&seed, &projects);
    let mut partial = Seed::new();
    partial.run(
        &projects[0].path,
        0,
        End::Succeeded {
            score: 55,
            complete: false,
            insights: r#"[{"severity":"warning","title":"Some files could not be read"}]"#,
        },
    );
    let partial = loaded(&partial, &projects);
    let mut running = Seed::new();
    graded(&mut running, &projects);
    running.run(&projects[3].path, 40, End::Running);
    let running = loaded(&running, &projects);
    // A run still going on a project no longer in the list: no row shows it.
    let mut running_unlisted = Seed::new();
    graded(&mut running_unlisted, &projects);
    running_unlisted.run("/work/old-atlas", 40, End::Running);
    let running_unlisted = loaded(&running_unlisted, &projects);
    // One run going and two gradings waiting after it, one of them on a
    // project no longer in the list.
    let mut waiting = Seed::new();
    graded(&mut waiting, &projects);
    waiting.run(&projects[3].path, 40, End::Running);
    let waiting = GradingOverview::Loaded(Box::new(waiting.overview_line(
        &projects,
        &skills(),
        &[
            queued(
                "00000000-0000-4000-8000-0000000000c1",
                &projects[0].path,
                SKILL,
            ),
            queued(
                "00000000-0000-4000-8000-0000000000c2",
                "/work/old-beacon",
                SKILL,
            ),
        ],
    )));
    // Findings whose titles run past two lines on a card.
    let mut long_findings = Seed::new();
    long_findings.run(
        &projects[0].path,
        0,
        End::Succeeded {
            score: 70,
            complete: true,
            insights: r#"[{"severity":"critical","title":"The instructions name a build command that no longer exists in this project, so an agent that follows them as written fails before it changes anything"},
                          {"severity":"warning","title":"Two files give different rules for how tests are run, and nothing says which one wins when they disagree"}]"#,
        },
    );
    let long_findings = loaded(&long_findings, &projects);
    let mut failed = Seed::new();
    graded(&mut failed, &projects);
    failed.run(&projects[0].path, 30, End::Failed("timeout"));
    failed.run(&projects[3].path, 35, End::Interrupted);
    let failed = loaded(&failed, &projects);
    // An added skill beside the built-in one: graded on two projects.
    let mut added = Seed::new();
    graded(&mut added, &projects);
    added.run_of(ADDED, &projects[0].path, 50, scored(80));
    added.run_of(
        ADDED,
        &projects[1].path,
        55,
        End::Succeeded {
            score: 60,
            complete: true,
            insights: r#"[{"severity":"warning","title":"No run command"}]"#,
        },
    );
    let added_problem = GradingOverview::Loaded(Box::new(added.overview_with(
        &projects,
        &with_added(Some(GradingSkillProblem::RubricInvalid {
            rubric: GradingRubricProblem::Total,
        })),
    )));
    let added =
        GradingOverview::Loaded(Box::new(added.overview_with(&projects, &with_added(None))));
    // A rubric being made for an installed skill holds the one slot: the
    // service adds it to the stored answer.
    let mut making = seed.overview(&projects);
    making.making_rubric = Some(Box::new(crate::grading_dto::GradingRubricNow {
        job_id: "00000000-0000-4000-8000-0000000000aa".into(),
        location: "/home/you/.claude/skills/deploy".into(),
        name: "deploy".into(),
        started_at_ms: 1_784_000_000_000,
    }));
    let making_rubric = GradingOverview::Loaded(Box::new(making));
    Export {
        no_projects,
        empty,
        graded: graded_answer,
        partial,
        running,
        running_unlisted,
        waiting,
        long_findings,
        failed,
        added,
        added_problem,
        making_rubric,
        not_configured: GradingOverview::NotConfigured,
        failed_read: GradingOverview::Failed,
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
    let GradingOverview::Loaded(graded) = &export.graded else {
        panic!("graded is loaded");
    };
    let skill = &graded.skills[0];
    // (75 + 90 + 61) / 3 = 75.33.
    assert_eq!((skill.average, skill.graded, skill.total), (Some(75), 3, 4));
    assert_eq!(skill.top_insights[0].severity, GradingSeverity::Critical);
    assert_eq!(skill.top_insights[1].projects, 3);
    assert!(skill.rows[2].shown.as_ref().unwrap().partial);
    let GradingOverview::Loaded(running) = &export.running else {
        panic!("running is loaded");
    };
    assert!(running.skills[0].rows[3].running);
    let GradingOverview::Loaded(unlisted) = &export.running_unlisted else {
        panic!("running_unlisted is loaded");
    };
    assert!(unlisted.skills[0].running);
    assert!(unlisted.skills[0].rows.iter().all(|row| !row.running));
    assert_eq!(unlisted.running.as_ref().unwrap().project_name, None);
    let GradingOverview::Loaded(waiting) = &export.waiting else {
        panic!("waiting is loaded");
    };
    let skill = &waiting.skills[0];
    assert!(skill.running && skill.rows[3].running);
    assert_eq!(skill.waiting, 2);
    assert_eq!(skill.rows[0].waiting.as_ref().unwrap().ahead, 0);
    assert_eq!(waiting.waiting_elsewhere.len(), 1);
    assert_eq!(waiting.waiting_elsewhere[0].project_name, None);
    assert_eq!(waiting.waiting_elsewhere[0].ahead, 1);
    let GradingOverview::Loaded(failed) = &export.failed else {
        panic!("failed is loaded");
    };
    assert!(matches!(
        failed.skills[0].rows[0].attempt,
        Some(GradingAttempt::Failed { .. })
    ));
    assert!(matches!(
        failed.skills[0].rows[3].attempt,
        Some(GradingAttempt::Interrupted { .. })
    ));
    // An added skill is a card of its own, after the built-in one, with its
    // own average; one that cannot grade keeps its scores.
    for (name, problem) in [("added", false), ("added_problem", true)] {
        let GradingOverview::Loaded(answer) = (if problem {
            &export.added_problem
        } else {
            &export.added
        }) else {
            panic!("{name} is loaded");
        };
        assert_eq!(answer.skills.len(), 2, "{name}");
        let added = &answer.skills[1];
        assert_eq!(added.skill.id, ADDED);
        assert_eq!((added.average, added.graded, added.total), (Some(70), 2, 4));
        assert_eq!(added.skill.problem.is_some(), problem);
        assert_eq!(answer.skills[0].average, Some(75));
    }
}
