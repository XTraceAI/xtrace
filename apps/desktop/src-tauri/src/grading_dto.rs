//! The wire contract of grading runs.
//!
//! Every decision is made in Rust: which skills exist (`xt_judge::skill`),
//! whether a run may start and how it ended (`crate::grading`), what the
//! score is (the skill's own `result.json`), and which reason a failure
//! carries ([`GradingFailure::of`]). The renderer shows these fields and
//! works nothing out again.
//!
//! A run's `project_path` is the project's full absolute folder path; it
//! stays on this machine and is never logged or sent in telemetry. A failure
//! is a reason code, never a path or a message.
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_judge::{
    GradeError, ResultError, RubricError, SkillError,
    installed::{PluginFile, PluginProblem, PluginReadFailure},
};

/// The grading skills this app can run: the built-in ones, then the ones
/// the user added from their installed skills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingSkills {
    Loaded {
        skills: Vec<GradingSkillInfo>,
    },
    /// Fixture mode: no skill is bundled with a disposable database, and it
    /// never falls back to one.
    NotConfigured,
    /// A bundled skill or the added skills could not be read.
    Failed,
}

/// One grading skill, from its rubric's header (`xtrace-rubric.md`). An
/// added skill that cannot be opened now keeps the name, description and
/// version stored when it was added, and says why ([`Self::problem`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingSkillInfo {
    pub id: String,
    pub name: String,
    /// The rubric's version and a short hash of the rubric file; runs are
    /// stored with the same text.
    pub version: String,
    pub description: String,
    pub origin: GradingSkillOrigin,
    /// Why an added skill cannot grade now; none when it can. A built-in
    /// skill always can (one that cannot makes the whole read fail).
    pub problem: Option<GradingSkillProblem>,
}

/// Where a grading skill comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GradingSkillOrigin {
    /// Bundled with the app; it cannot be removed.
    BuiltIn,
    /// Added from an installed skill; it can be removed from Scores.
    Added {
        /// The folder it is opened from now (an absolute path on this
        /// machine): a skills folder's skill's folder as added, a plugin's
        /// skill's folder in the plugin's current install. None for a
        /// plugin's skill whose plugin is not installed and turned on now.
        folder: Option<String>,
        /// The plugin's name, for a Claude Code plugin's skill.
        plugin: Option<String>,
    },
}

/// Why an added skill cannot grade now. Its card and stored grades stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum GradingSkillProblem {
    /// Its folder is gone or is no longer a real folder (a link).
    FolderMissing,
    /// It is a Claude Code plugin's skill, and that plugin is not installed
    /// for the user and turned on now (or its list cannot be read).
    PluginUnavailable,
    /// Its `SKILL.md` cannot be read.
    SkillUnreadable,
    /// Its folder has no `xtrace-rubric.md` any more.
    RubricMissing,
    /// Its rubric is refused.
    RubricInvalid { rubric: GradingRubricProblem },
    /// Its rubric now carries another id.
    RubricIdChanged,
}

impl GradingSkillProblem {
    /// The one mapping from an added skill that does not open to a reason.
    pub fn of(error: SkillError) -> Self {
        match error {
            SkillError::Rubric(RubricError::Missing) => Self::RubricMissing,
            SkillError::Rubric(rubric) => Self::RubricInvalid {
                rubric: rubric.into(),
            },
            SkillError::IdMismatch => Self::RubricIdChanged,
            SkillError::Missing => Self::FolderMissing,
            SkillError::Unknown | SkillError::PathNotUtf8 | SkillError::InstructionsUnreadable => {
                Self::SkillUnreadable
            }
        }
    }
}

/// Why a rubric was refused ([`xt_judge::RubricError`], one to one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingRubricProblem {
    Missing,
    NotRegularFile,
    TooLarge,
    Unreadable,
    NotUtf8,
    NoHeader,
    HeaderUnterminated,
    HeaderLine,
    UnknownKey,
    DuplicateKey,
    MissingKey,
    Protocol,
    Id,
    Field,
    Target,
    Heading,
    Points,
    CriterionName,
    DuplicateCriterion,
    NoGuidance,
    DuplicateFindings,
    FindingsPoints,
    NoCriteria,
    TooManyCriteria,
    Total,
}

