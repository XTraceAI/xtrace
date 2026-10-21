//! The skills the user installed, found where the coding agents keep them,
//! and whether each one has a valid rubric ([`discover`]).
//!
//! Where skills are looked for, all under one home folder (the native home
//! the app selected; fixture mode has none and never reads the real one):
//!
//! * `~/.claude/skills/<folder>/SKILL.md` — Claude Code's own skills.
//! * `~/.agents/skills/<folder>/SKILL.md` — skills shared between agents.
//! * `~/.codex/skills/<folder>/SKILL.md` — Codex's skills.
//! * Claude Code plugins: `~/.claude/plugins/installed_plugins.json` names
//!   each plugin's installs (`plugins.<name>@<marketplace>[]` with `scope`
//!   and `installPath`); only a `user`-scoped install of a plugin that
//!   `~/.claude/settings.json` turns on (`enabledPlugins.<name>@<marketplace>`
//!   is `true`, the only key read from that file) is looked in (the first
//!   such install of each plugin), at `<installPath>/skills/<folder>/SKILL.md`.
//!   The install folder must resolve inside `~/.claude/plugins`.
//!
//! Reading is bounded and never follows a link out of a folder: a skills
//! folder, a skill folder or a `SKILL.md` that is a link is not read, at
//! most [`MAX_ENTRIES_PER_FOLDER`] entries of one skills folder and
//! [`MAX_PLUGINS`] plugin installs are looked at, at most [`MAX_SKILLS`]
//! skills are listed, `SKILL.md` is read as a regular file of at most
//! [`MAX_INSTRUCTIONS_BYTES`], and each JSON file as one of at most
//! [`MAX_JSON_BYTES`]. Hidden entries (a leading `.`) are not skills. A
//! skill folder that is a link, has an unreadable `SKILL.md` or a name that
//! is not UTF-8 is counted in [`Discovery::skipped`], never listed. A skill
//! folder that is a link to a skill folder directly in another skills folder
//! read here (say `~/.claude/skills/x` → `~/.agents/skills/x`) is neither
//! followed nor counted: that skill is listed once, from its real folder.
//! A plugin list or settings file that cannot be used lists no plugin and
//! says why ([`Discovery::plugins_unread`]).
//!
//! Each skill carries a [`Location`] that finds it again: its folder, or for
//! a plugin's skill the plugin and the skill's folder name, resolved to the
//! plugin's current install by [`locate`] with the same rules.
//!
//! For each skill: the `name` and the first line of the `description` from
//! `SKILL.md`'s front matter ([`front_matter`]; the folder's name when it has
//! no name), and its rubric ([`RubricStatus`]): the one that applies
//! ([`crate::rubric_store::applicable`]: the folder's own `xtrace-rubric.md`,
//! else one XTrace keeps for it), ready exactly when the folder opens as a
//! grading skill with it ([`GradingSkill::open_with`]), missing when there is
//! none, otherwise the rubric's typed problem. Nothing is written and nothing
//! is run.

use crate::{
    rubric::{Rubric, RubricError},
    rubric_store::{self, RubricStore},
    skill::{GradingSkill, INSTRUCTIONS_FILE, MAX_INSTRUCTIONS_BYTES, SkillError},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use xt_probes::regular_file;

/// Claude Code's skills folder under the home folder.
pub const CLAUDE_SKILLS: &str = ".claude/skills";
/// The shared agents' skills folder under the home folder.
pub const AGENTS_SKILLS: &str = ".agents/skills";
/// Codex's skills folder under the home folder.
pub const CODEX_SKILLS: &str = ".codex/skills";
/// Claude Code's plugins folder under the home folder.
pub const CLAUDE_PLUGINS: &str = ".claude/plugins";
/// The list of installed plugins inside [`CLAUDE_PLUGINS`].
pub const INSTALLED_PLUGINS: &str = "installed_plugins.json";
/// Claude Code's user settings under the home folder; only
/// `enabledPlugins` is read from it.
pub const CLAUDE_SETTINGS: &str = ".claude/settings.json";
/// A plugin's skills folder inside its install folder.
pub const PLUGIN_SKILLS: &str = "skills";
/// The only plugin install scope that is looked in.
pub const USER_SCOPE: &str = "user";
/// Most entries of one skills folder that are looked at.
pub const MAX_ENTRIES_PER_FOLDER: usize = 500;
/// Most plugin installs that are looked in.
pub const MAX_PLUGINS: usize = 500;
/// Most skills one discovery lists.
pub const MAX_SKILLS: usize = 2_000;
/// Largest `installed_plugins.json` or `settings.json` read.
pub const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// Longest skill name kept, in characters.
pub const MAX_NAME_CHARS: usize = 200;
/// Longest description line kept, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 300;
/// Most front matter lines read before giving up on finding its end.
pub const MAX_FRONT_MATTER_LINES: usize = 400;

/// Where a skill was found.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// `~/.claude/skills`.
    Claude,
    /// `~/.agents/skills`.
    Agents,
    /// `~/.codex/skills`.
    Codex,
    /// A Claude Code plugin, by its name (without the marketplace).
    Plugin(String),
}

