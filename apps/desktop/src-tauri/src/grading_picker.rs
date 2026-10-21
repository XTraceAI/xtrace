//! The one rule for what the "Add a skill" dialog lists ([`picker`]).
//!
//! * **Group** of an installed skill: ready (its folder opens as a grading
//!   skill with a valid `xtrace-rubric.md`), rubric problem (its rubric is
//!   refused, with the typed reason), or needs a rubric (it has none), as
//!   [`xt_judge::installed`] read it.
//! * **Key**: a ready skill's rubric id; without a valid rubric, its
//!   `SKILL.md` name.
//! * **On Scores**: a skill whose location (its folder, or a plugin's skill
//!   by plugin and name: [`xt_judge::installed::Location`]) is an added
//!   skill's is on Scores. Otherwise one whose key is a built-in skill's id
//!   is marked built in, and one whose key is an added skill's id (from
//!   elsewhere) is marked as that id being on Scores. None of these can be
//!   added.
//! * **Clash**: of ready skills that share a rubric id not yet on Scores,
//!   only the first in the order below can be added; each later one names
//!   the first.
//! * **Order**: ready first, then rubric problems, then needs a rubric; by
//!   name within each (ignoring case), then by folder. Rows are then listed
//!   by display group (below), keeping this order within each.
//! * **Rubric action**: a skill that needs a rubric can have one made, and
//!   one whose rubric has a problem can have it made again; a ready one
//!   neither, and neither can one marked built in or whose key is an added
//!   skill's (from elsewhere): Scores already has that skill. One on Scores
//!   itself can (a remake keeps its card's id).
//! * **Rubric job**: its latest rubric-making job (by location) is shown
//!   with it while that job still describes its rubric: one going always; a
//!   made rubric only while the skill is ready; a failure, refusal or
//!   cancel only while it is not. A refusal comes from the kept refusals,
//!   given here only while still in force ([`xt_judge::refusals::in_force`]),
//!   and is shown only with a skill that has a rubric action (so only in
//!   "yours" or "can't be scored", never with one Scores already has).
//! * **Display group** ([`GradingInstalledGroup`]): a skill a rubric can be
//!   made for is "can't be scored" while its job is a refusal, otherwise
//!   "yours"; one without a rubric action is "ready" when it is addable or
//!   clashes with a ready skill listed before it, otherwise "already on
//!   Scores" (built in, added, or its id on Scores from elsewhere).
//!
//! Adding re-runs exactly this rule over a fresh discovery and accepts only
//! a location it marks addable ([`crate::grading::GradingService::add_skill`]);
//! making a rubric accepts only a location it offers a rubric action for
//! ([`crate::grading::GradingService::make_rubric`]).
//! The renderer shows these fields and works nothing out again.
use crate::grading_dto::{
    GradingInstalledGroup, GradingInstalledLoaded, GradingInstalledRubric, GradingInstalledScores,
    GradingInstalledSkill, GradingRubricAction, GradingRubricJob, GradingSkillSource,
};
use std::collections::HashMap;
use xt_judge::installed::{Discovery, InstalledSkill, RubricStatus, Source};
use xt_store::added_skills::AddedSkill;

/// The dialog's answer over one discovery, the built-in skills' ids and the
/// added skills, with no rubric-making job.
pub fn picker(found: Discovery, built_in: &[&str], added: &[AddedSkill]) -> GradingInstalledLoaded {
    picker_with(found, built_in, added, &HashMap::new())
}

