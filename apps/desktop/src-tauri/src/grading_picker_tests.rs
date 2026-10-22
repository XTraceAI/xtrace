//! The "Add a skill" dialog's rule over hand-built discoveries: groups,
//! order, counts, what is already on Scores and id clashes.
use super::*;
use crate::grading_dto::{
    GradingInstalledGroup, GradingRubricAction, GradingRubricJob, GradingRubricProblem,
};
use std::collections::HashMap;
use std::path::PathBuf;
use xt_judge::{RubricError, installed::Location, rubric};

fn rubric_with(id: &str, name: &str) -> xt_judge::Rubric {
    rubric::parse(
        format!(
            "---\nprotocol: 1\nid: {id}\nname: {name}\nversion: 1\n\
             description: Rubric of {name}.\ntarget: project\n---\n## All (100 points)\nAll.\n"
        )
        .as_bytes(),
    )
    .unwrap()
}

fn found(folder: &str, source: Source, name: &str, rubric: RubricStatus) -> InstalledSkill {
    InstalledSkill {
        folder: PathBuf::from(folder),
        location: Location::Folder(PathBuf::from(folder)),
        source,
        name: name.into(),
        description: format!("About {name}."),
        rubric,
        rubric_kept_by_xtrace: false,
    }
}

fn ready(folder: &str, name: &str, id: &str) -> InstalledSkill {
    found(
        folder,
        Source::Claude,
        &format!("skill {name}"),
        RubricStatus::Ready(rubric_with(id, name)),
    )
}

fn added(id: &str, location: &str) -> AddedSkill {
    AddedSkill {
        skill_id: id.into(),
        location: location.into(),
        name: id.into(),
        description: String::new(),
        rubric_version: "1 #000000000000".into(),
        added_at_ms: 0,
    }
}

fn discovery(skills: Vec<InstalledSkill>) -> Discovery {
    Discovery {
        skills,
        skipped: 2,
        incomplete: false,
        plugins_unread: None,
    }
}

fn groups(loaded: &GradingInstalledLoaded) -> Vec<GradingInstalledGroup> {
    loaded.skills.iter().map(|row| row.group).collect()
}

fn summary(loaded: &GradingInstalledLoaded) -> Vec<(&str, &str, GradingInstalledScores)> {
    loaded
        .skills
        .iter()
        .map(|row| (row.folder.as_str(), row.name.as_str(), row.scores.clone()))
        .collect()
}

#[test]
fn ready_first_then_problems_then_needs_a_rubric_by_name_with_counts() {
    let loaded = picker(
        discovery(vec![
            found("/h/z", Source::Codex, "zeta", RubricStatus::Missing),
            found(
                "/h/p",
                Source::Agents,
                "Problem",
                RubricStatus::Problem(RubricError::Total),
            ),
            ready("/h/b", "beta", "b-check"),
            found(
                "/h/a",
                Source::Plugin("tools".into()),
                "Alpha",
                RubricStatus::Missing,
            ),
            ready("/h/a2", "Alpha", "a-check"),
        ]),
        &[],
        &[],
    );
    assert_eq!(
        summary(&loaded),
        [
            ("/h/a2", "Alpha", GradingInstalledScores::Addable),
            ("/h/b", "beta", GradingInstalledScores::Addable),
            ("/h/p", "Problem", GradingInstalledScores::NotReady),
            ("/h/a", "Alpha", GradingInstalledScores::NotReady),
            ("/h/z", "zeta", GradingInstalledScores::NotReady),
        ]
    );
    assert_eq!(
        (
            loaded.ready,
            loaded.yours,
            loaded.cant_score,
            loaded.on_scores,
            loaded.skipped,
            loaded.incomplete
        ),
        (2, 3, 0, 0, 2, false)
    );
    assert_eq!(
        groups(&loaded),
        [
            GradingInstalledGroup::Ready,
            GradingInstalledGroup::Ready,
            GradingInstalledGroup::Yours,
            GradingInstalledGroup::Yours,
            GradingInstalledGroup::Yours,
        ]
    );
    // A ready skill shows its rubric's name and description; the others
    // their SKILL.md's.
    assert_eq!(loaded.skills[0].description, "Rubric of Alpha.");
    assert_eq!(loaded.skills[3].description, "About Alpha.");
    assert_eq!(
        loaded.skills[0].rubric,
        GradingInstalledRubric::Ready {
            id: "a-check".into()
        }
    );
    assert_eq!(
        loaded.skills[2].rubric,
        GradingInstalledRubric::Problem {
            reason: GradingRubricProblem::Total
        }
    );
    assert_eq!(
        loaded.skills[3].source,
        GradingSkillSource::Plugin {
            plugin: "tools".into()
        }
    );
}