/// What stays the same for a skill while it is installed, so a skill added
/// to Scores is found again ([`locate`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A skill in one of the three skills folders: its folder as found.
    Folder(PathBuf),
    /// A plugin's skill: the plugin's key in the installed list
    /// (`<name>@<marketplace>`) and the skill's folder name in the plugin's
    /// `skills` folder. A plugin update installs into a new folder; this
    /// stays the same and is resolved to the current one each time.
    Plugin { key: String, skill: String },
}

/// The start of a plugin skill's [`Location::text`].
pub const PLUGIN_LOCATION_PREFIX: &str = "plugin:";

impl Location {
    /// The stored form: the folder path, or `plugin:<key>/<skill>`. None for
    /// a folder whose path is not UTF-8.
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Folder(folder) => folder.to_str().map(str::to_owned),
            Self::Plugin { key, skill } => Some(format!("{PLUGIN_LOCATION_PREFIX}{key}/{skill}")),
        }
    }

    /// Read [`Self::text`] back: `plugin:<key>/<skill>` with a non-empty key
    /// and a plain skill folder name, or an absolute folder path.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(rest) = text.strip_prefix(PLUGIN_LOCATION_PREFIX) {
            let (key, skill) = rest.rsplit_once('/')?;
            (!key.is_empty() && plain_name(skill)).then(|| Self::Plugin {
                key: key.to_owned(),
                skill: skill.to_owned(),
            })
        } else {
            let folder = Path::new(text);
            folder
                .is_absolute()
                .then(|| Self::Folder(folder.to_path_buf()))
        }
    }

    /// The plugin's name (without the marketplace), for a plugin's skill.
    pub fn plugin(&self) -> Option<&str> {
        match self {
            Self::Folder(_) => None,
            Self::Plugin { key, .. } => Some(plugin_name(key)),
        }
    }
}

/// A folder name a skill can have: not empty, not hidden, no `/`.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('/')
}

/// A plugin's name: its key without `@<marketplace>`.
fn plugin_name(key: &str) -> &str {
    key.split_once('@').map_or(key, |(name, _)| name)
}

/// The folder a skill at `location` is in now: a skills folder's skill as
/// stored; a plugin's skill in the `skills` folder of the plugin's current
/// install, by exactly the rules [`discover`] lists plugins with (enabled,
/// user-scoped, inside the plugins folder, its `skills` folder a real
/// folder). None when the plugin is not installed and enabled that way now,
/// or its list cannot be read. Whether the folder is still a skill is the
/// caller's to check.
pub fn locate(home: &Path, location: &Location) -> Option<PathBuf> {
    match location {
        Location::Folder(folder) => Some(folder.clone()),
        Location::Plugin { key, skill } => {
            let (_, root) = plugin_installs(home)
                .installs
                .into_iter()
                .find(|(installed, _)| installed == key)?;
            let skills = root.join(PLUGIN_SKILLS);
            std::fs::symlink_metadata(&skills)
                .is_ok_and(|meta| meta.is_dir())
                .then(|| skills.join(skill))
        }
    }
}

/// A skill's rubric, as XTrace reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RubricStatus {
    /// The folder opens as a grading skill with this rubric.
    Ready(Rubric),
    /// The folder has no `xtrace-rubric.md`.
    Missing,
    /// The folder's `xtrace-rubric.md` is refused for this reason.
    Problem(RubricError),
}

