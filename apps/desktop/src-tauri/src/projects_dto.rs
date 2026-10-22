//! The wire contract of the project list.
//!
//! Every decision — which folders are projects, their names, merging,
//! skipping, worktree folding, hiding the home folder and folders that only
//! group other projects, and order — is made by
//! `xt_ingest::native::saved_projects`. This only carries its answer: the
//! renderer shows these fields and works nothing out again.
//!
//! A project's `path` is its full absolute folder path, as the file system
//! resolves it; a view may shorten the home folder to `~` for display. Paths
//! stay on this machine: they are never logged and never sent in telemetry.
use serde::Serialize;
use ts_rs::TS;
use xt_ingest::native::saved_projects::{
    ProjectSource, SavedProject, SavedProjects, SkipCounts, SourceReport, SourceStatus,
};

/// The outcome of one `projects_list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectList {
    /// The saved files were read. Each source's own status says whether it
    /// gave anything; a broken source never turns the list into a failure.
    Loaded(ProjectListLoaded),
    /// This process has no home folder to read: fixture mode, which never
    /// falls back to the user's own home.
    NotConfigured,
    /// The read failed in a way with no safe detail to report.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct ProjectListLoaded {
    /// Ordered by name ignoring case, then path.
    pub projects: Vec<ProjectEntry>,
    /// One per source, always all five, in a fixed order.
    pub sources: Vec<ProjectSourceReport>,
    pub skipped: ProjectSkipCounts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct ProjectEntry {
    /// The Claude app's preset name, else the Codex app's project name, else
    /// the folder's own name.
    pub name: String,
    /// The full absolute folder path.
    pub path: String,
    /// Every app whose saved files named this folder or one of its folded
    /// worktrees.
    pub sources: Vec<ProjectSourceId>,
}

/// The app whose saved file names projects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSourceId {
    ClaudeCode,
    ClaudeApp,
    CodexApp,
    CodexCli,
    Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct ProjectSourceReport {
    pub source: ProjectSourceId,
    pub status: ProjectSourceStatus,
}

/// What reading one source gave. Only `read` carries a count, so a source
/// that failed can never be shown as an empty one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectSourceStatus {
    /// `count` is the entries it named, before skipping, merging and
    /// folding, including entries it skipped itself (see
    /// `xt_ingest::native::saved_projects`).
    Read {
        count: u32,
    },
    Missing,
    Unreadable,
    UnexpectedLayout,
    TooLarge,
}

/// Folders an app named that are not listed, per reason. `home_folder` is the
/// home folder itself; `container_folder` is a folder that only groups other
/// listed projects and is not a repository of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct ProjectSkipCounts {
    pub invalid_path: u32,
    pub missing_folder: u32,
    pub temporary: u32,
    pub app_library: u32,
    pub app_internal: u32,
    pub codex_task: u32,
    pub remote: u32,
    pub multi_root_workspace: u32,
    pub unreadable_entry: u32,
    pub home_folder: u32,
    pub container_folder: u32,
}

impl From<ProjectSource> for ProjectSourceId {
    fn from(source: ProjectSource) -> Self {
        match source {
            ProjectSource::ClaudeCode => Self::ClaudeCode,
            ProjectSource::ClaudeApp => Self::ClaudeApp,
            ProjectSource::CodexApp => Self::CodexApp,
            ProjectSource::CodexCli => Self::CodexCli,
            ProjectSource::Cursor => Self::Cursor,
        }
    }
}

impl From<SourceStatus> for ProjectSourceStatus {
    fn from(status: SourceStatus) -> Self {
        match status {
            SourceStatus::Read { count } => Self::Read { count },
            SourceStatus::Missing => Self::Missing,
            SourceStatus::Unreadable => Self::Unreadable,
            SourceStatus::UnexpectedLayout => Self::UnexpectedLayout,
            SourceStatus::TooLarge => Self::TooLarge,
        }
    }
}

impl From<SourceReport> for ProjectSourceReport {
    fn from(report: SourceReport) -> Self {
        Self {
            source: report.source.into(),
            status: report.status.into(),
        }
    }
}

impl From<SkipCounts> for ProjectSkipCounts {
    fn from(skipped: SkipCounts) -> Self {
        let SkipCounts {
            invalid_path,
            missing_folder,
            temporary,
            app_library,
            app_internal,
            codex_task,
            remote,
            multi_root_workspace,
            unreadable_entry,
            home_folder,
            container_folder,
        } = skipped;
        Self {
            invalid_path,
            missing_folder,
            temporary,
            app_library,
            app_internal,
            codex_task,
            remote,
            multi_root_workspace,
            unreadable_entry,
            home_folder,
            container_folder,
        }
    }
}

impl From<SavedProject> for ProjectEntry {
    fn from(project: SavedProject) -> Self {
        Self {
            name: project.name,
            // The reader lists only folders whose path is valid text, so
            // this conversion is exact.
            path: project.path.to_string_lossy().into_owned(),
            sources: project.sources.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SavedProjects> for ProjectList {
    fn from(listed: SavedProjects) -> Self {
        Self::Loaded(ProjectListLoaded {
            projects: listed.projects.into_iter().map(Into::into).collect(),
            sources: listed.sources.into_iter().map(Into::into).collect(),
            skipped: listed.skipped.into(),
        })
    }
}