impl From<RubricError> for GradingRubricProblem {
    fn from(error: RubricError) -> Self {
        match error {
            RubricError::Missing => Self::Missing,
            RubricError::NotRegularFile => Self::NotRegularFile,
            RubricError::TooLarge => Self::TooLarge,
            RubricError::Unreadable => Self::Unreadable,
            RubricError::NotUtf8 => Self::NotUtf8,
            RubricError::NoHeader => Self::NoHeader,
            RubricError::HeaderUnterminated => Self::HeaderUnterminated,
            RubricError::HeaderLine => Self::HeaderLine,
            RubricError::UnknownKey => Self::UnknownKey,
            RubricError::DuplicateKey => Self::DuplicateKey,
            RubricError::MissingKey => Self::MissingKey,
            RubricError::Protocol => Self::Protocol,
            RubricError::Id => Self::Id,
            RubricError::Field => Self::Field,
            RubricError::Target => Self::Target,
            RubricError::Heading => Self::Heading,
            RubricError::Points => Self::Points,
            RubricError::CriterionName => Self::CriterionName,
            RubricError::DuplicateCriterion => Self::DuplicateCriterion,
            RubricError::NoGuidance => Self::NoGuidance,
            RubricError::DuplicateFindings => Self::DuplicateFindings,
            RubricError::FindingsPoints => Self::FindingsPoints,
            RubricError::NoCriteria => Self::NoCriteria,
            RubricError::TooManyCriteria => Self::TooManyCriteria,
            RubricError::Total => Self::Total,
        }
    }
}

/// The skills the user installed, for the "Add a grading skill" dialog, decided
/// once by [`crate::grading_picker::picker`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingInstalledSkills {
    Loaded(GradingInstalledLoaded),
    /// Fixture mode: no home folder is read.
    NotConfigured,
    /// The added skills could not be read.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingInstalledLoaded {
    /// By display group (ready to add, yours, can't be scored, already on
    /// Scores); within each, ready rubrics first, then rubric problems, then
    /// those that need a rubric, by name.
    pub skills: Vec<GradingInstalledSkill>,
    /// How many skills are in each display group ([`GradingInstalledGroup`]).
    pub ready: u32,
    pub yours: u32,
    pub cant_score: u32,
    pub on_scores: u32,
    /// Skill folders found but not listed: links, an unreadable or too large
    /// `SKILL.md`, a name that is not UTF-8.
    pub skipped: u32,
    /// Not every skill is listed: more than the bounds allow, or the
    /// plugin list could not be used ([`Self::plugins_unread`]).
    pub incomplete: bool,
    /// Why plugins' skills are not all listed, when that is the plugin list.
    pub plugins_unread: Option<GradingPluginProblem>,
}

/// A Claude Code plugin list file that could not be used, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingPluginProblem {
    pub file: GradingPluginFile,
    pub failure: GradingPluginReadFailure,
}

/// Which file of Claude Code's plugin list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingPluginFile {
    /// `~/.claude/plugins`.
    PluginsFolder,
    /// `~/.claude/plugins/installed_plugins.json`.
    InstalledPlugins,
    /// `~/.claude/settings.json`.
    Settings,
}

/// Why it could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingPluginReadFailure {
    Link,
    Unreadable,
    Malformed,
    UnknownShape,
}

impl From<PluginProblem> for GradingPluginProblem {
    fn from(problem: PluginProblem) -> Self {
        Self {
            file: match problem.file {
                PluginFile::PluginsFolder => GradingPluginFile::PluginsFolder,
                PluginFile::InstalledPlugins => GradingPluginFile::InstalledPlugins,
                PluginFile::Settings => GradingPluginFile::Settings,
            },
            failure: match problem.failure {
                PluginReadFailure::Link => GradingPluginReadFailure::Link,
                PluginReadFailure::Unreadable => GradingPluginReadFailure::Unreadable,
                PluginReadFailure::Malformed => GradingPluginReadFailure::Malformed,
                PluginReadFailure::UnknownShape => GradingPluginReadFailure::UnknownShape,
            },
        }
    }
}

/// One installed skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingInstalledSkill {
    /// The skill's folder now (an absolute path on this machine).
    pub folder: String,
    /// What finds the skill again: its folder, or for a plugin's skill
    /// `plugin:<plugin key>/<skill folder name>`. Adding sends exactly this.
    pub location: String,
    /// The rubric's name when the skill is ready, otherwise `SKILL.md`'s.
    pub name: String,
    /// One line: the rubric's when ready, otherwise `SKILL.md`'s.
    pub description: String,
    pub source: GradingSkillSource,
    pub rubric: GradingInstalledRubric,
    /// Whether the rubric that applies is one XTrace keeps in its own folder
    /// (a Claude Code plugin's skill, or a folder XTrace could not write),
    /// rather than the skill folder's own `xtrace-rubric.md`.
    pub rubric_kept_by_xtrace: bool,
    pub scores: GradingInstalledScores,
    /// Where the Add a grading skill dialog shows it.
    pub group: GradingInstalledGroup,
    /// What "Make a rubric" can do for it: make one when it has none, make
    /// it again when its rubric has a problem; none when it is ready, or
    /// when a built-in skill or another skill on Scores already has its id.
    pub rubric_action: Option<GradingRubricAction>,
    /// Its latest rubric-making job: going now, or how it ended while that
    /// outcome still matches its rubric (a made rubric while it is ready; a
    /// failure, refusal or cancel while it is not). A refusal is kept across
    /// restarts while the skill is unchanged
    /// ([`xt_judge::refusals::in_force`]); the other outcomes are kept in
    /// memory only.
    pub rubric_job: Option<GradingRubricJob>,
}