/// One installed skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledSkill {
    /// The skill's folder as found (not resolved further), UTF-8.
    pub folder: PathBuf,
    /// What finds it again: for a plugin's skill, not the versioned folder.
    pub location: Location,
    pub source: Source,
    /// `SKILL.md`'s `name`, or the folder's name when it has none.
    pub name: String,
    /// The first line of `SKILL.md`'s `description`; empty when it has none.
    pub description: String,
    pub rubric: RubricStatus,
    /// Whether that rubric is one XTrace keeps ([`RubricStore`]) rather than
    /// the folder's own.
    pub rubric_kept_by_xtrace: bool,
}

/// A Claude Code file the plugin list is read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFile {
    /// `~/.claude/plugins` itself.
    PluginsFolder,
    /// `~/.claude/plugins/installed_plugins.json`.
    InstalledPlugins,
    /// `~/.claude/settings.json`.
    Settings,
}

/// Why that file could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginReadFailure {
    /// It is a link.
    Link,
    /// It could not be read, or is larger than [`MAX_JSON_BYTES`].
    Unreadable,
    /// It is not JSON.
    Malformed,
    /// It is JSON of a shape XTrace does not know.
    UnknownShape,
}

/// Why plugins' skills may be missing from a discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginProblem {
    pub file: PluginFile,
    pub failure: PluginReadFailure,
}

/// What one look through the home folder found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discovery {
    /// In the order found: Claude, Agents, Codex, then plugins by name; by
    /// folder name within each.
    pub skills: Vec<InstalledSkill>,
    /// Skill folders found but not listed: a link (the skill folder, its
    /// `SKILL.md`, or a skills folder itself), an unreadable or too large
    /// `SKILL.md`, a name that is not UTF-8. A skill folder that is a link
    /// to a skill folder directly in another skills folder read here is not
    /// counted: that skill is listed from there.
    pub skipped: u32,
    /// Not every skill is listed: some skills folder had more entries than
    /// are looked at, more skills or plugins were installed than are
    /// listed, or the plugin list could not be read ([`Self::plugins_unread`]).
    pub incomplete: bool,
    /// Why some or all plugins' skills are not listed: the first problem
    /// met reading the plugin list.
    pub plugins_unread: Option<PluginProblem>,
}

/// Every installed skill under `home`, bounded as the module says, with
/// only the rubrics in the skills' own folders.
pub fn discover(home: &Path) -> Discovery {
    discover_with(home, None)
}

/// [`discover`], with the rubrics XTrace keeps in `store` for skills whose
/// folder has none.
pub fn discover_with(home: &Path, store: Option<&RubricStore>) -> Discovery {
    let mut found = Discovery::default();
    let plugins = plugin_installs(home);
    found.skipped += plugins.skipped;
    found.incomplete = plugins.incomplete || plugins.problem.is_some();
    found.plugins_unread = plugins.problem;
    let mut folders: Vec<(PathBuf, Source, Option<String>)> = [
        (CLAUDE_SKILLS, Source::Claude),
        (AGENTS_SKILLS, Source::Agents),
        (CODEX_SKILLS, Source::Codex),
    ]
    .into_iter()
    .map(|(folder, source)| (home.join(folder), source, None))
    .collect();
    for (key, root) in plugins.installs {
        let source = Source::Plugin(plugin_name(&key).to_owned());
        folders.push((root.join(PLUGIN_SKILLS), source, Some(key)));
    }
    // The skills folders read here, resolved: a link to a skill directly in
    // one of them is that skill, listed from its own folder.
    let read: Vec<PathBuf> = folders
        .iter()
        .filter(|(folder, ..)| std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.is_dir()))
        .filter_map(|(folder, ..)| std::fs::canonicalize(folder).ok())
        .collect();
    for (folder, source, key) in &folders {
        read_skills_folder(folder, source, key.as_deref(), &read, store, &mut found);
    }
    found
}

/// Whether the link at `link` names a (non-hidden) skill folder directly in
/// one of the resolved skills folders `read`.
fn listed_elsewhere(link: &Path, read: &[PathBuf]) -> bool {
    std::fs::canonicalize(link).is_ok_and(|target| {
        target
            .parent()
            .is_some_and(|parent| read.iter().any(|folder| folder == parent))
            && target
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(plain_name)
            && std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_dir())
    })
}

