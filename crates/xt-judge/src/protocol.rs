//! XTrace grader protocol v1: the `result.json` a grading skill writes into
//! its grade folder, and the one strict reader of it.
//!
//! ```json
//! {"protocol": 1, "score": 0-100, "summary": "…",
//!  "insights": [{"severity": "critical"|"warning"|"info"|"pass",
//!                "title": "…", "detail": "…", "fix": "…"}],
//!  "criteria": [{"name": "…", "max": 1-100, "awarded": 0-max}],
//!  "report": "report.html" | null, "complete": true}
//! ```
//!
//! * The file is read only as a regular file of at most
//!   [`MAX_RESULT_BYTES`], opened without following a symbolic link and
//!   without blocking, and checked on the opened handle
//!   ([`xt_probes::regular_file`]). It must be one JSON object with exactly
//!   these keys (`detail` and `fix` may be left out of an insight).
//! * `score` is an integer from 0 to 100. The run's final XTrace step
//!   computes it from the skill's rubric; XTrace never maps a letter or
//!   recomputes it.
//! * Every text has a length cap in characters; at most [`MAX_INSIGHTS`]
//!   insights. The run's prompt states the same numbers to the step that
//!   writes the file ([`crate::skill::GradingSkill::prompt`]).
//! * `report`, when present, is a relative path to an existing regular
//!   `.html` file inside the grade folder, symlinks resolved, and not inside
//!   its work folder ([`crate::runner::WORK_DIR`]), which holds the exam's
//!   working files, transcript excerpts included.
//! * `complete` is required: false when the skill skipped a stage.
//! * `criteria` is optional (results written before rubrics have none):
//!   1 to [`crate::rubric::MAX_CRITERIA`] objects with exactly `name`,
//!   `max` (1-100) and `awarded` (0 to `max`); the `max` values add up to
//!   100 and the `awarded` values to `score`. A rubric run must report
//!   exactly its rubric's criteria, in order ([`check_rubric`]).
//!
//! Anything else is a typed [`ResultError`]. Error messages hold no path and
//! no text from the file.

use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Component, Path};
use xt_probes::regular_file::{self, RegularFileError};

/// The protocol version this reader accepts.
pub const PROTOCOL: u64 = 1;
/// The result file's name inside the grade folder.
pub const RESULT_FILE: &str = "result.json";
/// Largest accepted result file.
pub const MAX_RESULT_BYTES: u64 = 1024 * 1024;
pub const MAX_SUMMARY_CHARS: usize = 1000;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DETAIL_CHARS: usize = 2000;
pub const MAX_FIX_CHARS: usize = 2000;
pub const MAX_INSIGHTS: usize = 100;
/// Longest accepted `report` path, in bytes.
pub const MAX_REPORT_PATH_BYTES: usize = 512;
/// Highest score.
pub const MAX_SCORE: u64 = 100;
/// The only accepted report file extension.
pub const REPORT_EXTENSION: &str = "html";

/// How a finding is weighed, in the protocol's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    Warning,
    Info,
    Pass,
}

impl Severity {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "critical" => Self::Critical,
            "warning" => Self::Warning,
            "info" => Self::Info,
            "pass" => Self::Pass,
            _ => return None,
        })
    }
}

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Insight {
    pub severity: Severity,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// A validated grading result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GradeResult {
    pub score: u8,
    pub summary: String,
    pub insights: Vec<Insight>,
    /// Relative to the grade folder.
    pub report: Option<String>,
    pub complete: bool,
    /// The points each rubric criterion was awarded, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<CriterionScore>>,
}

/// The points one rubric criterion was awarded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CriterionScore {
    pub name: String,
    pub max: u8,
    pub awarded: u8,
}

