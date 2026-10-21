//! The user's projects, read **only** from the coding apps' own saved files.
//!
//! Five files name folders the user works in: Claude Code's `~/.claude.json`,
//! the Claude app's session presets, the Codex app's global state, the Codex
//! CLI's `config.toml` and Cursor's per-window `workspace.json` files. Each is
//! opened for reading only, refused past a size cap, and parsed only for the
//! fields that name a folder (and, for the two apps that have one, the
//! project's name). Nothing here runs Git, reads a conversation, opens the
//! database, writes a file or logs a path.
//!
//! Every source reports exactly one status, and a broken source never fails
//! the list: its status says why it gave nothing. A JSON file, and every
//! object inside it this reads, must really be an object; anything else is
//! `unexpected_layout`, never read by position.
//!
//! A source that was read reports a count: the **entries it named** — each
//! folder key, preset, root path or Cursor window with a `workspace.json` —
//! before skipping, merging and folding. An entry the source itself could not
//! use (a Claude preset without a folder, a remote, multi-root or unreadable
//! Cursor window) is still counted there, and also under its skip reason.
//!
//! Every folder found then goes through one rule per question, each a single
//! function below:
//!
//! - **Normalize** ([`lexical_path`], then the file system's own canonical
//!   path): one spelling per folder, so a folder several apps name is one
//!   project that remembers every source.
//! - **Skip** ([`skip_reason`]): folders that are gone, temporary, inside
//!   `~/Library`, the apps' own folders, Codex task folders, or the home
//!   folder itself. Each skip is counted by reason.
//! - **Fold worktrees** ([`fold_target`]): a listed folder `A/worktrees/<name>/…`
//!   or `A/.claude/worktrees/<name>/…` is a worktree of listed folder `A`: it
//!   is never listed on its own, and its sources count for `A`. A listed bare
//!   container (`A/worktrees`, `A/.claude/worktrees`) also folds into `A`. A
//!   `worktrees` segment anywhere deeper folds nothing: a folder merely nested
//!   in another stays its own project. The home folder, and any folder above
//!   it (`/` included), never absorbs anything.
//! - **Hide containers** ([`is_container`]): after folding, a folder that
//!   holds another listed project, has no `.git` entry of its own (a file
//!   or folder; only its existence is checked, no Git runs) and absorbed
//!   nothing by folding only groups other projects, such as a folder of
//!   repositories. It is not listed, its sources are dropped (never merged
//!   into anything), and it is counted as skipped. A folder that absorbed a
//!   worktree or a bare worktrees container is a project with worktrees, so
//!   it is never a container. A folder whose only nested folders were
//!   worktrees folded into it holds no other listed project, so it stays too.
//!   Containers are decided together against the folded list, so a container
//!   inside a container is hidden too.
//! - **Name** ([`project_name`]) and **order** ([`project_order`]).
//!
//! The home folder is the caller's: fixture mode has none and never calls
//! this, so nothing here can fall back to the user's own home.

use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, de::value::MapAccessDeserializer};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fmt, fs,
    io::{self, Read},
    marker::PhantomData,
    path::{Component, Path, PathBuf},
};

/// The largest saved file read. Today's real files are a few hundred
/// kilobytes at most; a bigger one is reported `too_large`, never partly read.
pub const MAX_SOURCE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The largest single Cursor `workspace.json` read. Each holds one URI.
pub const MAX_CURSOR_ENTRY_BYTES: u64 = 64 * 1024;

/// The most Cursor window folders looked at. More than this reports the
/// source `too_large` rather than reading part of it.
pub const MAX_CURSOR_WORKSPACES: usize = 10_000;

/// The bounds one read keeps to. Production always uses [`Limits::default`];
/// tests lower a bound to reach it cheaply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Limits {
    cursor_windows: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cursor_windows: MAX_CURSOR_WORKSPACES,
        }
    }
}

/// Folders whose contents are temporary by design.
pub const TEMP_ROOTS: [&str; 4] = [
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];

/// The folder that holds a project's worktrees, directly under the project
/// (`A/worktrees/x`) or under its `.claude` folder (`A/.claude/worktrees/x`).
pub const WORKTREES_SEGMENT: &str = "worktrees";
const CLAUDE_SEGMENT: &str = ".claude";