/// The skills directly inside one skills folder, which must be a real
/// folder (not a link); a missing one has none. `plugin` is the plugin's
/// key for a plugin's skills folder.
fn read_skills_folder(
    folder: &Path,
    source: &Source,
    plugin: Option<&str>,
    read: &[PathBuf],
    store: Option<&RubricStore>,
    found: &mut Discovery,
) {
    let Ok(meta) = std::fs::symlink_metadata(folder) else {
        return;
    };
    if !meta.is_dir() {
        found.skipped += 1;
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        found.skipped += 1;
        return;
    };
    let mut names = Vec::new();
    for entry in entries {
        if names.len() == MAX_ENTRIES_PER_FOLDER {
            found.incomplete = true;
            break;
        }
        let Ok(entry) = entry else {
            found.skipped += 1;
            continue;
        };
        names.push(entry.file_name());
    }
    names.sort();
    for name in names {
        let Some(text) = name.to_str() else {
            found.skipped += 1;
            continue;
        };
        if text.starts_with('.') {
            continue;
        }
        let path = folder.join(text);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            found.skipped += 1;
            continue;
        };
        if meta.file_type().is_symlink() {
            if !listed_elsewhere(&path, read) {
                found.skipped += 1;
            }
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        // A folder without SKILL.md is not a skill; one whose SKILL.md
        // cannot be read as a regular file of bounded size is skipped.
        let instructions = match regular_file::read_capped(
            &path.join(INSTRUCTIONS_FILE),
            MAX_INSTRUCTIONS_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(regular_file::RegularFileError::Missing) => continue,
            Err(_) => {
                found.skipped += 1;
                continue;
            }
        };
        let Ok(instructions) = String::from_utf8(instructions) else {
            found.skipped += 1;
            continue;
        };
        if path.to_str().is_none() {
            found.skipped += 1;
            continue;
        }
        let location = match plugin {
            Some(key) => Location::Plugin {
                key: key.to_owned(),
                skill: text.to_owned(),
            },
            None => Location::Folder(path.clone()),
        };
        let applies = rubric_store::applicable(&path, &location, store);
        let rubric = match GradingSkill::open_with(&path, Some(&applies.path)) {
            Ok(skill) => RubricStatus::Ready(skill.rubric().clone()),
            Err(SkillError::Rubric(RubricError::Missing)) => RubricStatus::Missing,
            Err(SkillError::Rubric(problem)) => RubricStatus::Problem(problem),
            Err(_) => {
                found.skipped += 1;
                continue;
            }
        };
        if found.skills.len() == MAX_SKILLS {
            found.incomplete = true;
            return;
        }
        let (name, description) = front_matter(&instructions);
        found.skills.push(InstalledSkill {
            folder: path,
            location,
            source: source.clone(),
            name: name.unwrap_or_else(|| clip(text, MAX_NAME_CHARS)),
            description: description.unwrap_or_default(),
            rubric_kept_by_xtrace: applies.kept_by_xtrace
                && !matches!(rubric, RubricStatus::Missing),
            rubric,
        });
    }
}

#[derive(Deserialize)]
struct InstalledPlugins {
    plugins: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct PluginInstall {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default, rename = "installPath")]
    install_path: Option<String>,
}

/// Claude Code's user settings, of which only `enabledPlugins` is kept.
#[derive(Deserialize)]
struct EnabledPlugins {
    #[serde(default, rename = "enabledPlugins")]
    enabled_plugins: BTreeMap<String, serde_json::Value>,
}

/// A JSON file of the plugin list: none when it is missing.
fn read_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    file: PluginFile,
) -> Result<Option<T>, PluginProblem> {
    let problem = |failure| PluginProblem { file, failure };
    let bytes = match regular_file::read_capped(path, MAX_JSON_BYTES) {
        Ok(bytes) => bytes,
        Err(regular_file::RegularFileError::Missing) => return Ok(None),
        Err(regular_file::RegularFileError::NotRegular)
            if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) =>
        {
            return Err(problem(PluginReadFailure::Link));
        }
        Err(_) => return Err(problem(PluginReadFailure::Unreadable)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        problem(match error.classify() {
            serde_json::error::Category::Data => PluginReadFailure::UnknownShape,
            _ => PluginReadFailure::Malformed,
        })
    })
}