/// Why a result was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResultError {
    #[error("the grading run wrote no result file")]
    Missing,
    #[error("the result is not a regular file")]
    NotRegularFile,
    #[error("the result file is larger than the protocol allows")]
    TooLarge,
    #[error("the result file could not be read")]
    Unreadable,
    #[error("the result is not valid JSON")]
    NotJson,
    #[error("the result is not a JSON object")]
    NotObject,
    #[error("the result has a field the protocol does not define")]
    UnknownField,
    #[error("the result does not declare protocol 1")]
    Protocol,
    #[error("the result score is not a whole number from 0 to 100")]
    Score,
    #[error("the result summary is missing or too long")]
    Summary,
    #[error("the result insights are not a list")]
    Insights,
    #[error("the result has more insights than the protocol allows")]
    TooManyInsights,
    #[error("a result insight is not a valid object")]
    Insight,
    #[error("a result insight has an unknown severity")]
    Severity,
    #[error("a result insight title is missing, empty or too long")]
    Title,
    #[error("a result insight detail is not text or is too long")]
    Detail,
    #[error("a result insight fix is not text or is too long")]
    Fix,
    #[error("the result report path is not a valid relative path")]
    ReportPath,
    #[error("the result report is not a file inside the grade folder")]
    ReportOutside,
    #[error("the result report is not an HTML file")]
    ReportNotHtml,
    #[error("the result report is inside the work folder")]
    ReportInWorkFolder,
    #[error("the result does not say whether the run was complete")]
    Complete,
    #[error("the result criteria are not a valid list")]
    Criteria,
    #[error("the result criteria do not add up to 100 points and to the score")]
    CriteriaTotal,
    #[error("the result criteria are missing or are not the rubric's")]
    CriteriaRubric,
}

/// Read and validate `<grade_dir>/result.json`.
pub fn read(grade_dir: &Path) -> Result<GradeResult, ResultError> {
    let bytes = regular_file::read_capped(&grade_dir.join(RESULT_FILE), MAX_RESULT_BYTES).map_err(
        |error| match error {
            RegularFileError::Missing => ResultError::Missing,
            RegularFileError::NotRegular => ResultError::NotRegularFile,
            RegularFileError::TooLarge => ResultError::TooLarge,
            RegularFileError::Unreadable => ResultError::Unreadable,
        },
    )?;
    parse(&bytes, grade_dir)
}

/// Validate result bytes against the protocol. `grade_dir` anchors the
/// `report` path.
pub fn parse(bytes: &[u8], grade_dir: &Path) -> Result<GradeResult, ResultError> {
    if bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err(ResultError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ResultError::NotJson)?;
    let Value::Object(object) = value else {
        return Err(ResultError::NotObject);
    };
    const KEYS: [&str; 7] = [
        "protocol", "score", "summary", "insights", "report", "complete", "criteria",
    ];
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(ResultError::UnknownField);
    }
    if object.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL) {
        return Err(ResultError::Protocol);
    }
    let score = object
        .get("score")
        .and_then(Value::as_u64)
        .filter(|score| *score <= MAX_SCORE)
        .and_then(|score| u8::try_from(score).ok())
        .ok_or(ResultError::Score)?;
    let summary = text(object.get("summary"), MAX_SUMMARY_CHARS).ok_or(ResultError::Summary)?;
    let Some(Value::Array(items)) = object.get("insights") else {
        return Err(ResultError::Insights);
    };
    if items.len() > MAX_INSIGHTS {
        return Err(ResultError::TooManyInsights);
    }
    let insights = items.iter().map(insight).collect::<Result<Vec<_>, _>>()?;
    let report = match object.get("report") {
        Some(Value::Null) => None,
        Some(Value::String(path)) => Some(report(path, grade_dir)?),
        _ => return Err(ResultError::ReportPath),
    };
    let complete = object
        .get("complete")
        .and_then(Value::as_bool)
        .ok_or(ResultError::Complete)?;
    let criteria = match object.get("criteria") {
        None => None,
        Some(value) => Some(criteria(value, score)?),
    };
    Ok(GradeResult {
        score,
        summary,
        insights,
        report,
        complete,
        criteria,
    })
}