/// The entry whose presence marks a folder as a repository of its own, so it
/// is never hidden as a container ([`is_container`]).
const GIT_ENTRY: &str = ".git";

/// Each saved file, relative to the home folder.
pub const CLAUDE_CODE_FILE: &str = ".claude.json";
pub const CLAUDE_APP_FILE: &str = "Library/Application Support/Claude/ccd-session-presets.json";
pub const CODEX_APP_FILE: &str = ".codex/.codex-global-state.json";
pub const CODEX_CLI_FILE: &str = ".codex/config.toml";
pub const CURSOR_WORKSPACES_DIR: &str = "Library/Application Support/Cursor/User/workspaceStorage";

/// The app whose saved file named a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectSource {
    ClaudeCode,
    ClaudeApp,
    CodexApp,
    CodexCli,
    Cursor,
}

impl ProjectSource {
    /// Every source, in the order reports list them.
    pub const ALL: [Self; 5] = [
        Self::ClaudeCode,
        Self::ClaudeApp,
        Self::CodexApp,
        Self::CodexCli,
        Self::Cursor,
    ];
}

/// What reading one source gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    /// Read and understood. `count` is the entries it named (see the module
    /// documentation), including ones it skipped itself.
    Read { count: u32 },
    /// The file (or Cursor's folder) does not exist.
    Missing,
    /// It exists but could not be read, or is not a regular file (Cursor:
    /// not a folder).
    Unreadable,
    /// It was read but is not laid out as expected (including invalid JSON
    /// or TOML). Nothing from it is used.
    UnexpectedLayout,
    /// Past [`MAX_SOURCE_FILE_BYTES`] (Cursor: more than
    /// [`MAX_CURSOR_WORKSPACES`] windows). Nothing from it is used.
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceReport {
    pub source: ProjectSource,
    pub status: SourceStatus,
}

/// Why a folder some app named is not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// Not an absolute path, a local Claude preset with no folder (or one
    /// that is not text), a Cursor URI that does not decode, or a folder
    /// whose real path is not valid text.
    InvalidPath,
    /// The folder does not exist, or is not a directory.
    MissingFolder,
    /// Under one of [`TEMP_ROOTS`].
    Temporary,
    /// Under `~/Library/`.
    AppLibrary,
    /// The apps' own folders: under `~/.claude/` or `~/.codex/`.
    AppInternal,
    /// A Codex task folder, under `~/Documents/Codex/`.
    CodexTask,
    /// A Cursor window on a remote (non-`file://`) folder.
    Remote,
    /// A Cursor window on a multi-root `.code-workspace` file, not a folder.
    MultiRootWorkspace,
    /// A Cursor window whose `workspace.json` could not be read or names
    /// neither a folder nor a workspace.
    UnreadableEntry,
    /// The home folder itself.
    HomeFolder,
    /// A folder that only groups other listed projects ([`is_container`]).
    ContainerFolder,
}

/// How many folders were skipped, per reason. A folder named by several
/// sources (after normalizing) is counted once; an invalid path, and each
/// entry a source skipped itself, is counted per occurrence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkipCounts {
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

impl SkipCounts {
    fn add(&mut self, reason: SkipReason) {
        let slot = match reason {
            SkipReason::InvalidPath => &mut self.invalid_path,
            SkipReason::MissingFolder => &mut self.missing_folder,
            SkipReason::Temporary => &mut self.temporary,
            SkipReason::AppLibrary => &mut self.app_library,
            SkipReason::AppInternal => &mut self.app_internal,
            SkipReason::CodexTask => &mut self.codex_task,
            SkipReason::Remote => &mut self.remote,
            SkipReason::MultiRootWorkspace => &mut self.multi_root_workspace,
            SkipReason::UnreadableEntry => &mut self.unreadable_entry,
            SkipReason::HomeFolder => &mut self.home_folder,
            SkipReason::ContainerFolder => &mut self.container_folder,
        };
        *slot = slot.saturating_add(1);
    }
}