/// The plugin installs whose skills are listed, and what reading the list
/// met.
#[derive(Default)]
struct PluginInstalls {
    /// Each plugin's key and its resolved install folder, in the installed
    /// list's key order.
    installs: Vec<(String, PathBuf)>,
    /// Installs whose folder is not inside the plugins folder.
    skipped: u32,
    /// More installs than [`MAX_PLUGINS`].
    incomplete: bool,
    /// The first problem reading the list.
    problem: Option<PluginProblem>,
}

/// The enabled, user-scoped plugin installs: for each plugin key the
/// settings turn on, its first user-scoped install, whose folder must
/// resolve inside the plugins folder (itself a real folder). A missing
/// plugins folder, list or settings file lists none and is no problem; a
/// list or settings file that is a link, unreadable, not JSON or of another
/// shape lists none and says so, and so does a plugins folder that is not a
/// real folder. One plugin's installs of another shape skip that plugin and
/// say so.
fn plugin_installs(home: &Path) -> PluginInstalls {
    let mut out = PluginInstalls::default();
    let folder_problem = |failure| PluginProblem {
        file: PluginFile::PluginsFolder,
        failure,
    };
    let plugins = home.join(CLAUDE_PLUGINS);
    match std::fs::symlink_metadata(&plugins) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return out,
        Err(_) => {
            out.problem = Some(folder_problem(PluginReadFailure::Unreadable));
            return out;
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            out.problem = Some(folder_problem(PluginReadFailure::Link));
            return out;
        }
        Ok(meta) if !meta.is_dir() => {
            out.problem = Some(folder_problem(PluginReadFailure::UnknownShape));
            return out;
        }
        Ok(_) => {}
    }
    let Ok(plugins) = std::fs::canonicalize(&plugins) else {
        out.problem = Some(folder_problem(PluginReadFailure::Unreadable));
        return out;
    };
    let installed = match read_json::<InstalledPlugins>(
        &plugins.join(INSTALLED_PLUGINS),
        PluginFile::InstalledPlugins,
    ) {
        Ok(Some(installed)) => installed,
        Ok(None) => return out,
        Err(problem) => {
            out.problem = Some(problem);
            return out;
        }
    };
    let enabled =
        match read_json::<EnabledPlugins>(&home.join(CLAUDE_SETTINGS), PluginFile::Settings) {
            Ok(settings) => settings
                .map(|settings| settings.enabled_plugins)
                .unwrap_or_default(),
            Err(problem) => {
                out.problem = Some(problem);
                return out;
            }
        };
    let mut looked = 0_usize;
    for (key, installs) in installed.plugins {
        if enabled.get(&key) != Some(&serde_json::Value::Bool(true)) {
            continue;
        }
        let Ok(installs) = serde_json::from_value::<Vec<PluginInstall>>(installs) else {
            out.problem.get_or_insert(PluginProblem {
                file: PluginFile::InstalledPlugins,
                failure: PluginReadFailure::UnknownShape,
            });
            continue;
        };
        let Some(install) = installs
            .into_iter()
            .find(|install| install.scope.as_deref() == Some(USER_SCOPE))
        else {
            continue;
        };
        if looked == MAX_PLUGINS {
            out.incomplete = true;
            return out;
        }
        looked += 1;
        let Some(path) = install.install_path.map(PathBuf::from) else {
            out.problem.get_or_insert(PluginProblem {
                file: PluginFile::InstalledPlugins,
                failure: PluginReadFailure::UnknownShape,
            });
            continue;
        };
        let resolved = path
            .is_absolute()
            .then(|| std::fs::canonicalize(&path).ok())
            .flatten()
            .filter(|resolved| resolved.starts_with(&plugins) && *resolved != plugins);
        match resolved {
            Some(resolved) => out.installs.push((key, resolved)),
            None => out.skipped += 1,
        }
    }
    out
}