#[test]
fn a_built_in_or_added_id_is_already_on_scores_and_cannot_be_added_again() {
    let loaded = picker(
        discovery(vec![
            // The user's own copy of the built-in skill, without a rubric.
            found(
                "/h/doctor",
                Source::Claude,
                "claude-md-doctor",
                RubricStatus::Missing,
            ),
            // A ready skill whose rubric id is the built-in's.
            ready("/h/doctor2", "Doctor two", "claude-md-doctor"),
            // The added skill itself, and another folder with its id.
            ready("/h/lint", "Lint", "lint-check"),
            ready("/h/lint-copy", "Lint copy", "lint-check"),
            // A skill whose rubric is broken now but whose folder was added.
            found(
                "/h/was",
                Source::Agents,
                "was",
                RubricStatus::Problem(RubricError::NoHeader),
            ),
        ]),
        &["claude-md-doctor"],
        &[added("lint-check", "/h/lint"), added("was-check", "/h/was")],
    );
    // The added skill whose rubric broke can have it remade: it is one of
    // "your skills"; every other one is already on Scores.
    assert_eq!(
        summary(&loaded),
        [
            ("/h/was", "was", GradingInstalledScores::OnScores),
            ("/h/doctor2", "Doctor two", GradingInstalledScores::BuiltIn),
            ("/h/lint", "Lint", GradingInstalledScores::OnScores),
            (
                "/h/lint-copy",
                "Lint copy",
                GradingInstalledScores::IdOnScores {
                    location: "/h/lint".into()
                }
            ),
            (
                "/h/doctor",
                "claude-md-doctor",
                GradingInstalledScores::BuiltIn
            ),
        ]
    );
    assert_eq!(
        groups(&loaded),
        [
            GradingInstalledGroup::Yours,
            GradingInstalledGroup::OnScores,
            GradingInstalledGroup::OnScores,
            GradingInstalledGroup::OnScores,
            GradingInstalledGroup::OnScores,
        ]
    );
    assert_eq!(
        (
            loaded.ready,
            loaded.yours,
            loaded.cant_score,
            loaded.on_scores
        ),
        (0, 1, 0, 4)
    );
}

#[test]
fn of_two_ready_skills_with_one_id_only_the_first_can_be_added() {
    let skills = || {
        discovery(vec![
            ready("/h/two", "Second", "same-check"),
            ready("/h/one", "First", "same-check"),
            ready("/h/other", "Other", "other-check"),
        ])
    };
    let loaded = picker(skills(), &[], &[]);
    assert_eq!(
        summary(&loaded),
        [
            ("/h/one", "First", GradingInstalledScores::Addable),
            ("/h/other", "Other", GradingInstalledScores::Addable),
            (
                "/h/two",
                "Second",
                GradingInstalledScores::IdClash {
                    name: "First".into(),
                    folder: "/h/one".into()
                }
            ),
        ]
    );
    // Both are ready to add; only the first has Add.
    assert_eq!(groups(&loaded), [GradingInstalledGroup::Ready; 3]);
    // Once the second one is added, the first is the one whose id is taken:
    // both are listed as already on Scores, after the one ready to add.
    let loaded = picker(skills(), &[], &[added("same-check", "/h/two")]);
    assert_eq!(
        summary(&loaded),
        [
            ("/h/other", "Other", GradingInstalledScores::Addable),
            (
                "/h/one",
                "First",
                GradingInstalledScores::IdOnScores {
                    location: "/h/two".into()
                }
            ),
            ("/h/two", "Second", GradingInstalledScores::OnScores),
        ]
    );
    assert_eq!(
        groups(&loaded),
        [
            GradingInstalledGroup::Ready,
            GradingInstalledGroup::OnScores,
            GradingInstalledGroup::OnScores,
        ]
    );
}