/// One listed project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedProject {
    pub name: String,
    /// The folder's canonical absolute path. Always valid text.
    pub path: PathBuf,
    /// Every source that named this folder or a worktree folded into it,
    /// in [`ProjectSource::ALL`] order.
    pub sources: Vec<ProjectSource>,
}

/// The whole answer: listed projects, each source's status and skip counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedProjects {
    /// Ordered by [`project_order`].
    pub projects: Vec<SavedProject>,
    /// One per source, in [`ProjectSource::ALL`] order.
    pub sources: Vec<SourceReport>,
    pub skipped: SkipCounts,
}

/// List the projects the coding apps' saved files under `home` name.
pub fn list_saved_projects(home: &Path) -> SavedProjects {
    list_within(home, Limits::default())
}

fn list_within(home: &Path, limits: Limits) -> SavedProjects {
    let mut skipped = SkipCounts::default();
    let mut candidates = Vec::new();
    let mut sources = Vec::with_capacity(ProjectSource::ALL.len());
    for source in ProjectSource::ALL {
        let read = read_source(home, source, limits);
        let status = match read.status {
            Status::Read => {
                let named = read.candidates.len().saturating_add(read.skips.len());
                let count = u32::try_from(named).unwrap_or(u32::MAX);
                for reason in read.skips {
                    skipped.add(reason);
                }
                candidates.extend(read.candidates);
                SourceStatus::Read { count }
            }
            Status::Missing => SourceStatus::Missing,
            Status::Unreadable => SourceStatus::Unreadable,
            Status::UnexpectedLayout => SourceStatus::UnexpectedLayout,
            Status::TooLarge => SourceStatus::TooLarge,
        };
        sources.push(SourceReport { source, status });
    }
    let homes = Homes::new(home);
    let projects = build_projects(candidates, &homes, &mut skipped);
    SavedProjects {
        projects,
        sources,
        skipped,
    }
}

// ---------------------------------------------------------------------------
// Reading the sources
// ---------------------------------------------------------------------------

/// A folder one source named, with the project name that source gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    source: ProjectSource,
    path: String,
    name: Option<String>,
}

enum Status {
    Read,
    Missing,
    Unreadable,
    UnexpectedLayout,
    TooLarge,
}

struct SourceRead {
    status: Status,
    candidates: Vec<Candidate>,
    /// Entries this source named but skipped itself.
    skips: Vec<SkipReason>,
}

impl SourceRead {
    fn failed(status: Status) -> Self {
        Self {
            status,
            candidates: Vec::new(),
            skips: Vec::new(),
        }
    }

    fn read(candidates: Vec<Candidate>) -> Self {
        Self {
            status: Status::Read,
            candidates,
            skips: Vec::new(),
        }
    }
}

/// A parsed source, or none when it is not laid out as expected.
type Parser = fn(&[u8]) -> Option<SourceRead>;

fn read_source(home: &Path, source: ProjectSource, limits: Limits) -> SourceRead {
    let (file, parse): (&str, Parser) = match source {
        ProjectSource::ClaudeCode => (CLAUDE_CODE_FILE, parse_claude_code),
        ProjectSource::ClaudeApp => (CLAUDE_APP_FILE, parse_claude_app),
        ProjectSource::CodexApp => (CODEX_APP_FILE, parse_codex_app),
        ProjectSource::CodexCli => (CODEX_CLI_FILE, parse_codex_cli),
        ProjectSource::Cursor => {
            return read_cursor(&home.join(CURSOR_WORKSPACES_DIR), limits.cursor_windows);
        }
    };
    let bytes = match read_capped(&home.join(file), MAX_SOURCE_FILE_BYTES) {
        FileRead::Bytes(bytes) => bytes,
        FileRead::Missing => return SourceRead::failed(Status::Missing),
        FileRead::Unreadable => return SourceRead::failed(Status::Unreadable),
        FileRead::TooLarge => return SourceRead::failed(Status::TooLarge),
    };
    parse(&bytes).unwrap_or_else(|| SourceRead::failed(Status::UnexpectedLayout))
}

/// A JSON value that must be an object. Serde's derived structs also accept
/// an array, read by field position; a saved file laid out that way is not
/// one this understands, so it is refused here instead.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OnlyObject<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for OnlyObject<T> {
            type Value = T;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map))
            }
        }
        deserializer
            .deserialize_map(OnlyObject(PhantomData))
            .map(Object)
    }
}