/// `SKILL.md`'s `name` and the first line of its `description`, from the
/// front matter between its first two `---` lines; `None` for each that is
/// missing or empty, and for both when there is no front matter. Only a key
/// at the start of a line counts. A value is the text after the colon,
/// without surrounding quotes and a trailing ` #` comment; a block (`>` or
/// `|`) or a value on the following indented lines gives that block's first
/// line. Kept to one line of at most [`MAX_NAME_CHARS`] or
/// [`MAX_DESCRIPTION_CHARS`] characters.
pub fn front_matter(text: &str) -> (Option<String>, Option<String>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .take(MAX_FRONT_MATTER_LINES + 1)
        .collect();
    if lines.first().map(|line| line.trim_end()) != Some("---") {
        return (None, None);
    }
    let Some(end) = lines
        .iter()
        .skip(1)
        .position(|line| matches!(line.trim_end(), "---" | "..."))
        .map(|at| at + 1)
    else {
        return (None, None);
    };
    let header = &lines[1..end];
    let value = |key: &str, cap: usize| -> Option<String> {
        let at = header.iter().position(|line| {
            line.split_once(':')
                .is_some_and(|(name, _)| name == key && !line.starts_with([' ', '\t']))
        })?;
        let (_, rest) = header[at].split_once(':')?;
        let rest = rest.trim();
        let block = rest.is_empty()
            || (rest.starts_with(['>', '|'])
                && rest[1..]
                    .chars()
                    .all(|c| c == '-' || c == '+' || c.is_ascii_digit()));
        let line = if block {
            header[at + 1..]
                .iter()
                .take_while(|line| line.trim().is_empty() || line.starts_with([' ', '\t']))
                .map(|line| line.trim())
                .find(|line| !line.is_empty())?
                .to_owned()
        } else {
            scalar(rest)
        };
        let line = clip(line.trim(), cap);
        (!line.is_empty()).then_some(line)
    };
    (
        value("name", MAX_NAME_CHARS),
        value("description", MAX_DESCRIPTION_CHARS),
    )
}

/// A one-line YAML scalar: a quoted one without its quotes, a plain one
/// without a trailing comment.
fn scalar(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n' | 't') => out.push(' '),
                    Some(other) => out.push(other),
                    None => break,
                },
                other => out.push(other),
            }
        }
        out
    } else if let Some(inner) = value.strip_prefix('\'') {
        inner
            .split("''")
            .map(|part| part.split('\'').next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("'")
    } else {
        value
            .split_once(" #")
            .map_or(value, |(plain, _)| plain)
            .to_owned()
    }
}

/// At most `cap` characters, with every control character a space.
fn clip(text: &str, cap: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(cap)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_reads_plain_quoted_and_block_values() {
        assert_eq!(
            front_matter("---\nname: doc\ndescription: Checks things. # note\n---\nbody"),
            (Some("doc".into()), Some("Checks things.".into()))
        );
        assert_eq!(
            front_matter("---\nname: \"quoted \\\"one\\\"\"\ndescription: 'it''s fine'\n---\n"),
            (Some("quoted \"one\"".into()), Some("it's fine".into()))
        );
        assert_eq!(
            front_matter(
                "---\nname: b\ndescription: >-\n  First line\n  second line\nother: x\n---\n"
            ),
            (Some("b".into()), Some("First line".into()))
        );
        assert_eq!(
            front_matter("\u{feff}---\r\ndescription: |\r\n\r\n   Kept\r\nname: c\r\n---\r\n"),
            (Some("c".into()), Some("Kept".into()))
        );
        // An indented key is part of another value, not a key.
        assert_eq!(
            front_matter("---\nmeta:\n  name: inner\n---\n"),
            (None, None)
        );
    }

    #[test]
    fn no_or_unclosed_front_matter_gives_nothing() {
        assert_eq!(front_matter("# Title\nname: x\n"), (None, None));
        assert_eq!(front_matter("---\nname: x\n"), (None, None));
        assert_eq!(front_matter("---\nname:\n---\n"), (None, None));
        assert_eq!(front_matter(""), (None, None));
    }

    #[test]
    fn long_values_are_clipped_to_one_line() {
        let long = "x".repeat(MAX_DESCRIPTION_CHARS + 50);
        let (_, description) = front_matter(&format!("---\ndescription: {long}\tend\n---\n"));
        assert_eq!(description.unwrap().chars().count(), MAX_DESCRIPTION_CHARS);
        let (name, _) = front_matter("---\nname: a\u{7}b\n---\n");
        assert_eq!(name.as_deref(), Some("a b"));
    }
}
