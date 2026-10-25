//! The one rule for what the Scores page shows ([`overview`]).
//!
//! * **Shown score** of a (project, skill) pair: the score of the pair's
//!   newest succeeded run, only for a project in the current project list. A
//!   result that skipped a stage (`complete: false`) still counts and is
//!   marked partial.
//! * **Average** of a skill: the mean of its listed projects' shown scores,
//!   rounded to the nearest whole number with halves rounded up; none while
//!   no listed project has a shown score. `graded` counts the listed projects
//!   with one and `total` every listed project.
//! * **Top insights** of a skill: only from the shown runs. Findings are
//!   grouped by exact title and ranked by their most severe weight (critical,
//!   then warning, then info), then by how many projects have them, then by
//!   title; at most [`TOP_INSIGHTS`]. A `pass` finding is no problem: passes
//!   are listed, ranked the same way, only when there is no other finding.
//! * **Attempt** of a pair: its newest run when that is newer than the shown
//!   one: going now (with its start), or failed, cancelled or interrupted
//!   (with its stored reason).
//! * **Running**: only the stored status `running` says a run is going;
//!   progress events never decide it.
//! * **Waiting**: the service's line of gradings waiting for the one slot,
//!   read before the stored runs. An entry the read already has a run for
//!   (the run going now, or a pair's newest run) started in between and is
//!   not shown waiting. A pair's row names its entry and how many entries
//!   are before it in the whole line; a skill counts its entries on any
//!   project; an entry no row shows (its project is not listed, or its
//!   skill is not on Scores) is listed on its own, so it can be cancelled.
//!
//! The renderer formats these fields and works nothing out again.
use crate::{
    grading::{Waiting, exact_ms, stored_insights},
    grading_dto::{
        GradingAttempt, GradingFailure, GradingOverviewLoaded, GradingProjectRow,
        GradingRowWaiting, GradingRunningNow, GradingSeverity, GradingShownScore, GradingSkillInfo,
        GradingSkillOverview, GradingTopInsight, GradingWaitingElsewhere,
    },
    projects_dto::ProjectEntry,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use xt_store::grades::{GradePairs, GradeRow, GradeStatus};

/// Most findings a skill lists.
pub const TOP_INSIGHTS: usize = 3;

/// The (project path, skill ID) pairs [`overview`] reads, in the order it
/// expects them: skill by skill, each over every project in list order.
pub fn pairs<'a>(
    projects: &'a [ProjectEntry],
    skills: &'a [GradingSkillInfo],
) -> Vec<(&'a str, &'a str)> {
    skills
        .iter()
        .flat_map(|skill| {
            projects
                .iter()
                .map(|project| (project.path.as_str(), skill.id.as_str()))
        })
        .collect()
}

/// The Scores page's answer over a fresh project list, the skills, the
/// line of waiting gradings (oldest first, read before the runs) and the
/// stored runs of exactly [`pairs`] (read in one snapshot). `None` when the
/// read does not match the pairs or holds a time a renderer cannot read
/// exactly.
pub fn overview(
    projects: Vec<ProjectEntry>,
    skills: &[GradingSkillInfo],
    read: &GradePairs,
    line: &[Waiting],
) -> Option<GradingOverviewLoaded> {
    if read.pairs.len() != projects.len() * skills.len() {
        return None;
    }
    let line = still_waiting(read, line);
    let place = |project: &str, skill: &str| -> Option<Option<GradingRowWaiting>> {
        let Some(ahead) = line
            .iter()
            .position(|entry| entry.project_path == project && entry.skill_id == skill)
        else {
            return Some(None);
        };
        Some(Some(GradingRowWaiting {
            run_id: line[ahead].run_id.clone(),
            ahead: u32::try_from(ahead).ok()?,
        }))
    };
    let mut runs = read.pairs.iter();
    let mut out = Vec::with_capacity(skills.len());
    for skill in skills {
        let mut rows = Vec::with_capacity(projects.len());
        let mut shown_runs = Vec::new();
        for project in &projects {
            let pair = runs.next()?;
            let shown_run = pair.newest_succeeded.as_ref();
            let shown = match shown_run {
                Some(run) => Some(shown_score(run)?),
                None => None,
            };
            // Only a run that did not succeed can be newer than the newest
            // succeeded one.
            let attempt = match &pair.newest {
                Some(newest)
                    if newest.status != GradeStatus::Succeeded
                        && shown_run.is_none_or(|run| run.run_id != newest.run_id) =>
                {
                    Some(attempt(newest)?)
                }
                _ => None,
            };
            if let Some(run) = shown_run {
                shown_runs.push(run);
            }
            rows.push(GradingProjectRow {
                name: project.name.clone(),
                path: project.path.clone(),
                running: matches!(attempt, Some(GradingAttempt::Running { .. })),
                waiting: place(&project.path, &skill.id)?,
                shown,
                attempt,
            });
        }
        let scores: Vec<u8> = rows
            .iter()
            .filter_map(|row| row.shown.as_ref().map(|shown| shown.score))
            .collect();
        out.push(GradingSkillOverview {
            skill: skill.clone(),
            average: average(&scores),
            graded: u32::try_from(scores.len()).ok()?,
            total: u32::try_from(rows.len()).ok()?,
            top_insights: top_insights(&shown_runs),
            running: read
                .running
                .as_ref()
                .is_some_and(|run| run.skill_id == skill.id),
            waiting: u32::try_from(
                line.iter()
                    .filter(|entry| entry.skill_id == skill.id)
                    .count(),
            )
            .ok()?,
            rows,
        });
    }
    let running = match &read.running {
        Some(run) => Some(GradingRunningNow {
            run_id: run.run_id.clone(),
            skill_id: run.skill_id.clone(),
            skill_name: skills
                .iter()
                .find(|skill| skill.id == run.skill_id)
                .map(|skill| skill.name.clone()),
            project_path: run.project_path.clone(),
            project_name: projects
                .iter()
                .find(|project| project.path == run.project_path)
                .map(|project| project.name.clone()),
            started_at_ms: exact_ms(run.started_at_ms)?,
        }),
        None => None,
    };
    let skill_name = |id: &str| {
        skills
            .iter()
            .find(|skill| skill.id == id)
            .map(|skill| skill.name.clone())
    };
    let project_name = |path: &str| {
        projects
            .iter()
            .find(|project| project.path == path)
            .map(|project| project.name.clone())
    };
    let mut waiting_elsewhere = Vec::new();
    for (ahead, entry) in line.iter().enumerate() {
        let (skill_name, project_name) = (
            skill_name(&entry.skill_id),
            project_name(&entry.project_path),
        );
        // Every listed project has a row on every skill's page.
        if skill_name.is_some() && project_name.is_some() {
            continue;
        }
        waiting_elsewhere.push(GradingWaitingElsewhere {
            run_id: entry.run_id.clone(),
            skill_id: entry.skill_id.clone(),
            skill_name,
            project_path: entry.project_path.clone(),
            project_name,
            ahead: u32::try_from(ahead).ok()?,
        });
    }
    Some(GradingOverviewLoaded {
        projects,
        skills: out,
        running,
        // The service's own: a rubric-making job is not stored.
        making_rubric: None,
        waiting_elsewhere,
    })
}

