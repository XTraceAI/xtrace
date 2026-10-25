//! Opens the user's terminal running their own Claude Code's `auth login`,
//! only when they click "sign in". XTrace writes a two-line `.command`
//! script naming the same Claude Code the usage probe runs, then asks macOS
//! to open it, which needs no Automation permission. Nothing from the screen
//! reaches the script, and XTrace never sees the sign-in or a credential.
use crate::account_usage_claude_local::ClaudeUsagePaths;
use crate::account_usage_process::run_to_exit;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// The folder under the app's data folder that holds the script.
pub(super) const SIGN_IN_DIR_NAME: &str = "xtrace-claude-sign-in";
/// Terminal shows this name in its window title.
const SCRIPT_NAME: &str = "claude-sign-in.command";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SignInError {
    /// No Claude Code was found, or its path cannot be written into a script.
    NotFound,
    /// The script could not be written or the terminal did not open it.
    CouldNotOpen,
    /// Not on macOS, or this app reads no Claude usage (fixture mode).
    Unsupported,
}

impl SignInError {
    /// Short enough for one line in the sidebar.
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::NotFound => "Claude Code not found",
            Self::CouldNotOpen => "Terminal did not open",
            Self::Unsupported => "Sign-in unavailable here",
        }
    }
}

/// Writes the script and opens it in the user's terminal.
pub(super) fn open_sign_in(paths: &ClaudeUsagePaths) -> Result<(), SignInError> {
    if !cfg!(target_os = "macos") {
        return Err(SignInError::Unsupported);
    }
    sign_in_with(
        &paths.sign_in_dir,
        super::account_usage_claude_probe::resolve_claude(),
        open_in_terminal,
    )
}

fn sign_in_with(
    dir: &Path,
    binary: Option<PathBuf>,
    open: impl FnOnce(&Path) -> bool,
) -> Result<(), SignInError> {
    let binary = binary.ok_or(SignInError::NotFound)?;
    let text = script_text(&binary).ok_or(SignInError::NotFound)?;
    let script = write_script(dir, &text).map_err(|_| SignInError::CouldNotOpen)?;
    if open(&script) {
        Ok(())
    } else {
        Err(SignInError::CouldNotOpen)
    }
}

/// The whole script: the resolved Claude Code, quoted for the shell, with
/// `auth login` and nothing else. A path that is not UTF-8 is refused.
fn script_text(binary: &Path) -> Option<String> {
    let binary = binary.to_str()?;
    Some(format!(
        "#!/bin/sh\n# Written by XTrace when you chose to sign in to Claude Code.\nexec {} auth login\n",
        shell_quote(binary)
    ))
}

/// One single-quoted shell word: inside single quotes only `'` itself is
/// special, so it closes the quote, adds an escaped quote and reopens it.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Replaces the script with a fresh owner-only executable. A file or link
/// left at the script's name is removed first, and the new file is created
/// only if nothing took its place, so the write never follows a link.
fn write_script(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(SCRIPT_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        options.mode(0o700);
    }
    let mut file = options.open(&path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

/// `open` hands a `.command` file to the user's default terminal and exits.
fn open_in_terminal(script: &Path) -> bool {
    let mut command = Command::new("/usr/bin/open");
    command.arg(script);
    run_to_exit(&mut command, b"", Duration::from_secs(10), 4096, false).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_runs_only_the_quoted_binary_with_auth_login() {
        assert_eq!(
            script_text(Path::new("/Users/a b/.local/bin/claude")).unwrap(),
            "#!/bin/sh\n# Written by XTrace when you chose to sign in to Claude Code.\n\
             exec '/Users/a b/.local/bin/claude' auth login\n"
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let not_utf8 = std::ffi::OsStr::from_bytes(b"/tmp/\xff/claude");
            assert_eq!(script_text(Path::new(not_utf8)), None);
        }
    }

    /// The shell reads every awkward path back as the same single word.
    #[cfg(unix)]
    #[test]
    fn quoting_survives_the_shell() {
        for path in [
            "/a b/claude",
            "/it's/claude",
            "/$(touch x)/`id`/claude",
            "/new\nline/claude",
            "/semi;colon/\"q\"/\\/claude",
        ] {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("printf %s {}", shell_quote(path)))
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(output.stdout).unwrap(), path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_script_is_replaced_owner_only_and_opened() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(SIGN_IN_DIR_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        // A link left at the script's name is replaced, never written through.
        let outside = root.path().join("outside");
        std::fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join(SCRIPT_NAME)).unwrap();
        let mut opened = None;
        let result = sign_in_with(&dir, Some(PathBuf::from("/opt/claude")), |script| {
            opened = Some(script.to_path_buf());
            true
        });
        assert_eq!(result, Ok(()));
        let script = opened.unwrap();
        assert_eq!(script, dir.join(SCRIPT_NAME));
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
        assert!(!std::fs::symlink_metadata(&script).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(&script).unwrap(),
            script_text(Path::new("/opt/claude")).unwrap()
        );
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&script), 0o700);
        assert_eq!(mode(&dir), 0o700);
        // A second click rewrites it.
        sign_in_with(&dir, Some(PathBuf::from("/other/claude")), |_| true).unwrap();
        assert!(
            std::fs::read_to_string(&script)
                .unwrap()
                .contains("exec '/other/claude' auth login")
        );
    }

    #[test]
    fn failures_say_why_without_opening_anything() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(SIGN_IN_DIR_NAME);
        assert_eq!(
            sign_in_with(&dir, None, |_| panic!("nothing to open")),
            Err(SignInError::NotFound)
        );
        assert!(!dir.exists());
        assert_eq!(
            sign_in_with(&dir, Some(PathBuf::from("/opt/claude")), |_| false),
            Err(SignInError::CouldNotOpen)
        );
        // A folder that cannot be made: a file stands where it would go.
        let blocked = root.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        assert_eq!(
            sign_in_with(
                &blocked.join("dir"),
                Some(PathBuf::from("/opt/claude")),
                |_| panic!("nothing to open")
            ),
            Err(SignInError::CouldNotOpen)
        );
        for error in [
            SignInError::NotFound,
            SignInError::CouldNotOpen,
            SignInError::Unsupported,
        ] {
            assert!(error.message().len() <= 24, "{error:?}");
        }
    }
}