#[test]
fn a_plugin_skill_is_on_scores_by_its_plugin_whatever_its_versioned_folder() {
    let mut skill = ready(
        "/h/.claude/plugins/cache/m/tools/2.0/skills/lint",
        "Lint",
        "lint-check",
    );
    skill.source = Source::Plugin("tools".into());
    skill.location = Location::Plugin {
        key: "tools@m".into(),
        skill: "lint".into(),
    };
    let mut found = discovery(vec![skill]);
    found.incomplete = true;
    found.plugins_unread = Some(xt_judge::installed::PluginProblem {
        file: xt_judge::installed::PluginFile::Settings,
        failure: xt_judge::installed::PluginReadFailure::Malformed,
    });
    let loaded = picker(found.clone(), &[], &[]);
    assert_eq!(loaded.skills[0].location, "plugin:tools@m/lint");
    assert_eq!(
        loaded.skills[0].folder,
        "/h/.claude/plugins/cache/m/tools/2.0/skills/lint"
    );
    assert_eq!(loaded.skills[0].scores, GradingInstalledScores::Addable);
    assert!(loaded.incomplete);
    assert_eq!(
        loaded.plugins_unread,
        Some(crate::grading_dto::GradingPluginProblem {
            file: crate::grading_dto::GradingPluginFile::Settings,
            failure: crate::grading_dto::GradingPluginReadFailure::Malformed,
        })
    );
    // Added while it was version 1.0: the same skill now.
    let loaded = picker(found, &[], &[added("lint-check", "plugin:tools@m/lint")]);
    assert_eq!(loaded.skills[0].scores, GradingInstalledScores::OnScores);
}

#[test]
fn an_empty_discovery_lists_nothing() {
    let loaded = picker(Discovery::default(), &["claude-md-doctor"], &[]);
    assert!(loaded.skills.is_empty());
    assert_eq!(
        (
            loaded.ready,
            loaded.yours,
            loaded.cant_score,
            loaded.on_scores
        ),
        (0, 0, 0, 0)
    );
}

/// The answers the renderer's tests and acceptance pages read for the Add a
/// skill dialog. Folders and names are made up.
#[derive(serde::Serialize)]
struct Export {
    mixed: crate::grading_dto::GradingInstalledSkills,
    plugins_unread: crate::grading_dto::GradingInstalledSkills,
    added: crate::grading_dto::GradingInstalledSkills,
    many: crate::grading_dto::GradingInstalledSkills,
    making: crate::grading_dto::GradingInstalledSkills,
    made: crate::grading_dto::GradingInstalledSkills,
    make_failed: crate::grading_dto::GradingInstalledSkills,
    yours: crate::grading_dto::GradingInstalledSkills,
    all_refused: crate::grading_dto::GradingInstalledSkills,
    empty: crate::grading_dto::GradingInstalledSkills,
    not_configured: crate::grading_dto::GradingInstalledSkills,
    failed: crate::grading_dto::GradingInstalledSkills,
}

const EXPORT: &str = "../ui/src/app/scores/picker.synthetic.json";
const HOME: &str = "/home/you";

fn at(folder: &str) -> String {
    format!("{HOME}/{folder}")
}

fn sample(
    folder: &str,
    source: Source,
    name: &str,
    description: &str,
    rubric: RubricStatus,
) -> InstalledSkill {
    // A plugin's skill is found again by its plugin, as discovery says.
    let location = match &source {
        Source::Plugin(plugin) => Location::Plugin {
            key: format!("{plugin}@official"),
            skill: folder.rsplit('/').next().unwrap_or_default().to_owned(),
        },
        _ => Location::Folder(PathBuf::from(at(folder))),
    };
    InstalledSkill {
        folder: PathBuf::from(at(folder)),
        location,
        source,
        name: name.into(),
        description: description.into(),
        rubric,
        rubric_kept_by_xtrace: false,
    }
}

fn graded(id: &str, name: &str, description: &str) -> RubricStatus {
    RubricStatus::Ready(
        rubric::parse(
            format!(
                "---\nprotocol: 1\nid: {id}\nname: {name}\nversion: 1.0\n\
                 description: {description}\ntarget: project\n---\n## All (100 points)\nAll.\n"
            )
            .as_bytes(),
        )
        .unwrap(),
    )
}

