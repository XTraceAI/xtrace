//! The wire contract of "Add from a link" in the Add a skill dialog.
//!
//! Every decision is made in Rust: what the link names
//! (`xt_judge::skill_link::parse`), which skills it holds, whether each can be
//! kept and why not (`xt_judge::skill_link::keep_states`), and what keeping
//! did (`crate::skill_link`). The renderer shows these fields and works
//! nothing out again. Folders are shown under the home folder as `~/…`.
use crate::grading_dto::GradingInstalledRubric;
use serde::Serialize;
use ts_rs::TS;

/// What `grading_link_lookup` found. Only `found` keeps anything (in a
/// private staging folder, until it is kept, discarded, replaced by another
/// look-up, or XTrace quits).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingLinkLookup {
    Found(GradingLinkFound),
    /// Not a public GitHub link to a repository or a folder in one.
    NotGithubLink,
    /// GitHub has no public repository, branch, tag or commit by that name.
    NotFound,
    /// The repository has no folder at the link's path at that commit.
    PathNotFound,
    /// No folder with a `SKILL.md` is at or below the link's folder.
    NoSkill,
    /// GitHub refused more look-ups from this network for now.
    RateLimited,
    /// The download or what it unpacks to is past XTrace's bounds.
    TooLarge {
        limit: GradingLinkLimit,
    },
    /// The download has an entry XTrace never unpacks (an absolute path, a
    /// `..`, a link out of the repository, a hard link, a device).
    UnsafeArchive,
    /// The download is not an archive XTrace can read.
    BadArchive,
    /// GitHub did not answer in time.
    TimedOut,
    /// Cancelled, or replaced by another look-up.
    Cancelled,
    /// GitHub could not be reached, or answered with an error.
    NetworkFailed,
    /// Fixture mode never looks anything up.
    NotConfigured,
    /// XTrace hit an error (its staging folder could not be made, or it is
    /// closing).
    Failed,
}

/// Which bound a download passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GradingLinkLimit {
    /// The download itself (50 MB).
    Download,
    /// Entries in the archive (5,000).
    Entries,
    /// Bytes unpacked (100 MB).
    Unpacked,
}

/// A look-up's preview: where the skills come from and each one found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingLinkFound {
    /// Names this look-up for keeping or discarding it.
    pub lookup_id: String,
    pub source: GradingLinkSource,
    /// By path in the repository.
    pub skills: Vec<GradingLinkSkill>,
    /// How many rows can be kept.
    pub keepable: u32,
    /// More skills were found than are listed.
    pub incomplete: bool,
}

/// The one commit a look-up downloaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingLinkSource {
    /// `github.com/<owner>/<repo>`.
    pub repository: String,
    /// The link's folder in the repository; empty for its top.
    pub path: String,
    /// The branch, tag or commit the link named; none for the default
    /// branch.
    pub reference: Option<String>,
    /// The full commit downloaded.
    pub commit: String,
    /// Its first seven digits, as shown.
    pub short_commit: String,
}

/// One skill found at a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingLinkSkill {
    /// Its folder in the repository (empty for the top); keeping sends this.
    pub path: String,
    /// `SKILL.md`'s `name`, or its folder's name.
    pub name: String,
    /// One line from `SKILL.md`'s `description`; empty when none.
    pub description: String,
    /// Files (and links) in its folder.
    pub files: u32,
    /// Bytes of its files.
    #[ts(type = "number")]
    pub bytes: u64,
    /// Its `xtrace-rubric.md`, read with the same rubric reader as an
    /// installed skill's.
    pub rubric: GradingInstalledRubric,
    /// The folder it would get in `~/.agents/skills`; none when it has no
    /// usable name.
    pub folder_name: Option<String>,
    pub keep: GradingLinkKeepState,
}

/// Whether one found skill can be kept now, or why not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingLinkKeepState {
    Keepable,
    /// Something in `~/.agents/skills` has its name (shown as `~/…`).
    NameInAgents {
        folder: String,
    },
    /// Something in `~/.claude/skills` has its name (shown as `~/…`).
    NameInClaude {
        folder: String,
    },
    /// A skill listed before it (at this path) gets the same folder name;
    /// only that one can be kept.
    SameNameAbove {
        path: String,
    },
    /// Neither its `SKILL.md` name nor its folder's name can be a folder
    /// name.
    UnusableName,
    /// More files or data than XTrace copies for a grading.
    TooLarge,
    /// A link in it names a folder, nothing, or something outside it.
    LinkLeavesFolder,
    /// Its `SKILL.md` is a link, too large, or not UTF-8 text.
    InstructionsUnreadable,
}

/// What `grading_link_keep` did. Only `done` kept anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingLinkKeep {
    /// Each chosen skill, in the preview's order, and what happened to it.
    Done {
        skills: Vec<GradingLinkKept>,
        /// Where each kept skill came from was saved
        /// (`<XTrace data>/linked-skills.json`).
        recorded: bool,
    },
    /// A chosen skill cannot be kept any more (its name was taken
    /// meanwhile, or it is not in the preview); nothing was kept. The
    /// preview as it is now.
    Changed(GradingLinkFound),
    /// No skill was chosen.
    NothingChosen,
    /// That look-up is gone: replaced by another, discarded, or XTrace
    /// restarted.
    Expired,
    /// `~/.agents/skills` is not a folder XTrace can use (a link or a
    /// file); nothing was kept.
    AgentsFolderUnusable,
    NotConfigured,
    Failed,
}

/// One chosen skill after keeping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct GradingLinkKept {
    pub path: String,
    pub name: String,
    pub result: GradingLinkKeptResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingLinkKeptResult {
    /// It is in this folder now (shown as `~/…`).
    Kept {
        folder: String,
        claude: GradingLinkClaude,
    },
    /// Its name in `~/.agents/skills` was taken meanwhile; nothing was kept.
    NameTaken,
    /// It could not be copied; nothing was kept.
    Failed,
}

/// Whether `~/.claude/skills/<name>` now links to the kept folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GradingLinkClaude {
    /// This link was made (shown as `~/…`).
    Linked { link: String },
    /// Something there already has that name; it was left alone.
    NameTaken,
    /// There is no `~/.claude` folder.
    NoClaudeFolder,
    /// `~/.claude/skills` is not a folder, or the link could not be made.
    Failed,
}
