//! Finding the command-line executables XTrace runs (`gh`, `claude`, `codex`).
//!
//! This is the one resolver. A probe or judge that runs a program takes an
//! already resolved absolute path; this module names it, from inputs the
//! caller supplies:
//!
//! * an explicit override (a backend-only environment variable such as
//!   `XTRACE_GH` or [`CLAUDE_ENV`]), which must be an absolute path to an
//!   executable file;
//! * otherwise the first match on the absolute entries of a `PATH` value (a
//!   relative entry would resolve against this process's working directory,
//!   which is not a place an executable may come from);
//! * otherwise the first match in the caller's known install directories.
//!
//! Codex checks its fixed ChatGPT bundle before `PATH`, after any override.
//!
//! Nothing else is consulted. It never runs a login shell, never starts a
//! process, never reads a configuration file or credential. A candidate is
//! accepted only as an absolute path to a regular file with an execute bit;
//! metadata follows symbolic links, so the usual package-manager symlink is
//! accepted as the absolute path it is spelled with. Every input is supplied
//! rather than read here, so a test resolves against its own directories
//! without touching the process environment or this host's installation.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// The Claude Code executable's name.
pub const CLAUDE: &str = "claude";
/// The backend-only override naming the Claude Code executable.
pub const CLAUDE_ENV: &str = "XTRACE_CLAUDE";

/// Directories added after the inherited `PATH` for a Claude Code child. A
/// desktop app launched from Finder inherits a short `PATH`; Claude Code may
/// need node or its own helpers beside it.
pub const CLAUDE_CHILD_PATH_EXTRAS: &[&str] =
    &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

/// Why no executable could be named. Holds no path; callers word their own
/// messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// The override is not an absolute path.
    OverrideNotAbsolute,
    /// The override does not name an executable file.
    OverrideNotExecutable,
    /// Not on `PATH` and not in a known directory.
    NotFound,
    /// Not a Unix host.
    UnsupportedPlatform,
}

/// Resolve `name` from an explicit override, a `PATH` value and known
/// directories, in that order.
pub fn resolve_within(
    name: &str,
    explicit: Option<&OsStr>,
    search: Option<&OsStr>,
    known: &[&Path],
) -> Result<PathBuf, Unavailable> {
    if !cfg!(unix) {
        return Err(Unavailable::UnsupportedPlatform);
    }
    if let Some(explicit) = explicit {
        let path = Path::new(explicit);
        if !path.is_absolute() {
            return Err(Unavailable::OverrideNotAbsolute);
        }
        return is_executable(path)
            .then(|| path.to_path_buf())
            .ok_or(Unavailable::OverrideNotExecutable);
    }
    if let Some(search) = search {
        let found = std::env::split_paths(search)
            .filter(|entry| entry.is_absolute())
            .map(|entry| entry.join(name))
            .find(|candidate| is_executable(candidate));
        if let Some(found) = found {
            return Ok(found);
        }
    }
    known
        .iter()
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
        .ok_or(Unavailable::NotFound)
}

