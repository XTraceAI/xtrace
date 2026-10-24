//! The one rule for what a run's details show ([`details`]) and which file
//! "Open full report" opens ([`report_target`]).
//!
//! * **Which run.** Only a succeeded run has details: the run behind a
//!   project's shown score.
//! * **Summary and findings** are the stored row's. A succeeded run always
//!   stores both; after "Delete stored content" it has neither
//!   ([`content_deleted`]): its content is `deleted`, and nothing is read
//!   from its grade folder (the purge also removes every kept folder).
//!   Stored findings that do not read are `unreadable`, never "none".
//! * **Finding order.** Every stored finding, most severe first (critical,
//!   warning, info, then pass, by [`crate::grading_overview::weight`]), each
//!   weight in the run's own order.
//! * **Criteria and report** come from the run's kept grade folder,
//!   `<data>/grades/<run id>/` ([`read_kept`]): a real folder (not a link)
//!   whose `result.json` reads under the grader protocol
//!   ([`xt_judge::protocol::read`]: a regular file within its size cap, its
//!   report checked by [`xt_judge::protocol::report_path`]) and matches the
//!   stored run (score, completeness and report name). A folder that is gone
//!   was pruned
//!   (`folder_not_kept`); one that does not read or match is `unreadable`; a
//!   result without criteria, or a run without a report, did not report
//!   them (`not_reported`).
//! * **The report opened** is the resolved path the protocol's report rule
//!   ([`xt_judge::protocol::report_path`]) gives for it, inside the folder's
//!   resolved path. The renderer names only the run, never a path.
//!
//! The renderer formats these fields and works nothing out again.
use crate::{
    grading::{exact_ms, stored_insights},
    grading_dto::{
        GradingCriterionScore, GradingRunContent, GradingRunCriteria, GradingRunDetailsLoaded,
        GradingRunFindings, GradingRunGap, GradingRunKept, GradingRunReport,
    },
    grading_overview::weight,
};
use std::path::{Path, PathBuf};
use xt_judge::{GradeResult, protocol, staging::is_run_id};
use xt_store::grades::{GradeRow, GradeStatus};

/// What a succeeded run's grade folder holds, read once by [`read_kept`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptFolder {
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
pub fn content_deleted(row: &GradeRow) -> bool {
    row.summary.is_none() || row.insights_json.is_none()
}

/// Read the succeeded run `row`'s kept grade folder under `grades_dir`;
/// `None`, reading nothing, after its stored content was deleted.
pub fn read_kept(grades_dir: &Path, row: &GradeRow) -> Option<KeptFolder> {
    if content_deleted(row) {
        return None;
    }
    if !is_run_id(&row.run_id) {
        return Some(KeptFolder::Unreadable);
    }
    let folder = grades_dir.join(&row.run_id);
    match std::fs::symlink_metadata(&folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(KeptFolder::Missing);
        }
        Ok(meta) if meta.is_dir() => {}
        _ => return Some(KeptFolder::Unreadable),
    }
    let Ok(result) = protocol::read(&folder) else {
        return Some(KeptFolder::Unreadable);
    };
    if Some(result.score) != row.score
        || Some(result.complete) != row.complete
        || result.report != row.report_file
    {
        return Some(KeptFolder::Unreadable);
    }
    let report = match result.report.as_deref() {
        None => None,
        Some(relative) => match protocol::report_path(relative, &folder) {
            Ok(path) => Some(path),
            Err(_) => return Some(KeptFolder::Unreadable),
        },
    };
    Some(KeptFolder::Read { result, report })
}

/// Why the kept folder shows nothing; none when it was read.
fn gap(kept: &KeptFolder) -> Option<GradingRunGap> {
    match kept {
        KeptFolder::Missing => Some(GradingRunGap::FolderNotKept),
        KeptFolder::Unreadable => Some(GradingRunGap::Unreadable),
        KeptFolder::Read { .. } => None,
    }
}

/// The file "Open full report" opens for the succeeded run `row` whose
/// stored content is kept, or why there is none.
pub fn report_target<'a>(row: &GradeRow, kept: &'a KeptFolder) -> Result<&'a Path, GradingRunGap> {
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

/// The details of the succeeded run `row` with its kept folder (`None`
/// after its stored content was deleted). `None` for a run that did not
/// succeed, or one whose stored score or finish time a renderer cannot read.
pub fn details(row: &GradeRow, kept: Option<&KeptFolder>) -> Option<GradingRunDetailsLoaded> {
    if row.status != GradeStatus::Succeeded {
        return None;
    }
    let content = match (kept, row.summary.as_ref()) {
        (Some(kept), Some(summary)) => GradingRunContent::Kept(kept_content(row, kept, summary)),
        _ => GradingRunContent::Deleted,
    };
    Some(GradingRunDetailsLoaded {
        run_id: row.run_id.clone(),
        score: row.score?,
        graded_at_ms: exact_ms(row.finished_at_ms?)?,
        complete: row.complete?,
        content,
    })
}

fn kept_content(row: &GradeRow, kept: &KeptFolder, summary: &str) -> GradingRunKept {
    let findings = match stored_insights(row.insights_json.as_deref()) {
        Some(mut findings) => {
            // Stable: each weight keeps the run's own order.
            findings.sort_by_key(|finding| std::cmp::Reverse(weight(finding.severity)));
            GradingRunFindings::Listed { findings }
        }
        None => GradingRunFindings::Unreadable,
    };
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
    GradingRunKept {
        summary: summary.to_owned(),
        findings,
        criteria,
        report,
    }
}

#[cfg(test)]
#[path = "grading_details_tests.rs"]
mod tests;