/// Skills in every group: ready ones (one addable from each of two places,
/// one clashing with another's id, one carrying the built-in id), rubric
/// problems, and skills without a rubric.
fn mixed() -> Discovery {
    Discovery {
        skills: vec![
            sample(
                ".claude/skills/readme-check",
                Source::Claude,
                "readme-check",
                "Checks READMEs.",
                graded(
                    "readme-check",
                    "README check",
                    "Checks that a project's README explains how to install and run it.",
                ),
            ),
            sample(
                ".claude/plugins/cache/tools/code-tools/1.2.0/skills/lint",
                Source::Plugin("code-tools".into()),
                "lint",
                "Lints.",
                graded(
                    "lint-check",
                    "Lint check",
                    "Grades how clean the linter leaves a project.",
                ),
            ),
            sample(
                ".agents/skills/lint",
                Source::Agents,
                "lint",
                "Lints.",
                graded(
                    "lint-check",
                    "Lint check (my copy)",
                    "My own copy of the lint grader.",
                ),
            ),
            sample(
                ".claude/skills/doctor-graded",
                Source::Claude,
                "doctor-graded",
                "A copy.",
                graded(
                    "claude-md-doctor",
                    "CLAUDE.md Doctor (copy)",
                    "A copy of the built-in rubric.",
                ),
            ),
            sample(
                ".agents/skills/release-notes",
                Source::Agents,
                "release-notes",
                "Writes release notes from merged pull requests.",
                RubricStatus::Problem(RubricError::Total),
            ),
            sample(
                ".codex/skills/security-scan",
                Source::Codex,
                "security-scan",
                "Looks for leaked keys and risky settings.",
                RubricStatus::Problem(RubricError::HeaderUnterminated),
            ),
            sample(
                ".claude/skills/claude-md-doctor",
                Source::Claude,
                "claude-md-doctor",
                "Checks CLAUDE.md and AGENTS.md files and backtests their rules.",
                RubricStatus::Missing,
            ),
            sample(
                ".claude/skills/deploy",
                Source::Claude,
                "deploy",
                "Ships the current branch to staging.",
                RubricStatus::Missing,
            ),
            sample(
                ".claude/plugins/cache/official/design/2.0.1/skills/frontend-design",
                Source::Plugin("design".into()),
                "frontend-design",
                "Builds distinctive, production-grade interfaces with a strong point of view on typography, color and layout.",
                RubricStatus::Missing,
            ),
            sample(
                ".claude/plugins/cache/official/documents/1.0.0/skills/pdf",
                Source::Plugin("documents".into()),
                "pdf",
                "Reads, fills and merges PDF files.",
                RubricStatus::Missing,
            ),
        ],
        skipped: 1,
        incomplete: false,
        plugins_unread: None,
    }
}

fn export() -> Export {
    use crate::grading_dto::GradingInstalledSkills::{Failed, Loaded, NotConfigured};
    let built_in = ["claude-md-doctor"];
    let mut many = mixed();
    for index in 0..36 {
        many.skills.push(sample(
            &format!(".claude/plugins/cache/official/tools/1.0.0/skills/tool-{index:02}"),
            Source::Plugin("tools".into()),
            &format!("tool-{index:02}"),
            "A plugin skill without a rubric.",
            RubricStatus::Missing,
        ));
    }
    let mut unread = mixed();
    unread.incomplete = true;
    unread.plugins_unread = Some(xt_judge::installed::PluginProblem {
        file: xt_judge::installed::PluginFile::InstalledPlugins,
        failure: xt_judge::installed::PluginReadFailure::Link,
    });
    unread
        .skills
        .retain(|skill| !matches!(skill.source, Source::Plugin(_)));
    Export {
        mixed: Loaded(picker(mixed(), &built_in, &[])),
        plugins_unread: Loaded(picker(unread, &built_in, &[])),
        added: Loaded(picker(
            mixed(),
            &built_in,
            &[AddedSkill {
                skill_id: "readme-check".into(),
                location: at(".claude/skills/readme-check"),
                name: "README check".into(),
                description: String::new(),
                rubric_version: "1.0 #3fa2b9c1d0e4".into(),
                added_at_ms: 0,
            }],
        )),
        many: Loaded(picker(many, &built_in, &[])),
        making: Loaded(picker_with(mixed(), &built_in, &[], &making_jobs())),
        made: Loaded(picker_with(made(), &built_in, &[], &made_jobs())),
        make_failed: Loaded(picker_with(mixed(), &built_in, &[], &failed_jobs())),
        yours: Loaded(picker_with(yours(), &built_in, &[], &yours_refusals())),
        all_refused: Loaded(picker_with(
            all_refused(),
            &built_in,
            &[],
            &yours_refusals(),
        )),
        empty: Loaded(picker(Discovery::default(), &built_in, &[])),
        not_configured: NotConfigured,
        failed: Failed,
    }
}

const DEPLOY: &str = ".claude/skills/deploy";
const DESIGN: &str = "plugin:design@official/frontend-design";
const NOTES: &str = ".agents/skills/release-notes";

/// A rubric being made for `deploy`, started at a fixed time.
fn making_jobs() -> HashMap<String, GradingRubricJob> {
    HashMap::from([(
        at(DEPLOY),
        GradingRubricJob::Running {
            job_id: "00000000-0000-4000-8000-0000000000aa".into(),
            started_at_ms: 1_784_000_000_000,
        },
    )])
}