/// Parse a whole JSON file whose top level must be an object.
fn json_object<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Option<T> {
    serde_json::from_slice::<Object<T>>(bytes)
        .ok()
        .map(|object| object.0)
}

enum FileRead {
    Bytes(Vec<u8>),
    Missing,
    Unreadable,
    TooLarge,
}

/// Read a regular file, opened for reading only, refusing it past `cap`
/// bytes (also when it grows past `cap` while it is read).
fn read_capped(path: &Path, cap: u64) -> FileRead {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return FileRead::Missing,
        Err(_) => return FileRead::Unreadable,
    };
    if !metadata.is_file() {
        return FileRead::Unreadable;
    }
    if metadata.len() > cap {
        return FileRead::TooLarge;
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return FileRead::Missing,
        Err(_) => return FileRead::Unreadable,
    };
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    if file
        .take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
    {
        return FileRead::Unreadable;
    }
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
        return FileRead::TooLarge;
    }
    FileRead::Bytes(bytes)
}

/// A nonblank name, trimmed.
fn given_name(name: Option<String>) -> Option<String> {
    name.map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// `~/.claude.json`: the keys of the top-level `projects` object.
fn parse_claude_code(bytes: &[u8]) -> Option<SourceRead> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        projects: Option<BTreeMap<String, IgnoredAny>>,
    }
    let file: File = json_object(bytes)?;
    Some(SourceRead::read(
        file.projects
            .unwrap_or_default()
            .into_keys()
            .map(|path| Candidate {
                source: ProjectSource::ClaudeCode,
                path,
                name: None,
            })
            .collect(),
    ))
}

/// The Claude app's presets: every `byAccount.*.presets[]` whose target is
/// local, with its name. A local preset whose folder is missing or not text
/// is one skipped entry; containers of the wrong shape refuse the source.
fn parse_claude_app(bytes: &[u8]) -> Option<SourceRead> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default, rename = "byAccount")]
        by_account: BTreeMap<String, Object<Account>>,
    }
    #[derive(Deserialize)]
    struct Account {
        #[serde(default)]
        presets: Vec<Object<Preset>>,
    }
    #[derive(Deserialize)]
    struct Preset {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        target: Option<Object<Target>>,
    }
    #[derive(Deserialize)]
    struct Target {
        kind: String,
        #[serde(default)]
        cwd: Option<serde_json::Value>,
    }
    let file: File = json_object(bytes)?;
    let mut read = SourceRead::read(Vec::new());
    for Object(preset) in file
        .by_account
        .into_values()
        .flat_map(|Object(account)| account.presets)
    {
        let Some(Object(target)) = preset.target else {
            continue;
        };
        if target.kind != "local" {
            continue;
        }
        match target.cwd {
            Some(serde_json::Value::String(path)) => read.candidates.push(Candidate {
                source: ProjectSource::ClaudeApp,
                path,
                name: given_name(preset.name),
            }),
            _ => read.skips.push(SkipReason::InvalidPath),
        }
    }
    Some(read)
}

/// The Codex app's global state: every `local-projects.*` root path, with
/// that project's name.
fn parse_codex_app(bytes: &[u8]) -> Option<SourceRead> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default, rename = "local-projects")]
        local_projects: BTreeMap<String, Object<Project>>,
    }
    #[derive(Deserialize)]
    struct Project {
        #[serde(default)]
        name: Option<String>,
        #[serde(default, rename = "rootPaths")]
        root_paths: Vec<String>,
    }
    let file: File = json_object(bytes)?;
    let mut candidates = Vec::new();
    for Object(project) in file.local_projects.into_values() {
        let name = given_name(project.name);
        for path in project.root_paths {
            candidates.push(Candidate {
                source: ProjectSource::CodexApp,
                path,
                name: name.clone(),
            });
        }
    }
    Some(SourceRead::read(candidates))
}