/// The line without the entries the read already has a run for: they
/// started after the line was read.
fn still_waiting<'a>(read: &GradePairs, line: &'a [Waiting]) -> Vec<&'a Waiting> {
    let started: HashSet<&str> = read
        .running
        .iter()
        .chain(read.pairs.iter().filter_map(|pair| pair.newest.as_ref()))
        .map(|run| run.run_id.as_str())
        .collect();
    line.iter()
        .filter(|entry| !started.contains(entry.run_id.as_str()))
        .collect()
}

/// The mean, rounded half up; none for no scores.
pub fn average(scores: &[u8]) -> Option<u8> {
    if scores.is_empty() {
        return None;
    }
    let count = scores.len() as u64;
    let sum: u64 = scores.iter().map(|&score| u64::from(score)).sum();
    u8::try_from((2 * sum + count) / (2 * count)).ok()
}

fn shown_score(run: &GradeRow) -> Option<GradingShownScore> {
    Some(GradingShownScore {
        run_id: run.run_id.clone(),
        score: run.score?,
        graded_at_ms: exact_ms(run.finished_at_ms?)?,
        partial: run.complete == Some(false),
    })
}

/// A run that did not succeed, as an attempt; `None` for an unreadable time
/// (or a succeeded run, which is never one).
fn attempt(run: &GradeRow) -> Option<GradingAttempt> {
    let run_id = run.run_id.clone();
    let finished = || run.finished_at_ms.and_then(exact_ms);
    let reason = || {
        run.failure_reason
            .as_deref()
            .map_or(GradingFailure::Other, GradingFailure::from_code)
    };
    Some(match run.status {
        GradeStatus::Running => GradingAttempt::Running {
            run_id,
            started_at_ms: exact_ms(run.started_at_ms)?,
        },
        GradeStatus::Failed => GradingAttempt::Failed {
            run_id,
            finished_at_ms: finished()?,
            reason: reason(),
        },
        GradeStatus::Cancelled => GradingAttempt::Cancelled {
            run_id,
            finished_at_ms: finished()?,
        },
        GradeStatus::Interrupted => GradingAttempt::Interrupted {
            run_id,
            finished_at_ms: finished()?,
            reason: reason(),
        },
        GradeStatus::Succeeded => return None,
    })
}

/// How a finding weighs when ranked: higher first.
fn weight(severity: GradingSeverity) -> u8 {
    match severity {
        GradingSeverity::Critical => 3,
        GradingSeverity::Warning => 2,
        GradingSeverity::Info => 1,
        GradingSeverity::Pass => 0,
    }
}

fn top_insights(shown: &[&GradeRow]) -> Vec<GradingTopInsight> {
    // Title → (most severe weight, the shown runs that have it).
    let mut problems: BTreeMap<String, (GradingSeverity, BTreeSet<usize>)> = BTreeMap::new();
    let mut passes: BTreeMap<String, (GradingSeverity, BTreeSet<usize>)> = BTreeMap::new();
    for (index, run) in shown.iter().enumerate() {
        for insight in stored_insights(run.insights_json.as_deref()).unwrap_or_default() {
            let groups = if insight.severity == GradingSeverity::Pass {
                &mut passes
            } else {
                &mut problems
            };
            let group = groups
                .entry(insight.title)
                .or_insert((insight.severity, BTreeSet::new()));
            if weight(insight.severity) > weight(group.0) {
                group.0 = insight.severity;
            }
            group.1.insert(index);
        }
    }
    let groups = if problems.is_empty() {
        passes
    } else {
        problems
    };
    let mut ranked: Vec<GradingTopInsight> = groups
        .into_iter()
        .map(|(title, (severity, runs))| GradingTopInsight {
            severity,
            title,
            projects: u32::try_from(runs.len()).unwrap_or(u32::MAX),
        })
        .collect();
    ranked.sort_by(|a, b| {
        weight(b.severity)
            .cmp(&weight(a.severity))
            .then(b.projects.cmp(&a.projects))
            .then_with(|| a.title.cmp(&b.title))
    });
    ranked.truncate(TOP_INSIGHTS);
    ranked
}

#[cfg(test)]
#[path = "grading_overview_tests.rs"]
mod tests;