/// `deploy` after its rubric was made and saved in its folder; the design
/// plugin's skill after its rubric was saved in XTrace's own folder.
fn made() -> Discovery {
    let mut found = mixed();
    for skill in &mut found.skills {
        if skill.folder.to_str() == Some(at(DEPLOY).as_str()) {
            skill.rubric = graded(
                "deploy",
                "Deploy",
                "Whether the branch ships to staging cleanly.",
            );
        }
        if skill.location.text().as_deref() == Some(DESIGN) {
            skill.rubric = graded(
                "frontend-design",
                "Frontend design",
                "How distinctive and finished a project's interface is.",
            );
            skill.rubric_kept_by_xtrace = true;
        }
    }
    found
}

fn made_jobs() -> HashMap<String, GradingRubricJob> {
    let criterion = |name: &str, points| crate::grading_dto::GradingRubricCriterion {
        name: name.into(),
        points,
    };
    HashMap::from([
        (
            at(DEPLOY),
            GradingRubricJob::Made {
                job_id: "00000000-0000-4000-8000-0000000000aa".into(),
                criteria: vec![
                    criterion("Build passes", 40),
                    criterion("Staging deploy succeeds", 40),
                    criterion("Rollback documented", 20),
                ],
                kept_by_xtrace: false,
            },
        ),
        (
            DESIGN.to_owned(),
            GradingRubricJob::Made {
                job_id: "00000000-0000-4000-8000-0000000000ab".into(),
                criteria: vec![criterion("Distinctive design", 100)],
                kept_by_xtrace: true,
            },
        ),
    ])
}

const PDF: &str = "plugin:documents@official/pdf";

/// Made-up skill names for a home like a real one: about fifty skills, none
/// with a rubric yet.
const YOUR_SKILLS: [&str; 47] = [
    "api-docs",
    "architecture",
    "brand-voice",
    "bug-triage",
    "changelog",
    "ci-fix",
    "code-review",
    "commit-message",
    "competitor-notes",
    "cost-report",
    "data-cleanup",
    "db-migrate",
    "debug",
    "deploy-check",
    "design-critique",
    "docs-site",
    "email-draft",
    "error-budget",
    "feature-flag",
    "frontend-polish",
    "grant-writer",
    "incident-review",
    "interview-notes",
    "invoice",
    "landing-copy",
    "meeting-notes",
    "metrics-sql",
    "onboarding",
    "perf-profile",
    "pitch-deck",
    "pr-describe",
    "privacy-check",
    "release-plan",
    "research-brief",
    "roadmap",
    "runbook",
    "sales-email",
    "schema-design",
    "seo-audit",
    "slides",
    "spec-writer",
    "standup",
    "support-reply",
    "test-plan",
    "translate",
    "user-interview",
    "weekly-update",
];

/// Where a made-up skill of [`YOUR_SKILLS`] is installed, by its index.
fn your_skill(index: usize, name: &str) -> InstalledSkill {
    let description = format!("Helps with {}.", name.replace('-', " "));
    match index % 4 {
        0 => sample(
            &format!(".claude/skills/{name}"),
            Source::Claude,
            name,
            &description,
            RubricStatus::Missing,
        ),
        1 => sample(
            &format!(".agents/skills/{name}"),
            Source::Agents,
            name,
            &description,
            RubricStatus::Missing,
        ),
        2 => sample(
            &format!(".claude/plugins/cache/official/work/1.4.0/skills/{name}"),
            Source::Plugin("work".into()),
            name,
            &description,
            RubricStatus::Missing,
        ),
        _ => sample(
            &format!(".codex/skills/{name}"),
            Source::Codex,
            name,
            &description,
            RubricStatus::Missing,
        ),
    }
}

/// A home like a real one: nothing ready, fifty skills, two Claude refused,
/// and a copy of the built-in skill.
fn yours() -> Discovery {
    let mut skills: Vec<InstalledSkill> = YOUR_SKILLS
        .iter()
        .enumerate()
        .map(|(index, name)| your_skill(index, name))
        .collect();
    for (folder, name, description) in [
        (
            ".claude/skills/framer",
            "framer",
            "Designs, edits and publishes Framer sites.",
        ),
        (
            ".agents/skills/code-review-patterns",
            "code-review-patterns",
            "Self-reviews a diff before a push and fixes what it finds.",
        ),
        (
            ".claude/skills/claude-md-doctor",
            "claude-md-doctor",
            "Checks CLAUDE.md and AGENTS.md files and backtests their rules.",
        ),
    ] {
        skills.push(sample(
            folder,
            if folder.starts_with(".agents") {
                Source::Agents
            } else {
                Source::Claude
            },
            name,
            description,
            RubricStatus::Missing,
        ));
    }
    Discovery {
        skills,
        skipped: 0,
        incomplete: false,
        plugins_unread: None,
    }
}