/// The Codex CLI's `config.toml`: the keys of its `[projects."<path>"]`
/// tables.
fn parse_codex_cli(bytes: &[u8]) -> Option<SourceRead> {
    let table: toml::Table = std::str::from_utf8(bytes).ok()?.parse().ok()?;
    let projects = match table.get("projects") {
        None => return Some(SourceRead::read(Vec::new())),
        Some(toml::Value::Table(projects)) => projects,
        Some(_) => return None,
    };
    Some(SourceRead::read(
        projects
            .keys()
            .map(|path| Candidate {
                source: ProjectSource::CodexCli,
                path: path.clone(),
                name: None,
            })
            .collect(),
    ))
}

/// Cursor: one `workspace.json` per window folder. A window folder without
/// one (an empty window) names nothing; any other entry that is not a folder
/// is ignored. More than `max_windows` entries refuses the source.
fn read_cursor(dir: &Path, max_windows: usize) -> SourceRead {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return SourceRead::failed(Status::Missing);
        }
        Err(_) => return SourceRead::failed(Status::Unreadable),
    };
    let mut read = SourceRead::read(Vec::new());
    for (seen, entry) in entries.enumerate() {
        if seen >= max_windows {
            return SourceRead::failed(Status::TooLarge);
        }
        let Ok(entry) = entry else {
            read.skips.push(SkipReason::UnreadableEntry);
            continue;
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let bytes = match read_capped(&entry.path().join("workspace.json"), MAX_CURSOR_ENTRY_BYTES)
        {
            FileRead::Bytes(bytes) => bytes,
            FileRead::Missing => continue,
            FileRead::Unreadable | FileRead::TooLarge => {
                read.skips.push(SkipReason::UnreadableEntry);
                continue;
            }
        };
        match cursor_entry(&bytes) {
            Ok(path) => read.candidates.push(Candidate {
                source: ProjectSource::Cursor,
                path,
                name: None,
            }),
            Err(reason) => read.skips.push(reason),
        }
    }
    read
}

/// One Cursor `workspace.json`: its local folder, or why it has none.
fn cursor_entry(bytes: &[u8]) -> Result<String, SkipReason> {
    #[derive(Deserialize)]
    struct Workspace {
        #[serde(default)]
        folder: Option<String>,
        #[serde(default)]
        workspace: Option<String>,
    }
    let workspace: Workspace = json_object(bytes).ok_or(SkipReason::UnreadableEntry)?;
    match (workspace.folder, workspace.workspace) {
        (Some(folder), _) => file_uri_path(&folder),
        (None, Some(_)) => Err(SkipReason::MultiRootWorkspace),
        (None, None) => Err(SkipReason::UnreadableEntry),
    }
}

/// The local path a `file://` URI names, percent-decoded. Any other scheme,
/// or a `file://` URI on another host, is remote.
fn file_uri_path(uri: &str) -> Result<String, SkipReason> {
    const SCHEME: &str = "file://";
    let rest = match uri.get(..SCHEME.len()) {
        Some(scheme) if scheme.eq_ignore_ascii_case(SCHEME) => &uri[SCHEME.len()..],
        _ => return Err(SkipReason::Remote),
    };
    let path = if rest.starts_with('/') {
        rest
    } else {
        match rest.strip_prefix("localhost") {
            Some(path) if path.starts_with('/') => path,
            _ => return Err(SkipReason::Remote),
        }
    };
    percent_decode(path).ok_or(SkipReason::InvalidPath)
}

/// `%XX` escapes decoded, each exactly two ASCII hex digits; the result must
/// be valid text.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            let hex = std::str::from_utf8(hex).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

// ---------------------------------------------------------------------------
// The rules
// ---------------------------------------------------------------------------

/// The home folder as given and as the file system resolves it. A rule about
/// "under the home folder" holds for either spelling.
struct Homes {
    given: PathBuf,
    canonical: Option<PathBuf>,
}

impl Homes {
    fn new(home: &Path) -> Self {
        let given = lexical_path(&home.to_string_lossy()).unwrap_or_else(|| home.to_path_buf());
        let canonical = fs::canonicalize(home).ok().filter(|real| *real != given);
        Self { given, canonical }
    }