/// Which group of the Add a grading skill dialog shows an installed skill, decided
/// once by [`crate::grading_picker::picker`].
/// Ordered as the dialog lists the groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingInstalledGroup {
    /// Ready to add: a valid rubric, and Scores does not have it yet (an
    /// addable skill, or one whose id another listed first has).
    Ready,
    /// Your skills: a rubric can be made for it (it has none, or its rubric
    /// has a problem), and Claude has not refused it as it is now.
    Yours,
    /// Can't be scored: a rubric could be made for it, but Claude refused it
    /// as it is now; Try again is still offered.
    CantScore,
    /// Already on Scores: built in, added, or another skill on Scores has its
    /// id.
    OnScores,
}

/// What making a rubric does for an installed skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingRubricAction {
    /// It has no rubric: write one.
    Make,
    /// Its rubric is refused: write a new one, keeping the old one as a
    /// backup beside it.
    Remake,
}

/// A rubric-making job for one installed skill, kept in memory only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRubricJob {
    Running {
        job_id: String,
        #[ts(type = "number")]
        started_at_ms: i64,
    },
    /// Saved; the skill is ready to add with these criteria.
    Made {
        job_id: String,
        criteria: Vec<GradingRubricCriterion>,
        /// Saved in XTrace's own folder rather than the skill's.
        kept_by_xtrace: bool,
    },
    Failed {
        job_id: String,
        failure: GradingRubricFailure,
    },
    /// Claude judged that the skill cannot be used to score projects; nothing
    /// was saved.
    Refused {
        job_id: String,
        /// Claude's short reason, as plain text of at most 300 characters.
        reason: String,
    },
    Cancelled {
        job_id: String,
    },
}

/// One criterion of a made rubric, for its short preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRubricCriterion {
    pub name: String,
    pub points: u8,
}

/// Why no rubric was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum GradingRubricFailure {
    /// The Claude Code run itself failed (the same reasons as a grading).
    Run { failure: GradingFailure },
    /// Claude wrote no rubric, on the retry either.
    RubricMissing,
    /// The rubric was refused on the retry too, for this reason.
    RubricInvalid { rubric: GradingRubricProblem },
    /// Claude's reason for not making one could not be read as short plain
    /// text, on the retry too.
    RefusalUnreadable,
    /// Claude wrote both a rubric and a refusal, on the retry too.
    BothWritten,
    /// Every free variant of its id is taken by other skills.
    IdUnavailable,
    /// The skill moved, went away, or got a rubric of its own while the job
    /// ran; nothing was saved.
    SkillChanged,
    /// The skill's folder has its own rubric and cannot be written, so the
    /// new one cannot replace it.
    FolderNotWritable,
    /// The rubric could not be saved.
    SaveFailed,
    /// The rubric maker bundled with the app could not be read.
    HelperUnavailable,
    /// XTrace hit an error while handling the job.
    InternalError,
}

/// What `grading_rubric_make` did. Only `started` started anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRubricMake {
    /// The job has begun; it is the installed skill's `rubric_job` now.
    Started { job_id: String },
    /// A grading or another rubric job holds the one slot (gradings may be
    /// waiting after it); one runs at a time.
    Busy,
    /// No installed skill is at that location now.
    NotFound,
    /// It has a valid rubric now.
    AlreadyReady,
    /// No rubric is made for it: a built-in skill or another skill on Scores
    /// already has its id.
    NotOffered,
    /// The rubric maker bundled with the app could not be read.
    HelperUnavailable,
    /// No Claude Code executable was found.
    ClaudeUnavailable,
    /// Fixture mode never runs anything.
    NotConfigured,
    /// The app is closing.
    Stopped,
    /// The installed skills could not be read, or no job ID could be made.
    Failed,
}

/// The rubric-making job going now, named for a screen that explains why
/// gradings wait.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRubricNow {
    pub job_id: String,
    /// The installed skill's location.
    pub location: String,
    /// Its name as the Add a grading skill dialog listed it.
    pub name: String,
    #[ts(type = "number")]
    pub started_at_ms: i64,
}