/// Where Claude Code is installed when it is not on `PATH`: its own installer
/// directories under the home folder, then Homebrew's.
pub fn claude_known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// Resolve the Claude Code executable: the override, then `PATH`, then
/// [`claude_known_dirs`].
pub fn resolve_claude(
    explicit: Option<&OsStr>,
    search: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<PathBuf, Unavailable> {
    let known = claude_known_dirs(home);
    let known: Vec<&Path> = known.iter().map(PathBuf::as_path).collect();
    resolve_within(CLAUDE, explicit, search, &known)
}

/// The Codex executable's name.
pub const CODEX: &str = "codex";
/// The Codex command-line tool bundled with the ChatGPT desktop app.
pub const CHATGPT_BUNDLED_CODEX: &str =
    "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";

/// Resolve the Codex executable: the override; then the one bundled with
/// the ChatGPT app; then `PATH`, `~/.local/bin` and Homebrew's folders.
pub fn resolve_codex(
    explicit: Option<&OsStr>,
    search: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<PathBuf, Unavailable> {
    let mut known = Vec::new();
    if let Some(home) = home {
        known.push(home.join(".local/bin"));
    }
    known.push(PathBuf::from("/opt/homebrew/bin"));
    known.push(PathBuf::from("/usr/local/bin"));
    let known: Vec<&Path> = known.iter().map(PathBuf::as_path).collect();
    resolve_codex_within(explicit, search, Path::new(CHATGPT_BUNDLED_CODEX), &known)
}

fn resolve_codex_within(
    explicit: Option<&OsStr>,
    search: Option<&OsStr>,
    bundled: &Path,
    known: &[&Path],
) -> Result<PathBuf, Unavailable> {
    if explicit.is_none() && is_executable(bundled) {
        return Ok(bundled.to_path_buf());
    }
    resolve_within(CODEX, explicit, search, known)
}

/// The `PATH` entries for a Claude Code child: the executable's own
/// directory, then the inherited entries, then any of
/// [`CLAUDE_CHILD_PATH_EXTRAS`] not already listed.
pub fn claude_child_path_entries(binary: &Path, inherited: Option<&OsStr>) -> Vec<PathBuf> {
    let mut path: Vec<PathBuf> = Vec::new();
    if let Some(parent) = binary.parent() {
        path.push(parent.to_path_buf());
    }
    if let Some(current) = inherited {
        path.extend(std::env::split_paths(current));
    }
    for extra in CLAUDE_CHILD_PATH_EXTRAS {
        let extra = PathBuf::from(extra);
        if !path.contains(&extra) {
            path.push(extra);
        }
    }
    path
}

/// [`claude_child_path_entries`] joined into one `PATH` value, or nothing
/// when an entry cannot be joined.
pub fn join_path(entries: &[PathBuf]) -> Option<OsString> {
    std::env::join_paths(entries).ok()
}

/// An absolute path to a regular file with an execute bit.
#[cfg(unix)]
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
pub fn is_executable(_: &Path) -> bool {
    false
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::Builder::new()
            .tempdir_in(env!("CARGO_MANIFEST_DIR").to_owned() + "/../../target")
            .unwrap()
    }

    #[test]
    fn override_then_path_then_known_dirs() {
        let root = scratch();
        let (named, on_path, known) = (
            root.path().join("named/claude"),
            root.path().join("bin"),
            root.path().join("known"),
        );
        executable_file(&named);
        executable_file(&on_path.join(CLAUDE));
        executable_file(&known.join(CLAUDE));
        let known_dirs = [known.as_path()];
        assert_eq!(
            resolve_within(
                CLAUDE,
                Some(named.as_os_str()),
                Some(on_path.as_os_str()),
                &known_dirs
            ),
            Ok(named.clone())
        );
        let search = std::env::join_paths(["relative", on_path.to_str().unwrap()]).unwrap();
        assert_eq!(
            resolve_within(CLAUDE, None, Some(&search), &known_dirs),
            Ok(on_path.join(CLAUDE))
        );
        assert_eq!(
            resolve_within(CLAUDE, None, Some(OsStr::new("relative")), &known_dirs),
            Ok(known.join(CLAUDE))
        );
        assert_eq!(
            resolve_within(CLAUDE, None, None, &[]),
            Err(Unavailable::NotFound)
        );
        assert_eq!(
            resolve_within(CLAUDE, Some(OsStr::new("claude")), None, &known_dirs),
            Err(Unavailable::OverrideNotAbsolute)
        );
        assert_eq!(
            resolve_within(CLAUDE, Some(root.path().as_os_str()), None, &known_dirs),
            Err(Unavailable::OverrideNotExecutable)
        );
    }

    #[test]
    fn codex_prefers_override_then_bundle_path_home_and_homebrew() {
        let root = scratch();
        let bundled = root.path().join("ChatGPT/codex");
        let named = root.path().join("named/codex");
        let missing = root.path().join("missing");
        let on_path = root.path().join("path");
        let home = root.path().join("home/.local/bin");
        let brew = root.path().join("homebrew/bin");
        let local = root.path().join("local/bin");
        let candidates = [
            bundled.clone(),
            named.clone(),
            on_path.join(CODEX),
            home.join(CODEX),
            brew.join(CODEX),
            local.join(CODEX),
        ];
        for candidate in &candidates {
            executable_file(candidate);
        }
        let search = std::env::join_paths([Path::new("relative"), &on_path]).unwrap();
        let known = [home.as_path(), brew.as_path(), local.as_path()];
        let select = |explicit| resolve_codex_within(explicit, Some(&search), &bundled, &known);
        assert_eq!(select(None), Ok(bundled.clone()));
        assert_eq!(select(Some(named.as_os_str())), Ok(named.clone()));
        assert_eq!(
            select(Some(OsStr::new(""))),
            Err(Unavailable::OverrideNotAbsolute)
        );
        assert_eq!(
            select(Some(OsStr::new("relative/codex"))),
            Err(Unavailable::OverrideNotAbsolute)
        );
        assert_eq!(
            select(Some(missing.as_os_str())),
            Err(Unavailable::OverrideNotExecutable)
        );
        std::fs::set_permissions(&named, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            select(Some(named.as_os_str())),
            Err(Unavailable::OverrideNotExecutable)
        );
        std::fs::remove_file(&bundled).unwrap();
        std::fs::create_dir(&bundled).unwrap();
        assert_eq!(select(None), Ok(on_path.join(CODEX)));
        for (current, next) in [(&on_path, &home), (&home, &brew), (&brew, &local)] {
            std::fs::remove_file(current.join(CODEX)).unwrap();
            assert_eq!(select(None), Ok(next.join(CODEX)));
        }
        std::fs::remove_file(local.join(CODEX)).unwrap();
        assert_eq!(select(None), Err(Unavailable::NotFound));
    }

    #[test]
    fn codex_ignores_relative_and_non_executable_candidates() {
        let root = scratch();
        let bundled = root.path().join("bundle/codex");
        let known = root.path().join("known");
        executable_file(&bundled);
        executable_file(&known.join(CODEX));
        for candidate in [&bundled, &known.join(CODEX)] {
            std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert_eq!(
            resolve_codex_within(
                None,
                Some(OsStr::new("relative")),
                &bundled,
                &[Path::new("relative/home/.local/bin"), &known],
            ),
            Err(Unavailable::NotFound)
        );
        assert_eq!(
            resolve_codex_within(Some(bundled.as_os_str()), None, &bundled, &[]),
            Err(Unavailable::OverrideNotExecutable)
        );
    }

    #[test]
    fn codex_keeps_the_selected_executable_symlink_spelling() {
        let root = scratch();
        let target = root.path().join("installed/codex");
        executable_file(&target);
        let alias_dir = root.path().join("linked");
        std::fs::create_dir(&alias_dir).unwrap();
        let alias = alias_dir.join(CODEX);
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert_eq!(
            resolve_codex_within(
                None,
                Some(alias_dir.as_os_str()),
                &root.path().join("missing-bundle"),
                &[],
            ),
            Ok(alias)
        );
    }

    #[test]
    fn claude_looks_in_its_installer_dirs_under_home() {
        let root = scratch();
        let home = root.path().join("home");
        let installed = home.join(".claude/local").join(CLAUDE);
        executable_file(&installed);
        assert_eq!(
            claude_known_dirs(Some(&home))[..2],
            [home.join(".local/bin"), home.join(".claude/local")]
        );
        // Only the override, PATH and the known dirs are consulted; an empty
        // PATH leaves the installer folder (Homebrew may or may not exist on
        // the test host, but the home folder comes first).
        assert_eq!(
            resolve_claude(None, Some(OsStr::new("")), Some(&home)),
            Ok(installed)
        );
    }

    #[test]
    fn child_path_puts_the_executable_first_and_adds_missing_extras() {
        let entries = claude_child_path_entries(
            Path::new("/opt/tools/claude"),
            Some(OsStr::new("/usr/bin:/custom")),
        );
        let expected: Vec<PathBuf> = [
            "/opt/tools",
            "/usr/bin",
            "/custom",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/bin",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(entries, expected);
    }
}