/// Only [`yours`]'s two refused skills and the built-in copy: no other skill
/// needs a rubric.
fn all_refused() -> Discovery {
    let mut found = yours();
    found
        .skills
        .retain(|skill| !YOUR_SKILLS.contains(&skill.name.as_str()));
    found
}

/// The two kept refusals of [`yours`], as in force.
fn yours_refusals() -> HashMap<String, GradingRubricJob> {
    HashMap::from([
        (
            at(".claude/skills/framer"),
            GradingRubricJob::Refused {
                job_id: "00000000-0000-4000-8000-0000000000b1".into(),
                reason: "It designs, edits and publishes Framer sites through a networked \
                         tool; it does not review a local project and cannot work offline."
                    .into(),
            },
        ),
        (
            at(".agents/skills/code-review-patterns"),
            GradingRubricJob::Refused {
                job_id: "00000000-0000-4000-8000-0000000000b2".into(),
                reason: "It fixes its own diff, commits and pushes; it needs project changes \
                         and the network and writes no report to score."
                    .into(),
            },
        ),
    ])
}

/// A making that failed on its retry, one cancelled, a remake whose run
/// timed out, and one Claude refused.
fn failed_jobs() -> HashMap<String, GradingRubricJob> {
    use crate::grading_dto::GradingRubricFailure;
    HashMap::from([
        (
            PDF.to_owned(),
            GradingRubricJob::Refused {
                job_id: "00000000-0000-4000-8000-0000000000ad".into(),
                reason: "This skill reads, fills and merges PDF files; it does not review a \
                         project, so its output cannot be scored."
                    .into(),
            },
        ),
        (
            at(DEPLOY),
            GradingRubricJob::Failed {
                job_id: "00000000-0000-4000-8000-0000000000aa".into(),
                failure: GradingRubricFailure::RubricInvalid {
                    rubric: GradingRubricProblem::Total,
                },
            },
        ),
        (
            DESIGN.to_owned(),
            GradingRubricJob::Cancelled {
                job_id: "00000000-0000-4000-8000-0000000000ab".into(),
            },
        ),
        (
            at(NOTES),
            GradingRubricJob::Failed {
                job_id: "00000000-0000-4000-8000-0000000000ac".into(),
                failure: GradingRubricFailure::Run {
                    failure: crate::grading_dto::GradingFailure::Timeout,
                },
            },
        ),
    ])
}

/// The committed export is exactly what the rule gives now. Regenerate it
/// with `XTRACE_WRITE_SCORES_SYNTHETIC=1`.
#[test]
fn the_synthetic_ui_export_is_what_the_rule_gives() {
    let export = export();
    let json = serde_json::to_string_pretty(&export).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPORT);
    if std::env::var_os("XTRACE_WRITE_SCORES_SYNTHETIC").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    assert_eq!(
        json,
        std::fs::read_to_string(&path).unwrap_or_default(),
        "regenerate with XTRACE_WRITE_SCORES_SYNTHETIC=1"
    );
    // What the renderer's acceptance relies on.
    let crate::grading_dto::GradingInstalledSkills::Loaded(mixed) = &export.mixed else {
        panic!("mixed is loaded");
    };
    assert_eq!(
        (mixed.ready, mixed.yours, mixed.cant_score, mixed.on_scores),
        (3, 5, 0, 2)
    );
    let states: Vec<_> = mixed.skills.iter().map(|row| row.scores.clone()).collect();
    assert!(states.contains(&GradingInstalledScores::Addable));
    assert!(states.contains(&GradingInstalledScores::BuiltIn));
    assert!(
        states
            .iter()
            .any(|state| matches!(state, GradingInstalledScores::IdClash { .. }))
    );
    // Make a rubric for each skill that needs one, make it again for each
    // whose rubric has a problem, neither for a ready one nor for one Scores
    // already has as a built-in skill.
    for row in &mixed.skills {
        let action = match (&row.rubric, &row.scores) {
            (GradingInstalledRubric::Ready { .. }, _) | (_, GradingInstalledScores::BuiltIn) => {
                None
            }
            (GradingInstalledRubric::Problem { .. }, _) => Some(GradingRubricAction::Remake),
            (GradingInstalledRubric::Missing, _) => Some(GradingRubricAction::Make),
        };
        assert_eq!(row.rubric_action, action, "{}", row.name);
        assert_eq!(row.rubric_job, None);
    }
    let crate::grading_dto::GradingInstalledSkills::Loaded(made) = &export.made else {
        panic!("made is loaded");
    };
    let deploy = made.skills.iter().find(|row| row.name == "Deploy").unwrap();
    assert_eq!(deploy.scores, GradingInstalledScores::Addable);
    assert!(matches!(
        deploy.rubric_job,
        Some(GradingRubricJob::Made { .. })
    ));
    let design = made
        .skills
        .iter()
        .find(|row| row.location == DESIGN)
        .unwrap();
    assert!(design.rubric_kept_by_xtrace);
    assert_eq!((made.ready, made.yours), (5, 3));
    // A refused skill is listed as one that can't be scored, with Try again.
    let crate::grading_dto::GradingInstalledSkills::Loaded(failed) = &export.make_failed else {
        panic!("make_failed is loaded");
    };
    let pdf = failed
        .skills
        .iter()
        .find(|row| row.location == PDF)
        .unwrap();
    assert_eq!(pdf.group, GradingInstalledGroup::CantScore);
    assert_eq!(pdf.rubric_action, Some(GradingRubricAction::Make));
    assert_eq!(
        (
            failed.ready,
            failed.yours,
            failed.cant_score,
            failed.on_scores
        ),
        (3, 4, 1, 2)
    );
    let crate::grading_dto::GradingInstalledSkills::Loaded(yours) = &export.yours else {
        panic!("yours is loaded");
    };
    assert_eq!(yours.skills.len(), 50);
    let crate::grading_dto::GradingInstalledSkills::Loaded(refused) = &export.all_refused else {
        panic!("all_refused is loaded");
    };
    assert_eq!(
        (
            refused.ready,
            refused.yours,
            refused.cant_score,
            refused.on_scores
        ),
        (0, 0, 2, 1)
    );
    assert_eq!(
        (yours.ready, yours.yours, yours.cant_score, yours.on_scores),
        (0, 47, 2, 1)
    );
    let crate::grading_dto::GradingInstalledSkills::Loaded(added) = &export.added else {
        panic!("added is loaded");
    };
    assert!(
        added
            .skills
            .iter()
            .any(|row| row.scores == GradingInstalledScores::OnScores
                && row.folder == at(".claude/skills/readme-check"))
    );
}