/// A rubric-making job's progress, sent as `grading://rubric-progress`: when
/// it starts, about every five seconds while it runs, and once when it is
/// over.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRubricProgress {
    pub job_id: String,
    pub location: String,
    pub state: GradingProgressState,
    /// Whole seconds since the job started.
    pub elapsed_seconds: u32,
}

/// Where an installed skill was found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GradingSkillSource {
    /// `~/.claude/skills`.
    Claude,
    /// `~/.agents/skills`.
    Agents,
    /// `~/.codex/skills`.
    Codex,
    /// A Claude Code plugin.
    Plugin { plugin: String },
}

/// An installed skill's rubric: its group in the dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingInstalledRubric {
    /// A valid `xtrace-rubric.md` with this id.
    Ready { id: String },
    /// An `xtrace-rubric.md` that is refused.
    Problem { reason: GradingRubricProblem },
    /// No `xtrace-rubric.md`.
    Missing,
}

/// Whether an installed skill can be added to Scores, or why not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingInstalledScores {
    /// Ready, and nothing on Scores or listed before it has its id.
    Addable,
    /// This folder is the one added to Scores.
    OnScores,
    /// Its rubric id (or, without a valid rubric, its name) is a built-in
    /// skill's id.
    BuiltIn,
    /// Its rubric id (or name) is the id of a skill added from elsewhere
    /// (that skill's location).
    IdOnScores { location: String },
    /// Another ready skill listed before it has the same rubric id; only one
    /// can be added.
    IdClash { name: String, folder: String },
    /// Not ready: it needs a rubric, or its rubric has a problem.
    NotReady,
}

/// What `grading_skill_add` did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingSkillAdd {
    /// Stored; its card is on Scores.
    Added { skill_id: String },
    /// No installed skill is at that location now.
    NotFound,
    /// The skill has no valid rubric now.
    NotReady,
    /// A skill with its id is already on Scores.
    AlreadyOnScores,
    /// Another installed skill listed first has the same id.
    IdClash,
    /// Fixture mode never adds anything.
    NotConfigured,
    /// Storage could not save it.
    Failed,
}

/// What `grading_skill_remove` did. Stored grades and files stay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingSkillRemove {
    Removed,
    /// Built-in skills cannot be removed.
    BuiltIn,
    /// No added skill has that id.
    NotFound,
    /// That skill is grading a project now or waiting to grade one; it
    /// stays until that run ends or is cancelled.
    Running,
    NotConfigured,
    Failed,
}

/// What `grading_start` did. Only `started` started anything now; `waiting`
/// starts by itself later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingStart {
    /// The run's `running` row is saved and the run has begun.
    Started { run_id: String },
    /// A grading or a rubric-making job holds the one slot: the request
    /// waits in line, in memory only, and starts by itself with this run ID
    /// once every request before it is done. Nothing is saved until then;
    /// Cancel with this ID takes it out of the line.
    Waiting { run_id: String },
    /// This project is being graded with this skill now, or is already
    /// waiting to be.
    AlreadyGoing,
    /// The folder is not a project in a fresh project list, or its path is
    /// not exactly that folder's resolved path (a link, `..`, a trailing
    /// slash).
    NotAProject,
    /// The skill is neither a built-in nor an added skill.
    UnknownSkill,
    /// The skill could not be opened: the bundled one, or an added one whose
    /// folder or rubric is gone or no longer valid (its info says which).
    SkillUnavailable,
    /// No Claude Code executable was found.
    ClaudeUnavailable,
    /// Fixture mode never runs anything.
    NotConfigured,
    /// The app is closing.
    Stopped,
    /// Storage could not save the run, or no run ID could be made.
    Failed,
}

/// The recent runs, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRuns {
    Loaded { runs: Vec<GradingRun> },
    NotConfigured,
    Failed,
}

/// Where a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// The app closed while the run was going.
    Interrupted,
}

impl From<xt_store::grades::GradeStatus> for GradingStatus {
    fn from(status: xt_store::grades::GradeStatus) -> Self {
        use xt_store::grades::GradeStatus;
        match status {
            GradeStatus::Running => Self::Running,
            GradeStatus::Succeeded => Self::Succeeded,
            GradeStatus::Failed => Self::Failed,
            GradeStatus::Cancelled => Self::Cancelled,
            GradeStatus::Interrupted => Self::Interrupted,
        }
    }
}