    fn each(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.given.as_path()).chain(self.canonical.as_deref())
    }

    /// The home folder itself, in either spelling.
    fn is_home(&self, path: &Path) -> bool {
        self.each().any(|home| home == path)
    }

    /// The home folder itself, or a folder above it (`/` included).
    fn is_home_or_above(&self, path: &Path) -> bool {
        self.each().any(|home| home.starts_with(path))
    }
}

/// Rule 1, first half: one lexical spelling of an absolute path — repeated
/// and trailing separators dropped, `.` removed and `..` applied. None when
/// the path is not absolute. The second half is the file system's canonical
/// path, which resolves links such as `/tmp` to `/private/tmp`.
fn lexical_path(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return None;
    }
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normal.push(Component::RootDir.as_os_str()),
            Component::Normal(part) => normal.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            Component::Prefix(_) => return None,
        }
    }
    Some(normal)
}

/// Rule 2: why a folder is not a project, from its path alone, or none.
fn skip_reason(path: &Path, homes: &Homes) -> Option<SkipReason> {
    if TEMP_ROOTS.iter().any(|root| path.starts_with(root)) {
        return Some(SkipReason::Temporary);
    }
    if homes.is_home(path) {
        return Some(SkipReason::HomeFolder);
    }
    for home in homes.each() {
        if path.starts_with(home.join("Library")) {
            return Some(SkipReason::AppLibrary);
        }
        if path.starts_with(home.join(".claude")) || path.starts_with(home.join(".codex")) {
            return Some(SkipReason::AppInternal);
        }
        if path.starts_with(home.join("Documents").join("Codex")) {
            return Some(SkipReason::CodexTask);
        }
    }
    None
}

/// Whether the path `inside` a listed folder makes it a worktree (or the
/// worktrees container itself) of that folder: only through a `worktrees`
/// folder directly under it or directly under its `.claude` folder.
fn is_worktree_path(inside: &Path) -> bool {
    let parts: Vec<&OsStr> = inside.components().map(Component::as_os_str).collect();
    let below_claude = match parts.as_slice() {
        [first, rest @ ..] if *first == CLAUDE_SEGMENT => rest,
        all => all,
    };
    matches!(below_claude, [segment, ..] if *segment == WORKTREES_SEGMENT)
}

/// Rule 3: the listed folder `folder` folds into, if any: the deepest other
/// listed folder it sits in as a worktree or worktrees container
/// ([`is_worktree_path`]). The home folder and every folder above it never
/// absorb anything.
fn fold_target<'a>(folder: &Path, listed: &'a [PathBuf], homes: &Homes) -> Option<&'a PathBuf> {
    listed
        .iter()
        .filter(|project| project.as_path() != folder && !homes.is_home_or_above(project))
        .filter(|project| folder.strip_prefix(project).is_ok_and(is_worktree_path))
        .max_by_key(|project| project.components().count())
}

/// Rule 3, after folding: whether the listed project `folder` only groups
/// other projects: it absorbed nothing by folding (`absorbed` is false: no
/// worktree or bare worktrees container folded into it), at least one other
/// project in `listed` (the list after worktree folding) is strictly inside
/// it, and it has no `.git` entry of its own. The `.git` check is a plain existence check of that entry (a file,
/// folder or link), made only when the first part holds; no Git runs. Only a
/// definite "not found" counts as no entry: when the check itself fails, the
/// folder stays listed.
fn is_container(folder: &Path, absorbed: bool, listed: &[PathBuf]) -> bool {
    !absorbed
        && listed
            .iter()
            .any(|project| project.as_path() != folder && project.starts_with(folder))
        && matches!(
            fs::symlink_metadata(folder.join(GIT_ENTRY)),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        )
}

/// Rule 4: the Claude app's preset name, else the Codex app's project name,
/// else the folder's last path component.
fn project_name(path: &Path, claude_app: Option<&str>, codex_app: Option<&str>) -> String {
    claude_app
        .or(codex_app)
        .map(str::to_owned)
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Rule 5: by name ignoring case, then by path.
fn project_order(a: &SavedProject, b: &SavedProject) -> Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.path.cmp(&b.path))
}

/// Everything known about one folder while sources are merged.
#[derive(Default)]
struct Folder {
    sources: BTreeSet<ProjectSource>,
    claude_app_name: Option<String>,
    codex_app_name: Option<String>,
}

