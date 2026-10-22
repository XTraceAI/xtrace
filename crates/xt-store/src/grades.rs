//! Grading runs (`grades`, migration 25): one row per run of a built-in
//! grading skill on one project.
//!
//! A run is inserted `running` before it starts ([`Store::grade_insert_running`])
//! and finished exactly once ([`Store::grade_finish`]): only a running row
//! moves, so a second finish of the same run changes nothing. At most one
//! row is running at a time. After the app stops unexpectedly, the next
//! start marks whatever is still running `interrupted`
//! ([`Store::grades_interrupt_running`]).
//!
//! Only a succeeded run has a score and a completeness flag. Its summary,
//! insights and report file name are content, registered with content
//! retention ([`CONTENT_COLUMNS`]): kept whatever the retention mode (the
//! user asked for this grade), cleared by an explicit purge. A failed or
//! interrupted run carries a reason code (lowercase letters and `_`), never
//! a path or a message. The table's checks hold all of this, so no writer
//! can store an inconsistent row.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};

/// The `grades` columns registered with content retention.
pub const CONTENT_COLUMNS: [&str; 3] = ["summary", "insights_json", "report_file"];
/// Most rows one list returns.
pub const MAX_LIST: u32 = 500;
/// Most (project, skill) pairs one [`Store::grade_pairs`] reads.
pub const MAX_PAIRS: usize = 20_000;

/// The runs of one (project, skill) pair that decide what a screen shows
/// for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GradePairRuns {
    /// The pair's newest run of any status (by start, then ID).
    pub newest: Option<GradeRow>,
    /// The pair's newest succeeded run (by start, then ID).
    pub newest_succeeded: Option<GradeRow>,
}

/// [`Store::grade_pairs`]'s answer, read in one snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GradePairs {
    /// One per requested pair, in the requested order.
    pub pairs: Vec<GradePairRuns>,
    /// The run still running, whichever pair it belongs to.
    pub running: Option<GradeRow>,
}

/// Where a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradeStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// The app closed while the run was going.
    Interrupted,
}

impl GradeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => return Err(Error::InvalidInput("stored grade status is invalid")),
        })
    }
}

/// A run about to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewGradeRun<'a> {
    /// A lowercase UUID.
    pub run_id: &'a str,
    /// The project's absolute folder path.
    pub project_path: &'a str,
    pub skill_id: &'a str,
    pub skill_version: &'a str,
    pub started_at_ms: i64,
}

/// How a run ended. Every variant carries its finish time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GradeFinish<'a> {
    Succeeded {
        finished_at_ms: i64,
        score: u8,
        complete: bool,
        summary: &'a str,
        /// A JSON array.
        insights_json: &'a str,
        /// Relative to the run's grade folder.
        report_file: Option<&'a str>,
    },
    Failed {
        finished_at_ms: i64,
        reason: &'a str,
    },
    Cancelled {
        finished_at_ms: i64,
    },
    Interrupted {
        finished_at_ms: i64,
        reason: &'a str,
    },
}

/// One stored run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GradeRow {
    pub run_id: String,
    pub project_path: String,
    pub skill_id: String,
    pub skill_version: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub status: GradeStatus,
    pub score: Option<u8>,
    pub complete: Option<bool>,
    /// `None` for a run that did not succeed, and after a purge.
    pub summary: Option<String>,
    pub insights_json: Option<String>,
    pub report_file: Option<String>,
    pub failure_reason: Option<String>,
}

const COLUMNS: &str = "run_id,project_path,skill_id,skill_version,started_at,finished_at,\
     status,score,complete,summary,insights_json,report_file,failure_reason";