/// One stored run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRun {
    pub run_id: String,
    pub project_path: String,
    pub skill_id: String,
    pub skill_version: String,
    #[ts(type = "number")]
    pub started_at_ms: i64,
    #[ts(type = "number | null")]
    pub finished_at_ms: Option<i64>,
    pub status: GradingStatus,
    /// The skill's own score, 0-100; only for a succeeded run.
    pub score: Option<u8>,
    /// False when the skill skipped a stage; only for a succeeded run.
    pub complete: Option<bool>,
    /// Only for a succeeded run, and none after stored content was deleted.
    pub summary: Option<String>,
    pub insights: Option<Vec<GradingInsight>>,
    /// The report's file name inside the run's kept grade folder.
    pub report_file: Option<String>,
    /// Only for a failed or interrupted run.
    pub failure: Option<GradingFailure>,
}

/// How a finding is weighed, in the grader protocol's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingSeverity {
    Critical,
    Warning,
    Info,
    Pass,
}

/// One finding. Stored exactly as the validated result gave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GradingInsight {
    pub severity: GradingSeverity,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fix: Option<String>,
}

/// Why a run failed or was interrupted. Stored by its code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingFailure {
    /// The app closed during the run.
    AppClosed,
    /// The folder stopped being a listed project before the run launched.
    ProjectChanged,
    /// The project folder could not be used.
    ProjectUnavailable,
    /// The run changed the project's `.claude-md-doctor` folder.
    ProjectWritten,
    /// The project's `.claude-md-doctor` folder is too large to check.
    ProjectWorkFolderUncheckable,
    /// The staged copy of the skill could not be read.
    SkillUnavailable,
    /// The skill's folder has more files, bytes or folder levels than a run
    /// copies.
    SkillTooLarge,
    /// The skill's folder has a link to a folder, out of it, or to nothing.
    SkillLinkRefused,
    /// The skill's rubric is missing or invalid.
    RubricInvalid,
    /// The skill's rubric changed between the start and the copy, so the
    /// version stored with the run would not be the one graded with.
    SkillChanged,
    /// Claude Code's sandbox is not available, so the project could not be
    /// kept read-only.
    SandboxUnavailable,
    /// A folder the run needs has whitespace in its path.
    PathHasWhitespace,
    /// The run's private folders could not be prepared.
    StagingFailed,
    /// The run could not be set up.
    SetupFailed,
    /// Claude Code could not be started.
    SpawnFailed,
    /// Claude Code is older than grading needs, or its version could not be
    /// read.
    ClaudeTooOld,
    /// Claude Code exited with a failure.
    ClaudeFailed,
    /// The run took longer than its deadline.
    Timeout,
    /// The run could not be observed.
    Unobservable,
    /// The run wrote no result.
    ResultMissing,
    /// The run's result broke the grader protocol.
    ResultInvalid,
    /// The validated result could not be kept.
    ArchiveFailed,
    /// The app failed while it handled the run.
    InternalError,
    /// No Claude Code executable was found when a waiting grading's turn
    /// came.
    ClaudeUnavailable,
    /// A code this build does not know.
    Other,
}

impl GradingFailure {
    /// Every reason, for tests.
    pub const ALL: [Self; 25] = [
        Self::AppClosed,
        Self::ProjectChanged,
        Self::ProjectUnavailable,
        Self::ProjectWritten,
        Self::ProjectWorkFolderUncheckable,
        Self::SkillUnavailable,
        Self::SkillTooLarge,
        Self::SkillLinkRefused,
        Self::RubricInvalid,
        Self::SkillChanged,
        Self::SandboxUnavailable,
        Self::PathHasWhitespace,
        Self::StagingFailed,
        Self::SetupFailed,
        Self::SpawnFailed,
        Self::ClaudeTooOld,
        Self::ClaudeFailed,
        Self::Timeout,
        Self::Unobservable,
        Self::ResultMissing,
        Self::ResultInvalid,
        Self::ArchiveFailed,
        Self::InternalError,
        Self::ClaudeUnavailable,
        Self::Other,
    ];