impl Folder {
    fn absorb(&mut self, other: Folder) {
        self.sources.extend(other.sources);
        if self.claude_app_name.is_none() {
            self.claude_app_name = other.claude_app_name;
        }
        if self.codex_app_name.is_none() {
            self.codex_app_name = other.codex_app_name;
        }
    }

    fn named(candidate: Candidate) -> Self {
        let mut folder = Folder::default();
        folder.sources.insert(candidate.source);
        match candidate.source {
            ProjectSource::ClaudeApp => folder.claude_app_name = candidate.name,
            ProjectSource::CodexApp => folder.codex_app_name = candidate.name,
            _ => {}
        }
        folder
    }
}

fn build_projects(
    candidates: Vec<Candidate>,
    homes: &Homes,
    skipped: &mut SkipCounts,
) -> Vec<SavedProject> {
    // Merge by lexical spelling first, so a reason that needs no file system
    // (a temporary or app folder, even one that no longer exists) is decided
    // and counted once per folder.
    let mut lexical: BTreeMap<PathBuf, Folder> = BTreeMap::new();
    for candidate in candidates {
        let Some(path) = lexical_path(&candidate.path) else {
            skipped.add(SkipReason::InvalidPath);
            continue;
        };
        lexical
            .entry(path)
            .or_default()
            .absorb(Folder::named(candidate));
    }
    // Then by the real path, which only an existing directory has.
    let mut canonical: BTreeMap<PathBuf, Folder> = BTreeMap::new();
    for (path, folder) in lexical {
        if let Some(reason) = skip_reason(&path, homes) {
            skipped.add(reason);
            continue;
        }
        if !fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
            skipped.add(SkipReason::MissingFolder);
            continue;
        }
        let Ok(real) = fs::canonicalize(&path) else {
            skipped.add(SkipReason::MissingFolder);
            continue;
        };
        if real.to_str().is_none() {
            skipped.add(SkipReason::InvalidPath);
            continue;
        }
        canonical.entry(real).or_default().absorb(folder);
    }
    // The real path can reveal what its spelling hid (`/tmp` is
    // `/private/tmp`).
    canonical.retain(|path, _| match skip_reason(path, homes) {
        Some(reason) => {
            skipped.add(reason);
            false
        }
        None => true,
    });

    let listed: Vec<PathBuf> = canonical.keys().cloned().collect();
    let mut root_of: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for folder in &listed {
        // Follow folds to the end: a worktree of a listed worktree lands in
        // the project the first was made from. Each step is to a strictly
        // shorter path, so this ends.
        let mut root = folder;
        while let Some(target) = fold_target(root, &listed, homes) {
            root = target;
        }
        if root != folder {
            root_of.insert(folder.clone(), root.clone());
        }
    }

    let mut projects: BTreeMap<PathBuf, Folder> = BTreeMap::new();
    let mut folded = Vec::new();
    for (path, folder) in canonical {
        match root_of.remove(&path) {
            Some(root) => folded.push((root, folder)),
            None => {
                projects.insert(path, folder);
            }
        }
    }
    let mut absorbers: BTreeSet<PathBuf> = BTreeSet::new();
    for (root, folder) in folded {
        if let Some(project) = projects.get_mut(&root) {
            // A worktree's sources count for its project; its own preset or
            // project name never names the project.
            project.sources.extend(folder.sources);
            absorbers.insert(root);
        }
    }

    // Containers are decided together against the folded list; a hidden
    // container's sources go nowhere.
    let folded_list: Vec<PathBuf> = projects.keys().cloned().collect();
    projects.retain(|path, _| {
        if is_container(path, absorbers.contains(path), &folded_list) {
            skipped.add(SkipReason::ContainerFolder);
            false
        } else {
            true
        }
    });

    let mut projects: Vec<SavedProject> = projects
        .into_iter()
        .map(|(path, folder)| SavedProject {
            name: project_name(
                &path,
                folder.claude_app_name.as_deref(),
                folder.codex_app_name.as_deref(),
            ),
            sources: folder.sources.into_iter().collect(),
            path,
        })
        .collect();
    projects.sort_by(project_order);
    projects
}

#[cfg(test)]
mod tests;