/// No rubric action for a skill Scores already has under another skill (a
/// built-in one, or an added one from elsewhere); one on Scores itself
/// keeps its remake.
#[test]
fn no_rubric_action_for_a_skill_scores_already_has_elsewhere() {
    let found = discovery(vec![
        found(
            "/s/claude-md-doctor",
            Source::Claude,
            "claude-md-doctor",
            RubricStatus::Missing,
        ),
        found(
            "/s/lint-check",
            Source::Claude,
            "lint-check",
            RubricStatus::Problem(RubricError::Total),
        ),
        found(
            "/s/mine",
            Source::Claude,
            "mine",
            RubricStatus::Problem(RubricError::Total),
        ),
        found("/s/new", Source::Claude, "new", RubricStatus::Missing),
    ]);
    let loaded = picker(
        found,
        &["claude-md-doctor"],
        &[added("lint-check", "/elsewhere"), added("mine", "/s/mine")],
    );
    let actions: Vec<_> = loaded
        .skills
        .iter()
        .map(|row| (row.name.as_str(), row.scores.clone(), row.rubric_action))
        .collect();
    assert_eq!(
        actions,
        [
            (
                "mine",
                GradingInstalledScores::OnScores,
                Some(GradingRubricAction::Remake)
            ),
            (
                "new",
                GradingInstalledScores::NotReady,
                Some(GradingRubricAction::Make)
            ),
            (
                "lint-check",
                GradingInstalledScores::IdOnScores {
                    location: "/elsewhere".into()
                },
                None
            ),
            ("claude-md-doctor", GradingInstalledScores::BuiltIn, None),
        ]
    );
}

/// A finished job is shown only while it matches the rubric now: a made
/// rubric while ready; a failure, refusal or cancel while not. One going is
/// always shown.
#[test]
fn a_finished_job_shows_only_while_it_matches_the_rubric() {
    use crate::grading_dto::GradingRubricFailure;
    let id = || "00000000-0000-4000-8000-0000000000aa".to_owned();
    let made = GradingRubricJob::Made {
        job_id: id(),
        criteria: vec![],
        kept_by_xtrace: false,
    };
    let finished_not_made = [
        GradingRubricJob::Failed {
            job_id: id(),
            failure: GradingRubricFailure::RubricMissing,
        },
        GradingRubricJob::Refused {
            job_id: id(),
            reason: "No.".into(),
        },
        GradingRubricJob::Cancelled { job_id: id() },
    ];
    let running = GradingRubricJob::Running {
        job_id: id(),
        started_at_ms: 0,
    };
    let rows = |job: &GradingRubricJob| {
        let found = discovery(vec![
            ready("/s/ready", "ready", "ready"),
            found(
                "/s/broken",
                Source::Claude,
                "broken",
                RubricStatus::Problem(RubricError::Total),
            ),
            found("/s/none", Source::Claude, "none", RubricStatus::Missing),
        ]);
        let jobs = ["/s/ready", "/s/broken", "/s/none"]
            .into_iter()
            .map(|location| (location.to_owned(), job.clone()))
            .collect();
        picker_with(found, &[], &[], &jobs)
            .skills
            .into_iter()
            .map(|row| (row.folder, row.rubric_job.is_some()))
            .collect::<Vec<_>>()
    };
    let shown = |ready: bool, others: bool| {
        vec![
            ("/s/ready".to_owned(), ready),
            ("/s/broken".to_owned(), others),
            ("/s/none".to_owned(), others),
        ]
    };
    assert_eq!(rows(&made), shown(true, false));
    for job in &finished_not_made {
        assert_eq!(rows(job), shown(false, true), "{job:?}");
    }
    assert_eq!(rows(&running), shown(true, true));
}