    /// The stored code: the wire name.
    pub fn code(self) -> &'static str {
        match self {
            Self::AppClosed => "app_closed",
            Self::ProjectChanged => "project_changed",
            Self::ProjectUnavailable => "project_unavailable",
            Self::ProjectWritten => "project_written",
            Self::ProjectWorkFolderUncheckable => "project_work_folder_uncheckable",
            Self::SkillUnavailable => "skill_unavailable",
            Self::SkillTooLarge => "skill_too_large",
            Self::SkillLinkRefused => "skill_link_refused",
            Self::RubricInvalid => "rubric_invalid",
            Self::SkillChanged => "skill_changed",
            Self::SandboxUnavailable => "sandbox_unavailable",
            Self::PathHasWhitespace => "path_has_whitespace",
            Self::StagingFailed => "staging_failed",
            Self::SetupFailed => "setup_failed",
            Self::SpawnFailed => "spawn_failed",
            Self::ClaudeTooOld => "claude_too_old",
            Self::ClaudeFailed => "claude_failed",
            Self::Timeout => "timeout",
            Self::Unobservable => "unobservable",
            Self::ResultMissing => "result_missing",
            Self::ResultInvalid => "result_invalid",
            Self::ArchiveFailed => "archive_failed",
            Self::InternalError => "internal_error",
            Self::ClaudeUnavailable => "claude_unavailable",
            Self::Other => "other",
        }
    }

    /// A stored code; one this build does not know reads as [`Self::Other`].
    pub fn from_code(code: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|reason| reason.code() == code)
            .unwrap_or(Self::Other)
    }

    /// The one mapping from a staged skill that cannot be opened to a
    /// reason: its rubric, or the skill itself.
    pub fn of_skill(error: SkillError) -> Self {
        if error.is_rubric() {
            Self::RubricInvalid
        } else {
            Self::SkillUnavailable
        }
    }

    /// The one mapping from a runner error to a reason. `None` for a
    /// cancellation, which is a status of its own.
    pub fn of(error: &GradeError) -> Option<Self> {
        Some(match error {
            GradeError::Cancelled => return None,
            GradeError::ProjectInvalid => Self::ProjectUnavailable,
            GradeError::ProjectWritten => Self::ProjectWritten,
            GradeError::ProjectWorkFolderUncheckable => Self::ProjectWorkFolderUncheckable,
            GradeError::SkillPathHasWhitespace | GradeError::GradeDirPathHasWhitespace => {
                Self::PathHasWhitespace
            }
            GradeError::StagingFailed => Self::StagingFailed,
            GradeError::SkillTooLarge => Self::SkillTooLarge,
            GradeError::SkillLinkRefused => Self::SkillLinkRefused,
            GradeError::ArchiveFailed => Self::ArchiveFailed,
            GradeError::UnsupportedPlatform
            | GradeError::ClaudeNotAbsolute
            | GradeError::GradeDirInvalid
            | GradeError::GradeDirNotEmpty
            | GradeError::GradeDirOverlapsProject
            | GradeError::PathNotUtf8
            | GradeError::Prepare => Self::SetupFailed,
            GradeError::Spawn => Self::SpawnFailed,
            GradeError::SandboxUnavailable => Self::SandboxUnavailable,
            GradeError::ClaudeTooOld => Self::ClaudeTooOld,
            GradeError::Exited { .. } => Self::ClaudeFailed,
            GradeError::Timeout => Self::Timeout,
            GradeError::Unobservable => Self::Unobservable,
            GradeError::Result(ResultError::Missing) => Self::ResultMissing,
            GradeError::Result(_) => Self::ResultInvalid,
        })
    }
}

/// The Scores page's answer, decided once by [`crate::grading_overview`]:
/// each grading skill's shown scores over the listed projects, their
/// average, its top insights, and the run going now. Read fresh: the project
/// list and the stored runs are both read for each answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingOverview {
    Loaded(Box<GradingOverviewLoaded>),
    /// Fixture mode: no skill is bundled and no project list is read.
    NotConfigured,
    /// A bundled skill, the added skills, the project list or the stored
    /// runs could not be read.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingOverviewLoaded {
    /// Every project in the fresh project list, in its order, with the apps
    /// that named it.
    pub projects: Vec<crate::projects_dto::ProjectEntry>,
    /// One per grading skill: the built-in ones, then the added ones in the
    /// order they were added.
    pub skills: Vec<GradingSkillOverview>,
    /// The run whose stored status is `running`, if any; at most one runs.
    /// Only the stored status says a run is going, never a progress event.
    pub running: Option<GradingRunningNow>,
    /// The rubric-making job going now, if any. It holds the same one slot
    /// as a grading, so gradings started meanwhile wait in line until it is
    /// done. Boxed: the answer stays small while no job runs.
    pub making_rubric: Option<Box<GradingRubricNow>>,
    /// The waiting gradings no row shows (their project is no longer listed,
    /// or their skill is not on Scores), oldest first, so each can still be
    /// cancelled.
    pub waiting_elsewhere: Vec<GradingWaitingElsewhere>,
}

