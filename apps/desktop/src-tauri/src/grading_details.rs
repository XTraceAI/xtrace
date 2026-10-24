//! The one rule for what a run's details show ([`details`]) and which file
//! "Open full report" opens ([`report_target`]).
//!
//! * **Which run.** Only a succeeded run has details: the run behind a
//!   project's shown score.
//! * **Summary and findings** are the stored row's. A succeeded run always
//!   stores both; after "Delete stored content" it has neither
//!   ([`KeptFolder::ContentDeleted`]), and nothing is read from its grade
//!   folder then (the purge also removes every kept folder).
//! * **Finding order.** Every stored finding, most severe first (critical,
//!   warning, info, then pass, by [`crate::grading_overview::weight`]), each
//!   weight in the run's own order.
//! * **Criteria and report** come from the run's kept grade folder,
//!   `<data>/grades/<run id>/` ([`read_kept`]): a real folder (not a link)
//!   whose `result.json` reads under the grader protocol
//!   ([`xt_judge::protocol::read`]: a regular file within its size cap; a
//!   report that is an `.html` file inside the folder, links resolved, and
//!   outside its work folder) and matches the stored run (score,
//!   completeness and report name). A folder that is gone was pruned
//!   (`folder_not_kept`); one that does not read or match is `unreadable`; a
//!   result without criteria, or a run without a report, did not report
//!   them (`not_reported`).
//! * **The report opened** is that checked file's resolved path, inside the
//!   folder's resolved path. The renderer names only the run, never a path.
//!
//! The renderer formats these fields and works nothing out again.
use crate::{
    grading::{exact_ms, stored_insights},
    grading_dto::{
        GradingCriterionScore, GradingRunCriteria, GradingRunDetailsLoaded, GradingRunGap,
        GradingRunReport,
    },
    grading_overview::weight,
};
use std::path::{Component, Path, PathBuf};
use xt_judge::{GradeResult, protocol};
use xt_store::grades::{GradeRow, GradeStatus};

/// What a succeeded run's grade folder holds, read once by [`read_kept`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptFolder {
    /// "Delete stored content" cleared the run; its folder is not read.
    ContentDeleted,
    /// The run has no grade folder any more: it was pruned.
    Missing,
    /// The folder is a link or not a folder, its result does not read, or
    /// the result does not match the stored run.
    Unreadable,
    /// The folder's checked result, and its report's resolved path when it
    /// has one.
    Read {
        result: GradeResult,
        report: Option<PathBuf>,
    },
}

/// Whether "Delete stored content" cleared this succeeded run: a succeeded
/// run always stores its summary and findings, and a purge clears both.
fn content_deleted(row: &GradeRow) -> bool {
    row.summary.is_none() || row.insights_json.is_none()
}