/// [`picker`], with each location's latest rubric-making job (a kept
/// refusal only while it is in force).
pub fn picker_with(
    found: Discovery,
    built_in: &[&str],
    added: &[AddedSkill],
    jobs: &HashMap<String, GradingRubricJob>,
) -> GradingInstalledLoaded {
    let mut rows: Vec<(u8, GradingInstalledSkill)> = found
        .skills
        .into_iter()
        .filter_map(|skill| row(skill).map(|row| (group(&row.rubric), row)))
        .collect();
    rows.sort_by(|(a_group, a), (b_group, b)| {
        a_group
            .cmp(b_group)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.folder.cmp(&b.folder))
    });
    // The first ready skill with each rubric id: (name, folder).
    let mut first: HashMap<String, (String, String)> = HashMap::new();
    let mut skills = Vec::with_capacity(rows.len());
    for (_, mut row) in rows {
        let key = match &row.rubric {
            GradingInstalledRubric::Ready { id } => id.as_str(),
            _ => row.name.as_str(),
        };
        row.scores = if added.iter().any(|skill| skill.location == row.location) {
            GradingInstalledScores::OnScores
        } else if built_in.contains(&key) {
            GradingInstalledScores::BuiltIn
        } else if let Some(skill) = added.iter().find(|skill| skill.skill_id == key) {
            GradingInstalledScores::IdOnScores {
                location: skill.location.clone(),
            }
        } else if let GradingInstalledRubric::Ready { id } = &row.rubric {
            match first.get(id) {
                Some((name, folder)) => GradingInstalledScores::IdClash {
                    name: name.clone(),
                    folder: folder.clone(),
                },
                None => {
                    first.insert(id.clone(), (row.name.clone(), row.folder.clone()));
                    GradingInstalledScores::Addable
                }
            }
        } else {
            GradingInstalledScores::NotReady
        };
        row.rubric_action = match (&row.rubric, &row.scores) {
            (GradingInstalledRubric::Ready { .. }, _)
            | (_, GradingInstalledScores::BuiltIn | GradingInstalledScores::IdOnScores { .. }) => {
                None
            }
            (GradingInstalledRubric::Problem { .. }, _) => Some(GradingRubricAction::Remake),
            (GradingInstalledRubric::Missing, _) => Some(GradingRubricAction::Make),
        };
        // A refusal only where it is offered again (Try again): a skill
        // Scores already has shows none.
        row.rubric_job = jobs
            .get(&row.location)
            .filter(|job| job_matches(job, &row.rubric))
            .filter(|job| {
                row.rubric_action.is_some() || !matches!(job, GradingRubricJob::Refused { .. })
            })
            .cloned();
        row.group = display_group(&row);
        skills.push(row);
    }
    // Stable: the order above holds within each group.
    skills.sort_by_key(|row| row.group);
    let count = |group: GradingInstalledGroup| {
        u32::try_from(skills.iter().filter(|row| row.group == group).count()).unwrap_or(u32::MAX)
    };
    GradingInstalledLoaded {
        ready: count(GradingInstalledGroup::Ready),
        yours: count(GradingInstalledGroup::Yours),
        cant_score: count(GradingInstalledGroup::CantScore),
        on_scores: count(GradingInstalledGroup::OnScores),
        skills,
        skipped: found.skipped,
        incomplete: found.incomplete,
        plugins_unread: found.plugins_unread.map(Into::into),
    }
}

/// The dialog group of a row whose scores state, rubric action and job are
/// decided.
fn display_group(row: &GradingInstalledSkill) -> GradingInstalledGroup {
    if row.rubric_action.is_some() {
        if matches!(row.rubric_job, Some(GradingRubricJob::Refused { .. })) {
            GradingInstalledGroup::CantScore
        } else {
            GradingInstalledGroup::Yours
        }
    } else if matches!(
        row.scores,
        GradingInstalledScores::Addable | GradingInstalledScores::IdClash { .. }
    ) {
        GradingInstalledGroup::Ready
    } else {
        GradingInstalledGroup::OnScores
    }
}

/// Whether a location's latest rubric-making job still describes its rubric
/// now: one going always; a made rubric while it is ready; a failure, a
/// refusal or a cancel while it is not.
fn job_matches(job: &GradingRubricJob, rubric: &GradingInstalledRubric) -> bool {
    let ready = matches!(rubric, GradingInstalledRubric::Ready { .. });
    match job {
        GradingRubricJob::Running { .. } => true,
        GradingRubricJob::Made { .. } => ready,
        GradingRubricJob::Failed { .. }
        | GradingRubricJob::Refused { .. }
        | GradingRubricJob::Cancelled { .. } => !ready,
    }
}

/// Ready, rubric problem, needs a rubric.
fn group(rubric: &GradingInstalledRubric) -> u8 {
    match rubric {
        GradingInstalledRubric::Ready { .. } => 0,
        GradingInstalledRubric::Problem { .. } => 1,
        GradingInstalledRubric::Missing => 2,
    }
}

/// One discovered skill as a row, its scores state still to be decided;
/// none for a folder whose path is not UTF-8 (discovery lists none).
fn row(skill: InstalledSkill) -> Option<GradingInstalledSkill> {
    let folder = skill.folder.to_str()?.to_owned();
    let location = skill.location.text()?;
    let (rubric, name, description) = match skill.rubric {
        RubricStatus::Ready(rubric) => (
            GradingInstalledRubric::Ready { id: rubric.id },
            rubric.name,
            rubric.description,
        ),
        RubricStatus::Problem(reason) => (
            GradingInstalledRubric::Problem {
                reason: reason.into(),
            },
            skill.name,
            skill.description,
        ),
        RubricStatus::Missing => (
            GradingInstalledRubric::Missing,
            skill.name,
            skill.description,
        ),
    };
    Some(GradingInstalledSkill {
        folder,
        location,
        name,
        description,
        source: match skill.source {
            Source::Claude => GradingSkillSource::Claude,
            Source::Agents => GradingSkillSource::Agents,
            Source::Codex => GradingSkillSource::Codex,
            Source::Plugin(plugin) => GradingSkillSource::Plugin { plugin },
        },
        rubric,
        rubric_kept_by_xtrace: skill.rubric_kept_by_xtrace,
        scores: GradingInstalledScores::NotReady,
        group: GradingInstalledGroup::Yours,
        rubric_action: None,
        rubric_job: None,
    })
}

#[cfg(test)]
#[path = "grading_picker_tests.rs"]
mod tests;