/// The display group follows the rubric action and the job: a skill a
/// rubric can be made for is "can't be scored" while its job is a refusal,
/// and back among "your skills" while another job goes or after another
/// outcome; a refusal of a skill Scores already has puts it nowhere new.
#[test]
fn a_refusal_puts_a_skill_under_cant_be_scored_and_another_job_takes_it_back() {
    use crate::grading_dto::GradingRubricFailure;
    let id = || "00000000-0000-4000-8000-0000000000aa".to_owned();
    let found = || {
        discovery(vec![
            found("/s/none", Source::Claude, "none", RubricStatus::Missing),
            found(
                "/s/broken",
                Source::Claude,
                "broken",
                RubricStatus::Problem(RubricError::Total),
            ),
            found(
                "/s/claude-md-doctor",
                Source::Claude,
                "claude-md-doctor",
                RubricStatus::Missing,
            ),
        ])
    };
    let with = |job: GradingRubricJob| {
        let jobs = ["/s/none", "/s/broken", "/s/claude-md-doctor"]
            .into_iter()
            .map(|location| (location.to_owned(), job.clone()))
            .collect();
        let loaded = picker_with(found(), &["claude-md-doctor"], &[], &jobs);
        let rows = loaded
            .skills
            .iter()
            .map(|row| (row.folder.as_str().to_owned(), row.group))
            .collect::<Vec<_>>();
        (
            rows,
            (
                loaded.ready,
                loaded.yours,
                loaded.cant_score,
                loaded.on_scores,
            ),
        )
    };
    let refused = with(GradingRubricJob::Refused {
        job_id: id(),
        reason: "No.".into(),
    });
    assert_eq!(
        refused,
        (
            vec![
                ("/s/broken".to_owned(), GradingInstalledGroup::CantScore),
                ("/s/none".to_owned(), GradingInstalledGroup::CantScore),
                (
                    "/s/claude-md-doctor".to_owned(),
                    GradingInstalledGroup::OnScores
                ),
            ],
            (0, 0, 2, 1)
        )
    );
    // The reason is shown only where Try again is: never with the skill
    // Scores already has built in.
    let loaded = picker_with(
        found(),
        &["claude-md-doctor"],
        &[],
        &["/s/none", "/s/claude-md-doctor"]
            .into_iter()
            .map(|location| {
                (
                    location.to_owned(),
                    GradingRubricJob::Refused {
                        job_id: id(),
                        reason: "No.".into(),
                    },
                )
            })
            .collect(),
    );
    let shown: Vec<_> = loaded
        .skills
        .iter()
        .map(|row| (row.folder.as_str(), row.group, row.rubric_job.is_some()))
        .collect();
    assert_eq!(
        shown,
        [
            ("/s/broken", GradingInstalledGroup::Yours, false),
            ("/s/none", GradingInstalledGroup::CantScore, true),
            (
                "/s/claude-md-doctor",
                GradingInstalledGroup::OnScores,
                false
            ),
        ]
    );
    for job in [
        GradingRubricJob::Running {
            job_id: id(),
            started_at_ms: 0,
        },
        GradingRubricJob::Failed {
            job_id: id(),
            failure: GradingRubricFailure::RubricMissing,
        },
        GradingRubricJob::Cancelled { job_id: id() },
    ] {
        let (rows, counts) = with(job.clone());
        assert_eq!(
            rows.iter().map(|(_, group)| *group).collect::<Vec<_>>(),
            [
                GradingInstalledGroup::Yours,
                GradingInstalledGroup::Yours,
                GradingInstalledGroup::OnScores
            ],
            "{job:?}"
        );
        assert_eq!(counts, (0, 2, 0, 1));
    }
}