/// A run ID as the store keeps it: 36 characters of lowercase hex digits
/// and dashes, so it names one folder below the grades folder.
fn is_run_id(run_id: &str) -> bool {
    run_id.len() == 36
        && run_id
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Read the succeeded run `row`'s kept grade folder under `grades_dir`.
pub fn read_kept(grades_dir: &Path, row: &GradeRow) -> KeptFolder {
    if content_deleted(row) {
        return KeptFolder::ContentDeleted;
    }
    if !is_run_id(&row.run_id) {
        return KeptFolder::Unreadable;
    }
    let folder = grades_dir.join(&row.run_id);
    match std::fs::symlink_metadata(&folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return KeptFolder::Missing,
        Ok(meta) if meta.is_dir() => {}
        _ => return KeptFolder::Unreadable,
    }
    let Ok(result) = protocol::read(&folder) else {
        return KeptFolder::Unreadable;
    };
    if Some(result.score) != row.score
        || Some(result.complete) != row.complete
        || result.report != row.report_file
    {
        return KeptFolder::Unreadable;
    }
    let report = match result.report.as_deref() {
        None => None,
        Some(relative) => match inside(&folder, relative) {
            Some(path) => Some(path),
            None => return KeptFolder::Unreadable,
        },
    };
    KeptFolder::Read { result, report }
}

/// The resolved path of the regular file `relative` names inside `folder`:
/// only plain steps, the last one not a link, and resolved inside the
/// folder's own resolved path.
fn inside(folder: &Path, relative: &str) -> Option<PathBuf> {
    let steps = Path::new(relative);
    if relative.is_empty()
        || !steps
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let root = std::fs::canonicalize(folder).ok()?;
    let joined = root.join(steps);
    let resolved = std::fs::canonicalize(&joined).ok()?;
    let file = std::fs::symlink_metadata(&joined).is_ok_and(|meta| meta.is_file());
    (file && resolved != root && resolved.starts_with(&root)).then_some(resolved)
}

/// Why the kept folder shows nothing; none when it was read.
fn gap(kept: &KeptFolder) -> Option<GradingRunGap> {
    match kept {
        KeptFolder::ContentDeleted => Some(GradingRunGap::ContentDeleted),
        KeptFolder::Missing => Some(GradingRunGap::FolderNotKept),
        KeptFolder::Unreadable => Some(GradingRunGap::Unreadable),
        KeptFolder::Read { .. } => None,
    }
}

/// The file "Open full report" opens for the succeeded run `row`, or why
/// there is none.
pub fn report_target<'a>(row: &GradeRow, kept: &'a KeptFolder) -> Result<&'a Path, GradingRunGap> {
    if *kept == KeptFolder::ContentDeleted {
        return Err(GradingRunGap::ContentDeleted);
    }
    if row.report_file.is_none() {
        return Err(GradingRunGap::NotReported);
    }
    match kept {
        KeptFolder::Read {
            report: Some(path), ..
        } => Ok(path),
        other => Err(gap(other).unwrap_or(GradingRunGap::NotReported)),
    }
}

/// The details of the succeeded run `row` with its kept folder. `None` for
/// a run that did not succeed, or one whose stored score or finish time a
/// renderer cannot read.
pub fn details(row: &GradeRow, kept: &KeptFolder) -> Option<GradingRunDetailsLoaded> {
    if row.status != GradeStatus::Succeeded {
        return None;
    }
    let deleted = *kept == KeptFolder::ContentDeleted;
    let mut findings = if deleted {
        Vec::new()
    } else {
        stored_insights(row.insights_json.as_deref()).unwrap_or_default()
    };
    // Stable: each weight keeps the run's own order.
    findings.sort_by_key(|finding| std::cmp::Reverse(weight(finding.severity)));
    let criteria = match kept {
        KeptFolder::Read {
            result:
                GradeResult {
                    criteria: Some(criteria),
                    ..
                },
            ..
        } => GradingRunCriteria::Reported {
            criteria: criteria
                .iter()
                .map(|criterion| GradingCriterionScore {
                    name: criterion.name.clone(),
                    max: criterion.max,
                    awarded: criterion.awarded,
                })
                .collect(),
        },
        other => GradingRunCriteria::Unavailable {
            reason: gap(other).unwrap_or(GradingRunGap::NotReported),
        },
    };
    let report = match report_target(row, kept) {
        Ok(_) => GradingRunReport::Kept,
        Err(reason) => GradingRunReport::Unavailable { reason },
    };
    Some(GradingRunDetailsLoaded {
        run_id: row.run_id.clone(),
        project_path: row.project_path.clone(),
        skill_id: row.skill_id.clone(),
        skill_version: row.skill_version.clone(),
        score: row.score?,
        graded_at_ms: exact_ms(row.finished_at_ms?)?,
        complete: row.complete?,
        content_deleted: deleted,
        summary: if deleted { None } else { row.summary.clone() },
        findings,
        criteria,
        report,
    })
}

#[cfg(test)]
#[path = "grading_details_tests.rs"]
mod tests;