/// One skill over the listed projects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingSkillOverview {
    pub skill: GradingSkillInfo,
    /// The mean of the listed projects' shown scores, rounded half up; none
    /// while no listed project has one.
    pub average: Option<u8>,
    /// Listed projects with a shown score.
    pub graded: u32,
    /// Listed projects.
    pub total: u32,
    /// At most three findings of the shown runs, grouped by exact title:
    /// critical before warning before info, then by how many projects have
    /// it, then by title. A `pass` finding is listed only when the shown runs
    /// have no other finding.
    pub top_insights: Vec<GradingTopInsight>,
    /// Whether the run going now is this skill's.
    pub running: bool,
    /// How many gradings with this skill are waiting in line, on any
    /// project.
    pub waiting: u32,
    /// One per listed project, in the project list's order.
    pub rows: Vec<GradingProjectRow>,
}

/// One finding title across the shown runs of a skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingTopInsight {
    /// The most severe weight any project gave this title.
    pub severity: GradingSeverity,
    pub title: String,
    /// How many listed projects' shown runs have it.
    pub projects: u32,
}

/// One listed project under one skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingProjectRow {
    pub name: String,
    /// The project's full absolute folder path.
    pub path: String,
    /// The newest succeeded run's score; none when the project was never
    /// graded with this skill.
    pub shown: Option<GradingShownScore>,
    /// The newest run, when it is newer than the shown one (so it did not
    /// succeed): going now, or how it ended.
    pub attempt: Option<GradingAttempt>,
    /// Whether this project's run is the one going now.
    pub running: bool,
    /// This project's grading with this skill waiting in line, if any.
    pub waiting: Option<GradingRowWaiting>,
}

/// A grading waiting in line for the one slot: it starts by itself when
/// every entry before it is done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRowWaiting {
    /// The ID it starts with, and the one Cancel takes it out of the line by.
    pub run_id: String,
    /// How many waiting gradings are before it in the whole line.
    pub ahead: u32,
}

/// A waiting grading no row shows, named for a screen that lists it with
/// its own Cancel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingWaitingElsewhere {
    pub run_id: String,
    pub skill_id: String,
    /// The skill's name when it is on Scores.
    pub skill_name: Option<String>,
    pub project_path: String,
    /// The project's name when it is still listed.
    pub project_name: Option<String>,
    /// How many waiting gradings are before it in the whole line.
    pub ahead: u32,
}

/// The score a project shows for a skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingShownScore {
    pub run_id: String,
    pub score: u8,
    /// When that run finished.
    #[ts(type = "number")]
    pub graded_at_ms: i64,
    /// The skill skipped a stage (`complete: false`); the score still counts.
    pub partial: bool,
}

/// A project's newest run when it is newer than its shown score.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingAttempt {
    Running {
        run_id: String,
        #[ts(type = "number")]
        started_at_ms: i64,
    },
    Failed {
        run_id: String,
        #[ts(type = "number")]
        finished_at_ms: i64,
        reason: GradingFailure,
    },
    Cancelled {
        run_id: String,
        #[ts(type = "number")]
        finished_at_ms: i64,
    },
    Interrupted {
        run_id: String,
        #[ts(type = "number")]
        finished_at_ms: i64,
        reason: GradingFailure,
    },
}

/// The run going now, named for a screen that shows it where no row does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRunningNow {
    pub run_id: String,
    pub skill_id: String,
    /// The skill's name when it is on Scores.
    pub skill_name: Option<String>,
    pub project_path: String,
    /// The project's name when it is still listed.
    pub project_name: Option<String>,
    #[ts(type = "number")]
    pub started_at_ms: i64,
}

/// A running run's progress, sent as `grading://progress`: when it starts,
/// about every five seconds while it runs, and once when it is over.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingProgress {
    pub run_id: String,
    pub state: GradingProgressState,
    /// Whole seconds since the run started.
    pub elapsed_seconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingProgressState {
    Running,
    Finished,
}

/// The details of one succeeded run (the run behind a shown score), decided
/// once by [`crate::grading_details`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRunDetails {
    Loaded(GradingRunDetailsLoaded),
    /// No succeeded run has that ID.
    NotFound,
    /// Fixture mode: no run is stored and no grade folder is read.
    NotConfigured,
    /// The stored run could not be read.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRunDetailsLoaded {
    pub run_id: String,
    pub score: u8,
    /// When the run finished.
    #[ts(type = "number")]
    pub graded_at_ms: i64,
    /// False when the skill skipped a stage; the score still counts.
    pub complete: bool,
    /// The run's summary, findings, criteria and report, or that "Delete
    /// stored content" removed them (the score stays).
    pub content: GradingRunContent,
}

/// What a run keeps besides its score.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRunContent {
    /// "Delete stored content" removed the summary, findings, criteria and
    /// report.
    Deleted,
    Kept(GradingRunKept),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingRunKept {
    pub summary: String,
    /// Every finding the run stored, most severe first (critical, warning,
    /// info, then pass), each weight in the run's own order.
    pub findings: GradingRunFindings,
    /// The points each rubric criterion was awarded, or why they are not
    /// shown.
    pub criteria: GradingRunCriteria,
    /// Whether "Open full report" can open the run's report, or why not.
    pub report: GradingRunReport,
}