impl Store {
    /// Insert a `running` row. Refused when another run is still running or
    /// the ID is already used.
    pub fn grade_insert_running(&mut self, run: &NewGradeRun<'_>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO grades(run_id,project_path,skill_id,skill_version,started_at,status)
             VALUES(?1,?2,?3,?4,?5,'running')",
            params![
                run.run_id,
                run.project_path,
                run.skill_id,
                run.skill_version,
                run.started_at_ms
            ],
        )?;
        Ok(())
    }

    /// Finish a running row. Returns whether it moved: false when the run is
    /// unknown or already finished, which then stays exactly as it was. A
    /// finish time before the run's start (the clock moved back meanwhile)
    /// is stored as the start, so a finish never fails on the clock.
    pub fn grade_finish(&mut self, run_id: &str, finish: &GradeFinish<'_>) -> Result<bool> {
        let changed = match *finish {
            GradeFinish::Succeeded {
                finished_at_ms,
                score,
                complete,
                summary,
                insights_json,
                report_file,
            } => self.connection.execute(
                "UPDATE grades SET status='succeeded',finished_at=max(started_at,?2),score=?3,complete=?4,
                     summary=?5,insights_json=?6,report_file=?7
                 WHERE run_id=?1 AND status='running'",
                params![
                    run_id,
                    finished_at_ms,
                    score,
                    complete,
                    summary,
                    insights_json,
                    report_file
                ],
            )?,
            GradeFinish::Failed {
                finished_at_ms,
                reason,
            } => self.connection.execute(
                "UPDATE grades SET status='failed',finished_at=max(started_at,?2),failure_reason=?3
                 WHERE run_id=?1 AND status='running'",
                params![run_id, finished_at_ms, reason],
            )?,
            GradeFinish::Cancelled { finished_at_ms } => self.connection.execute(
                "UPDATE grades SET status='cancelled',finished_at=max(started_at,?2)
                 WHERE run_id=?1 AND status='running'",
                params![run_id, finished_at_ms],
            )?,
            GradeFinish::Interrupted {
                finished_at_ms,
                reason,
            } => self.connection.execute(
                "UPDATE grades SET status='interrupted',finished_at=max(started_at,?2),failure_reason=?3
                 WHERE run_id=?1 AND status='running'",
                params![run_id, finished_at_ms, reason],
            )?,
        };
        Ok(changed == 1)
    }

    /// Mark every running row `interrupted` with `reason`, finished at
    /// `finished_at_ms` (or at its start, if that is later). Returns their
    /// IDs.
    pub fn grades_interrupt_running(
        &mut self,
        finished_at_ms: i64,
        reason: &str,
    ) -> Result<Vec<String>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = transaction
            .prepare("SELECT run_id FROM grades WHERE status='running' ORDER BY run_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        transaction.execute(
            "UPDATE grades SET status='interrupted',finished_at=max(started_at,?1),failure_reason=?2
             WHERE status='running'",
            params![finished_at_ms, reason],
        )?;
        transaction.commit()?;
        Ok(ids)
    }

    /// One run, if stored.
    pub fn grade(&self, run_id: &str) -> Result<Option<GradeRow>> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM grades WHERE run_id=?1"),
                [run_id],
                raw,
            )
            .optional()?
            .map(GradeRow::try_from)
            .transpose()
    }

    /// The newest runs first (by start, then ID), at most `limit`, itself at
    /// most [`MAX_LIST`].
    pub fn grades(&self, limit: u32) -> Result<Vec<GradeRow>> {
        self.grade_query(
            &format!("SELECT {COLUMNS} FROM grades ORDER BY started_at DESC, run_id DESC LIMIT ?1"),
            limit,
        )
    }

    /// The newest run of each (project, skill) pair, newest first, at most
    /// `limit` pairs, itself at most [`MAX_LIST`]. Any status counts.
    pub fn grades_latest_per_pair(&self, limit: u32) -> Result<Vec<GradeRow>> {
        self.grade_query(
            &format!(
                "SELECT {COLUMNS} FROM (
                     SELECT *, row_number() OVER (
                         PARTITION BY project_path, skill_id
                         ORDER BY started_at DESC, run_id DESC) AS rank
                     FROM grades)
                 WHERE rank=1 ORDER BY started_at DESC, run_id DESC LIMIT ?1"
            ),
            limit,
        )
    }

    /// For each `(project path, skill ID)` pair, its newest run and its
    /// newest succeeded run, and the run still running, all read in one
    /// snapshot so they agree. Refused for more than [`MAX_PAIRS`] pairs.
    pub fn grade_pairs(&self, pairs: &[(&str, &str)]) -> Result<GradePairs> {
        if pairs.len() > MAX_PAIRS {
            return Err(Error::InvalidInput("too many grade pairs"));
        }
        self.in_read_snapshot(|store| {
            let order = "ORDER BY started_at DESC, run_id DESC LIMIT 1";
            let mut newest = store.connection.prepare(&format!(
                "SELECT {COLUMNS} FROM grades WHERE project_path=?1 AND skill_id=?2 {order}"
            ))?;
            let mut succeeded = store.connection.prepare(&format!(
                "SELECT {COLUMNS} FROM grades
                 WHERE project_path=?1 AND skill_id=?2 AND status='succeeded' {order}"
            ))?;
            let one = |statement: &mut rusqlite::Statement<'_>, project: &str, skill: &str| {
                statement
                    .query_row(params![project, skill], raw)
                    .optional()?
                    .map(GradeRow::try_from)
                    .transpose()
            };
            let pairs = pairs
                .iter()
                .map(|(project, skill)| {
                    Ok(GradePairRuns {
                        newest: one(&mut newest, project, skill)?,
                        newest_succeeded: one(&mut succeeded, project, skill)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let running = store
                .connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM grades WHERE status='running' {order}"),
                    [],
                    raw,
                )
                .optional()?
                .map(GradeRow::try_from)
                .transpose()?;
            Ok(GradePairs { pairs, running })
        })
    }

    /// The IDs of the newest `limit` succeeded runs (by start, then ID).
    pub fn grade_newest_succeeded_ids(&self, limit: u32) -> Result<Vec<String>> {
        Ok(self
            .connection
            .prepare(
                "SELECT run_id FROM grades WHERE status='succeeded'
                 ORDER BY started_at DESC, run_id DESC LIMIT ?1",
            )?
            .query_map([limit], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn grade_query(&self, sql: &str, limit: u32) -> Result<Vec<GradeRow>> {
        let rows = self
            .connection
            .prepare(sql)?
            .query_map([limit.min(MAX_LIST)], raw)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(GradeRow::try_from).collect()
    }
}

type Raw = (
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    String,
    Option<i64>,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

impl TryFrom<Raw> for GradeRow {
    type Error = Error;
    fn try_from(raw: Raw) -> Result<Self> {
        let (
            run_id,
            project_path,
            skill_id,
            skill_version,
            started_at_ms,
            finished_at_ms,
            status,
            score,
            complete,
            summary,
            insights_json,
            report_file,
            failure_reason,
        ) = raw;
        Ok(Self {
            run_id,
            project_path,
            skill_id,
            skill_version,
            started_at_ms,
            finished_at_ms,
            status: GradeStatus::parse(&status)?,
            score: score
                .map(|score| {
                    u8::try_from(score)
                        .map_err(|_| Error::InvalidInput("stored grade score is invalid"))
                })
                .transpose()?,
            complete,
            summary,
            insights_json,
            report_file,
            failure_reason,
        })
    }
}