/// The `criteria` list: well formed, `max` adding up to the whole score
/// range and `awarded` to the score.
fn criteria(value: &Value, score: u8) -> Result<Vec<CriterionScore>, ResultError> {
    let Value::Array(items) = value else {
        return Err(ResultError::Criteria);
    };
    if items.is_empty() || items.len() > crate::rubric::MAX_CRITERIA {
        return Err(ResultError::Criteria);
    }
    let criteria = items
        .iter()
        .map(|item| {
            let Value::Object(object) = item else {
                return None;
            };
            if object.len() != 3 {
                return None;
            }
            let name = text(object.get("name"), crate::rubric::MAX_CRITERION_CHARS)
                .filter(|name| !name.trim().is_empty())?;
            let max = object
                .get("max")
                .and_then(Value::as_u64)
                .filter(|max| (1..=MAX_SCORE).contains(max))?;
            let awarded = object
                .get("awarded")
                .and_then(Value::as_u64)
                .filter(|awarded| *awarded <= max)?;
            Some(CriterionScore {
                name,
                max: u8::try_from(max).ok()?,
                awarded: u8::try_from(awarded).ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(ResultError::Criteria)?;
    let total = |pick: fn(&CriterionScore) -> u8| -> u64 {
        criteria
            .iter()
            .map(|criterion| u64::from(pick(criterion)))
            .sum()
    };
    if total(|criterion| criterion.max) != MAX_SCORE
        || total(|criterion| criterion.awarded) != u64::from(score)
    {
        return Err(ResultError::CriteriaTotal);
    }
    Ok(criteria)
}

/// A rubric run's result reports exactly the rubric's criteria: the same
/// names and points, in the rubric's order.
pub fn check_rubric(
    result: &GradeResult,
    rubric: &crate::rubric::Rubric,
) -> Result<(), ResultError> {
    let reported = result
        .criteria
        .as_deref()
        .ok_or(ResultError::CriteriaRubric)?;
    let same = reported.len() == rubric.criteria.len()
        && reported
            .iter()
            .zip(&rubric.criteria)
            .all(|(score, criterion)| {
                score.name == criterion.name && score.max == criterion.points
            });
    same.then_some(()).ok_or(ResultError::CriteriaRubric)
}

/// A string of at most `cap` characters.
fn text(value: Option<&Value>, cap: usize) -> Option<String> {
    match value {
        Some(Value::String(text)) if text.chars().count() <= cap => Some(text.clone()),
        _ => None,
    }
}

/// An optional string: absent is `None`, anything but a short string fails.
fn optional_text(
    object: &Map<String, Value>,
    key: &str,
    cap: usize,
    error: ResultError,
) -> Result<Option<String>, ResultError> {
    match object.get(key) {
        None => Ok(None),
        value => text(value, cap).map(Some).ok_or(error),
    }
}

fn insight(value: &Value) -> Result<Insight, ResultError> {
    let Value::Object(object) = value else {
        return Err(ResultError::Insight);
    };
    const KEYS: [&str; 4] = ["severity", "title", "detail", "fix"];
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(ResultError::Insight);
    }
    let severity = object
        .get("severity")
        .and_then(Value::as_str)
        .and_then(Severity::parse)
        .ok_or(ResultError::Severity)?;
    let title = text(object.get("title"), MAX_TITLE_CHARS)
        .filter(|title| !title.trim().is_empty())
        .ok_or(ResultError::Title)?;
    Ok(Insight {
        severity,
        title,
        detail: optional_text(object, "detail", MAX_DETAIL_CHARS, ResultError::Detail)?,
        fix: optional_text(object, "fix", MAX_FIX_CHARS, ResultError::Fix)?,
    })
}

/// A relative path naming an existing regular `.html` file inside the grade
/// folder and outside its work folder.
fn report(path: &str, grade_dir: &Path) -> Result<String, ResultError> {
    let relative = Path::new(path);
    if path.is_empty()
        || path.len() > MAX_REPORT_PATH_BYTES
        || path.contains('\0')
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ResultError::ReportPath);
    }
    if relative.extension().and_then(|ext| ext.to_str()) != Some(REPORT_EXTENSION) {
        return Err(ResultError::ReportNotHtml);
    }
    if in_work_folder(relative) {
        return Err(ResultError::ReportInWorkFolder);
    }
    let outside = || ResultError::ReportOutside;
    let root = std::fs::canonicalize(grade_dir).map_err(|_| outside())?;
    let joined = root.join(relative);
    let resolved = std::fs::canonicalize(&joined).map_err(|_| outside())?;
    let is_file = std::fs::symlink_metadata(&joined).is_ok_and(|meta| meta.is_file());
    let Some(inside) = resolved
        .strip_prefix(&root)
        .ok()
        .filter(|_| resolved != root)
    else {
        return Err(outside());
    };
    if !is_file {
        return Err(outside());
    }
    // A subfolder that is a link into the work folder resolves into it.
    if in_work_folder(inside) {
        return Err(ResultError::ReportInWorkFolder);
    }
    Ok(path.to_owned())
}

/// Whether a path relative to the grade folder lies in its work folder.
/// Compared without regard to case: the default macOS file system treats
/// `Work` and `work` as one folder.
fn in_work_folder(relative: &Path) -> bool {
    let mut components = relative.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(first)), Some(_)) => first
            .to_str()
            .is_none_or(|first| first.eq_ignore_ascii_case(crate::runner::WORK_DIR)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
            .unwrap()
    }

    fn good() -> Value {
        serde_json::json!({
            "protocol": 1, "score": 75, "summary": "One dead reference.",
            "insights": [
                {"severity": "critical", "title": "Dead path", "detail": "d", "fix": "f"},
                {"severity": "warning", "title": "Long"},
                {"severity": "info", "title": "Note"},
                {"severity": "pass", "title": "Fine"}
            ],
            "report": null, "complete": true
        })
    }

    fn check(value: &Value) -> Result<GradeResult, ResultError> {
        let root = dir();
        parse(&serde_json::to_vec(value).unwrap(), root.path())
    }

    fn with(key: &str, value: Value) -> Result<GradeResult, ResultError> {
        let mut result = good();
        result[key] = value;
        check(&result)
    }

    #[test]
    fn accepts_the_protocol_shape() {
        let result = check(&good()).unwrap();
        assert_eq!(result.score, 75);
        assert_eq!(result.insights.len(), 4);
        assert_eq!(result.insights[0].severity, Severity::Critical);
        assert_eq!(result.insights[1].detail, None);
        assert!(result.complete);
        assert_eq!(with("score", 0.into()).unwrap().score, 0);
        assert_eq!(with("score", 100.into()).unwrap().score, 100);
    }

    #[test]
    fn refuses_bad_top_levels() {
        let root = dir();
        assert_eq!(parse(b"not json", root.path()), Err(ResultError::NotJson));
        assert_eq!(parse(b"[1]", root.path()), Err(ResultError::NotObject));
        assert_eq!(parse(b"\"x\"", root.path()), Err(ResultError::NotObject));
        let big = vec![b' '; MAX_RESULT_BYTES as usize + 1];
        assert_eq!(parse(&big, root.path()), Err(ResultError::TooLarge));
        assert_eq!(with("extra", 1.into()), Err(ResultError::UnknownField));
        assert_eq!(with("protocol", 2.into()), Err(ResultError::Protocol));
        assert_eq!(with("protocol", "1".into()), Err(ResultError::Protocol));
        let mut missing = good();
        missing.as_object_mut().unwrap().remove("complete");
        assert_eq!(check(&missing), Err(ResultError::Complete));
        assert_eq!(with("complete", "yes".into()), Err(ResultError::Complete));
    }

    #[test]
    fn score_is_a_whole_number_in_range() {
        for bad in [
            serde_json::json!(101),
            serde_json::json!(-1),
            serde_json::json!(50.5),
            serde_json::json!(50.0),
            serde_json::json!("50"),
            Value::Null,
        ] {
            assert_eq!(with("score", bad), Err(ResultError::Score));
        }
    }

    #[test]
    fn texts_and_insights_are_capped() {
        let long = |n: usize| Value::String("é".repeat(n));
        assert!(with("summary", long(MAX_SUMMARY_CHARS)).is_ok());
        assert_eq!(
            with("summary", long(MAX_SUMMARY_CHARS + 1)),
            Err(ResultError::Summary)
        );
        let insight = |extra: Value| {
            let mut item = serde_json::json!({"severity": "info", "title": "t"});
            for (key, value) in extra.as_object().unwrap() {
                item[key] = value.clone();
            }
            with("insights", Value::Array(vec![item]))
        };
        assert_eq!(
            insight(serde_json::json!({"title": long(MAX_TITLE_CHARS + 1)})),
            Err(ResultError::Title)
        );
        assert_eq!(
            insight(serde_json::json!({"title": "  "})),
            Err(ResultError::Title)
        );
        assert_eq!(
            insight(serde_json::json!({"detail": long(MAX_DETAIL_CHARS + 1)})),
            Err(ResultError::Detail)
        );
        assert_eq!(
            insight(serde_json::json!({"fix": 3})),
            Err(ResultError::Fix)
        );
        assert_eq!(
            insight(serde_json::json!({"severity": "warn"})),
            Err(ResultError::Severity)
        );
        assert_eq!(
            insight(serde_json::json!({"other": 1})),
            Err(ResultError::Insight)
        );
        let many = vec![serde_json::json!({"severity": "info", "title": "t"}); MAX_INSIGHTS + 1];
        assert_eq!(
            with("insights", Value::Array(many)),
            Err(ResultError::TooManyInsights)
        );
        assert_eq!(with("insights", 1.into()), Err(ResultError::Insights));
        assert_eq!(
            with("insights", serde_json::json!([1])),
            Err(ResultError::Insight)
        );
    }

    #[cfg(unix)]
    #[test]
    fn report_must_be_a_file_inside_the_grade_folder() {
        let root = dir();
        let grade = root.path().join("grade");
        std::fs::create_dir_all(grade.join("sub")).unwrap();
        std::fs::create_dir_all(grade.join("work")).unwrap();
        std::fs::write(grade.join("report.html"), "<p>").unwrap();
        std::fs::write(grade.join("work.html"), "<p>").unwrap();
        std::fs::write(grade.join("sub/r.html"), "<p>").unwrap();
        std::fs::write(grade.join("work/excerpts.html"), "<p>").unwrap();
        std::fs::write(grade.join("notes.txt"), "x").unwrap();
        std::os::unix::fs::symlink(grade.join("work"), grade.join("alias")).unwrap();
        std::fs::write(root.path().join("outside.html"), "<p>").unwrap();
        std::os::unix::fs::symlink(root.path().join("outside.html"), grade.join("link.html"))
            .unwrap();
        std::os::unix::fs::symlink(root.path(), grade.join("up")).unwrap();
        let parse_report = |path: &str| {
            let mut result = good();
            result["report"] = path.into();
            parse(&serde_json::to_vec(&result).unwrap(), &grade).map(|result| result.report)
        };
        assert_eq!(parse_report("report.html"), Ok(Some("report.html".into())));
        assert_eq!(parse_report("sub/r.html"), Ok(Some("sub/r.html".into())));
        assert_eq!(parse_report("work.html"), Ok(Some("work.html".into())));
        for not_html in [
            "notes.txt",
            "sub",
            "report.HTML",
            "report.html.txt",
            "report",
        ] {
            assert_eq!(
                parse_report(not_html),
                Err(ResultError::ReportNotHtml),
                "{not_html}"
            );
        }
        for in_work in [
            "work/excerpts.html",
            "Work/excerpts.html",
            "alias/excerpts.html",
        ] {
            assert_eq!(
                parse_report(in_work),
                Err(ResultError::ReportInWorkFolder),
                "{in_work}"
            );
        }
        for bad in [
            "../outside.html",
            "/etc/hosts",
            "",
            "./report.html",
            "sub/../report.html",
        ] {
            assert_eq!(parse_report(bad), Err(ResultError::ReportPath), "{bad}");
        }
        for outside in ["link.html", "up/outside.html", "missing.html"] {
            assert_eq!(
                parse_report(outside),
                Err(ResultError::ReportOutside),
                "{outside}"
            );
        }
        let mut result = good();
        result["report"] = 3.into();
        assert_eq!(
            parse(&serde_json::to_vec(&result).unwrap(), &grade),
            Err(ResultError::ReportPath)
        );
    }

    #[test]
    fn criteria_are_optional_and_checked_when_present() {
        assert_eq!(check(&good()).unwrap().criteria, None);
        let two = serde_json::json!([
            {"name": "Accuracy", "max": 60, "awarded": 50},
            {"name": "Style", "max": 40, "awarded": 25}
        ]);
        let result = with("criteria", two.clone()).unwrap();
        assert_eq!(
            result.criteria,
            Some(vec![
                CriterionScore {
                    name: "Accuracy".into(),
                    max: 60,
                    awarded: 50
                },
                CriterionScore {
                    name: "Style".into(),
                    max: 40,
                    awarded: 25
                },
            ])
        );
        let serialized = serde_json::to_value(&result).unwrap();
        assert_eq!(serialized["criteria"][1]["awarded"], 25);
        assert!(
            serde_json::to_value(check(&good()).unwrap())
                .unwrap()
                .get("criteria")
                .is_none()
        );
        let shaped = |criteria: Value| with("criteria", criteria);
        for bad in [
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!([1]),
            serde_json::json!([{"name": "A", "max": 100}]),
            serde_json::json!([{"name": "A", "max": 100, "awarded": 75, "extra": 1}]),
            serde_json::json!([{"name": " ", "max": 100, "awarded": 75}]),
            serde_json::json!([{"name": "A", "max": 0, "awarded": 0},
                               {"name": "B", "max": 100, "awarded": 75}]),
            serde_json::json!([{"name": "A", "max": 101, "awarded": 75}]),
            serde_json::json!([{"name": "A", "max": 50, "awarded": 51},
                               {"name": "B", "max": 50, "awarded": 24}]),
            serde_json::json!([{"name": "A", "max": 100, "awarded": 75.0}]),
            serde_json::json!([{"name": "A", "max": 100, "awarded": -1}]),
            Value::Array(vec![
                serde_json::json!({"name": "A", "max": 5, "awarded": 0});
                21
            ]),
        ] {
            assert_eq!(shaped(bad.clone()), Err(ResultError::Criteria), "{bad}");
        }
        for bad in [
            serde_json::json!([{"name": "A", "max": 90, "awarded": 75}]),
            serde_json::json!([{"name": "A", "max": 100, "awarded": 74}]),
            serde_json::json!([{"name": "A", "max": 60, "awarded": 60},
                               {"name": "B", "max": 40, "awarded": 40}]),
        ] {
            assert_eq!(
                shaped(bad.clone()),
                Err(ResultError::CriteriaTotal),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_rubric_run_reports_exactly_the_rubric_criteria() {
        let rubric = crate::rubric::parse(
            b"---\nprotocol: 1\nid: t\nname: T\nversion: 1\ndescription: d\ntarget: project\n---\n\
              ## Accuracy (60 points)\nA.\n## Style (40 points)\nS.\n",
        )
        .unwrap();
        let run = |criteria: Value| {
            let mut result = good();
            result["criteria"] = criteria;
            check_rubric(&check(&result).unwrap(), &rubric)
        };
        assert_eq!(
            run(serde_json::json!([
                {"name": "Accuracy", "max": 60, "awarded": 50},
                {"name": "Style", "max": 40, "awarded": 25}
            ])),
            Ok(())
        );
        for wrong in [
            serde_json::json!([{"name": "Style", "max": 40, "awarded": 25},
                               {"name": "Accuracy", "max": 60, "awarded": 50}]),
            serde_json::json!([{"name": "accuracy", "max": 60, "awarded": 50},
                               {"name": "Style", "max": 40, "awarded": 25}]),
            serde_json::json!([{"name": "Accuracy", "max": 50, "awarded": 50},
                               {"name": "Style", "max": 50, "awarded": 25}]),
            serde_json::json!([{"name": "Accuracy", "max": 100, "awarded": 75}]),
        ] {
            assert_eq!(
                run(wrong.clone()),
                Err(ResultError::CriteriaRubric),
                "{wrong}"
            );
        }
        assert_eq!(
            check_rubric(&check(&good()).unwrap(), &rubric),
            Err(ResultError::CriteriaRubric)
        );
    }

    #[test]
    fn errors_name_no_path_or_content() {
        for error in [
            ResultError::ReportOutside,
            ResultError::Missing,
            ResultError::NotJson,
        ] {
            let message = error.to_string();
            assert!(!message.contains('/'), "{message}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn reads_only_a_regular_file() {
        let root = dir();
        assert_eq!(read(root.path()), Err(ResultError::Missing));
        std::fs::write(root.path().join("elsewhere.json"), "{}").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("elsewhere.json"),
            root.path().join(RESULT_FILE),
        )
        .unwrap();
        assert_eq!(read(root.path()), Err(ResultError::NotRegularFile));
        std::fs::remove_file(root.path().join(RESULT_FILE)).unwrap();
        std::fs::write(
            root.path().join(RESULT_FILE),
            serde_json::to_vec(&good()).unwrap(),
        )
        .unwrap();
        assert_eq!(read(root.path()).unwrap().score, 75);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_or_link_named_result_is_refused_without_blocking() {
        let root = dir();
        let fifo = root.path().join("fifo");
        let made = std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let refused = |grade: std::path::PathBuf| {
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = send.send(read(&grade));
            });
            receive
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("the read returned without blocking")
        };
        // A FIFO at the name, and a link at the name pointing to a FIFO.
        let a = root.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::rename(&fifo, a.join(RESULT_FILE)).unwrap();
        assert_eq!(refused(a.clone()), Err(ResultError::NotRegularFile));
        let b = root.path().join("b");
        std::fs::create_dir(&b).unwrap();
        std::os::unix::fs::symlink(a.join(RESULT_FILE), b.join(RESULT_FILE)).unwrap();
        assert_eq!(refused(b), Err(ResultError::NotRegularFile));
        let c = root.path().join("c");
        std::fs::create_dir(&c).unwrap();
        std::fs::write(
            c.join(RESULT_FILE),
            vec![b' '; MAX_RESULT_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(refused(c), Err(ResultError::TooLarge));
    }
}