/// A run's stored findings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRunFindings {
    Listed {
        findings: Vec<GradingInsight>,
    },
    /// The stored findings could not be read.
    Unreadable,
}

/// A run's rubric criteria with their points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRunCriteria {
    /// In the rubric's order; `max` adds up to 100 and `awarded` to the score.
    Reported {
        criteria: Vec<GradingCriterionScore>,
    },
    Unavailable {
        reason: GradingRunGap,
    },
}

/// The points one rubric criterion was awarded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingCriterionScore {
    pub name: String,
    pub max: u8,
    pub awarded: u8,
}

/// A run's full HTML report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingRunReport {
    /// Kept in the run's grade folder; it can be opened.
    Kept,
    Unavailable {
        reason: GradingRunGap,
    },
}

/// Why a kept run's criteria or report are not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingRunGap {
    /// Only the newest succeeded runs keep their grade folder
    /// ([`crate::grading::KEEP_FOLDERS`]); this run's was removed.
    FolderNotKept,
    /// The run did not write it: a result from before rubrics has no
    /// criteria, and a skill may write no report.
    NotReported,
    /// The run's grade folder could not be read, or what it holds does not
    /// match the stored run.
    Unreadable,
}

/// What `grading_report_open` did. Only `opened` opened anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingReportOpen {
    /// Handed to macOS to open in the default app for HTML files.
    Opened,
    /// No succeeded run has that ID.
    NotFound,
    /// "Delete stored content" removed the run's report.
    ContentDeleted,
    /// The run's report cannot be opened, for this reason.
    Unavailable { reason: GradingRunGap },
    /// Fixture mode never opens anything.
    NotConfigured,
    /// macOS could not open it.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_code_is_its_wire_name_and_reads_back() {
        for reason in GradingFailure::ALL {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::json!(reason.code())
            );
            assert_eq!(GradingFailure::from_code(reason.code()), reason);
            // The store accepts only lowercase letters and `_`.
            assert!(
                reason
                    .code()
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            );
        }
        assert_eq!(
            GradingFailure::from_code("unheard_of"),
            GradingFailure::Other
        );
    }

    #[test]
    fn unsupported_versions_and_invalid_rubrics_have_their_own_reasons() {
        assert_eq!(
            GradingFailure::of(&GradeError::ClaudeTooOld),
            Some(GradingFailure::ClaudeTooOld)
        );
        for rubric in [
            SkillError::Rubric(xt_judge::RubricError::Missing),
            SkillError::Rubric(xt_judge::RubricError::Total),
            SkillError::IdMismatch,
        ] {
            assert_eq!(
                GradingFailure::of_skill(rubric),
                GradingFailure::RubricInvalid
            );
        }
        for skill in [SkillError::Missing, SkillError::InstructionsUnreadable] {
            assert_eq!(
                GradingFailure::of_skill(skill),
                GradingFailure::SkillUnavailable
            );
        }
        // A result whose criteria are not the rubric's is an invalid result.
        assert_eq!(
            GradingFailure::of(&GradeError::Result(ResultError::CriteriaRubric)),
            Some(GradingFailure::ResultInvalid)
        );
    }

    #[test]
    fn a_cancellation_has_no_reason_and_every_result_error_has_one() {
        assert_eq!(GradingFailure::of(&GradeError::Cancelled), None);
        assert_eq!(
            GradingFailure::of(&GradeError::Result(ResultError::Missing)),
            Some(GradingFailure::ResultMissing)
        );
        assert_eq!(
            GradingFailure::of(&GradeError::Result(ResultError::Score)),
            Some(GradingFailure::ResultInvalid)
        );
        assert_eq!(
            GradingFailure::of(&GradeError::Exited { code: Some(2) }),
            Some(GradingFailure::ClaudeFailed)
        );
    }

    #[test]
    fn stored_insights_read_back_exactly_as_the_protocol_writes_them() {
        let written = serde_json::to_string(&vec![xt_judge::Insight {
            severity: xt_judge::Severity::Warning,
            title: "Long".into(),
            detail: None,
            fix: Some("Trim it.".into()),
        }])
        .unwrap();
        let read: Vec<GradingInsight> = serde_json::from_str(&written).unwrap();
        assert_eq!(
            read,
            [GradingInsight {
                severity: GradingSeverity::Warning,
                title: "Long".into(),
                detail: None,
                fix: Some("Trim it.".into()),
            }]
        );
    }
}
