//! Tool discovery, fixed command plans and sign-in rules for diagnosis.
//!
//! The default tool order is Claude Code, then Codex ([`ORDER`]). Cursor
//! requires an explicit choice because its prompt goes on the command
//! line and its run has not been verified end to end ([`Tool::limits`]).
//! This module builds plans and reads supplied output; it starts no child
//! process. A caller must check versions, sign-in and Codex's shell feature
//! before using a plan.

use super::answer::{self, Answer, AnswerError};
use crate::runner;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};
use xt_probes::executable::Unavailable;

/// The tools a run uses, in the order they are tried. Cursor is left out on
/// purpose: its prompt is passed on the command line and its run is unverified.
pub const ORDER: [Tool; 2] = [Tool::ClaudeCode, Tool::Codex];
/// How many tries one tool gets: the first, and one with the reason the
/// first answer was refused.
pub const TRIES: usize = 2;
/// The Claude Code model alias a run asks for.
pub const CLAUDE_MODEL: &str = "haiku";
/// Codex features turned off for a run, of those the installed Codex lists.
pub const CODEX_DISABLED: [&str; 14] = [
    "shell_tool",
    "unified_exec",
    "apps",
    "plugins",
    "hooks",
    "multi_agent",
    "browser_use",
    "computer_use",
    "image_generation",
    "view_image",
    "memories",
    "skill_search",
    "in_app_browser",
    "goals",
];
/// The Codex override naming its executable.
pub const CODEX_ENV: &str = "XTRACE_CODEX";
/// The Cursor override naming its agent executable.
pub const CURSOR_ENV: &str = "XTRACE_CURSOR_AGENT";
/// The folder under the grading staging parent that diagnosis runs use.
pub const DIAGNOSE_DIR: &str = "diagnose";
/// How long one try may take.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(150);
/// How long a check before a run (`--version`, a model or feature list)
/// may take.
pub const DEFAULT_CHECK_DEADLINE: Duration = Duration::from_secs(30);
/// Retained output of a try; more fails the try as too large.
pub const DEFAULT_MAX_STDOUT_BYTES: usize = 256 * 1024;
/// How long a sign-in check may take, at most.
pub const SIGN_IN_CHECK_DEADLINE: Duration = Duration::from_secs(10);
/// Inherited variables kept for a Codex child beside the runner's list.
const CODEX_KEPT_ENV: [&str; 1] = ["CODEX_HOME"];
/// Words in a failed tool's own error lines that mean it is not signed in,
/// each matched as whole words.
const SIGN_IN_WORDS: [&str; 11] = [
    "not logged in",
    "log in",
    "login",
    "sign in",
    "not signed in",
    "authenticate",
    "authentication",
    "unauthenticated",
    "oauth",
    "unauthorized",
    "401",
];

/// A coding tool that can run a diagnosis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    ClaudeCode,
    Codex,
    Cursor,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }

    /// What is known to reach the model beyond the prompt, or not to have
    /// been seen working, with this tool.
    pub fn limits(self) -> &'static [Limit] {
        match self {
            Self::ClaudeCode => &[],
            Self::Codex => &[Limit::CodexLoadsUserInstructions],
            Self::Cursor => &[Limit::CursorPromptOnCommandLine, Limit::CursorUnverified],
        }
    }
}

/// A known limit of a tool's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Codex still loads `~/.codex/AGENTS.md` and its installed skill list
    /// despite `--ignore-user-config`.
    CodexLoadsUserInstructions,
    /// Cursor's agent takes the prompt on its command line.
    CursorPromptOnCommandLine,
    /// A Cursor run was not seen working end to end.
    CursorUnverified,
}

/// Why a tool was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    NotInstalled,
    /// Claude Code older than the grading runner's minimum, or no readable
    /// version.
    TooOld,
    NotSignedIn,
    CouldNotStart,
    /// A check before its run exited with a failure.
    Failed {
        code: Option<i32>,
    },
    /// A check before its run took longer than its deadline.
    TimedOut,
    /// Codex: its features list could not be read or does not name
    /// `shell_tool`, so its shell could not surely be turned off.
    ShellNotOff,
}

/// One tool that was looked at and not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tried {
    pub tool: Tool,
    pub skip: Skip,
    /// Whether it was given the prompt (and so the messages). Always false:
    /// a tool given the prompt is never skipped, its failure ends the run.
    /// Kept so a screen can say so plainly.
    pub given_prompt: bool,
}

/// How a try that had started failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RunFailure {
    #[error("the run took longer than its deadline")]
    Timeout,
    #[error("the run's output was larger than XTrace reads")]
    OutputTooLarge,
    #[error("the run could not be observed")]
    Unobservable,
    #[error("the tool exited with a failure")]
    Exited { code: Option<i32> },
    #[error("the tool is not signed in")]
    NotSignedIn,
    #[error("the tool's answer could not be read")]
    Unreadable,
}

/// Why a run gave no diagnosis.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiagnoseError {
    #[error("no coding tool that can run a diagnosis is available")]
    NoTool { tried: Vec<Tried> },
    #[error("the answer broke the diagnosis rules twice: {error}")]
    Refused {
        tool: Tool,
        model: Model,
        error: AnswerError,
        tried: Vec<Tried>,
    },
    /// The tool given the messages failed; no other tool was asked.
    #[error("{failure}")]
    Run {
        tool: Tool,
        model: Model,
        failure: RunFailure,
        tried: Vec<Tried>,
    },
    #[error("the diagnosis was cancelled")]
    Cancelled,
    #[error("the diagnosis run's private folder could not be prepared")]
    Staging,
    #[error("diagnosis needs a Unix host")]
    UnsupportedPlatform,
}

/// Which model a run asked for, and which answered when the tool says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    /// As the run named it: `haiku`, a Codex slug or `default`, a Cursor
    /// name or `auto`.
    pub asked: String,
    /// The model that answered, as the tool names it, only when the tool's
    /// answer says so unambiguously; never guessed.
    pub answered: Option<String>,
}

/// An accepted diagnosis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub tool: Tool,
    pub model: Model,
    pub answer: Answer,
    /// 1 or 2.
    pub tries: usize,
    /// Tools tried before this one, and why they were not used.
    pub tried: Vec<Tried>,
}

/// Where each tool is installed, or why it is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools {
    pub claude: Result<PathBuf, Unavailable>,
    pub codex: Result<PathBuf, Unavailable>,
    pub cursor: Result<PathBuf, Unavailable>,
}

impl Tools {
    pub fn get(&self, tool: Tool) -> &Result<PathBuf, Unavailable> {
        match tool {
            Tool::ClaudeCode => &self.claude,
            Tool::Codex => &self.codex,
            Tool::Cursor => &self.cursor,
        }
    }
}

/// Find the three tools from an environment: Claude Code as grading finds
/// it ([`runner::find_claude`]); Codex as the app's account reading finds it
/// ([`xt_probes::executable::resolve_codex`]) with the [`CODEX_ENV`]
/// override; Cursor's `cursor-agent` from [`CURSOR_ENV`], `PATH`,
/// `~/.local/bin` and Homebrew's folders.
pub fn find_tools(env: &[(OsString, OsString)]) -> Tools {
    let home = runner::inherited_var(env, "HOME").map(Path::new);
    let path = runner::inherited_var(env, "PATH");
    let mut cursor_dirs: Vec<PathBuf> = home
        .map(|home| vec![home.join(".local/bin")])
        .unwrap_or_default();
    cursor_dirs.push(PathBuf::from("/opt/homebrew/bin"));
    cursor_dirs.push(PathBuf::from("/usr/local/bin"));
    let cursor_dirs: Vec<&Path> = cursor_dirs.iter().map(PathBuf::as_path).collect();
    Tools {
        claude: runner::find_claude(env),
        codex: xt_probes::executable::resolve_codex(
            runner::inherited_var(env, CODEX_ENV),
            path,
            home,
        ),
        cursor: xt_probes::executable::resolve_within(
            "cursor-agent",
            runner::inherited_var(env, CURSOR_ENV),
            path,
            &cursor_dirs,
        ),
    }
}

/// The bounds of one diagnosis. Tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentLimits {
    pub deadline: Duration,
    pub check_deadline: Duration,
    pub max_stdout_bytes: usize,
    pub exit_grace: Duration,
    pub term_grace: Duration,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            check_deadline: DEFAULT_CHECK_DEADLINE,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            exit_grace: runner::DEFAULT_EXIT_GRACE,
            term_grace: runner::DEFAULT_TERM_GRACE,
        }
    }
}

/// One diagnosis's inputs.
#[derive(Clone, Debug)]
pub struct DiagnoseJob<'a> {
    pub tools: &'a Tools,
    /// The skill's instructions ([`super::instructions`]).
    pub instructions: &'a str,
    /// The built input ([`super::input::build`]).
    pub input: &'a str,
    /// The system temporary folder.
    pub temp_dir: &'a Path,
    /// A run ID in the grading runs' shape ([`crate::staging::is_run_id`]).
    pub run_id: &'a str,
    /// The environment this process received; only [`agent_env`] reaches a
    /// child.
    pub inherited_env: &'a [(OsString, OsString)],
    pub limits: AgentLimits,
}

/// The prompt of one try.
pub fn prompt(instructions: &str, input: &str, previous: Option<&AnswerError>) -> String {
    let note = previous.map(answer::retry_note).unwrap_or_default();
    format!(
        "{}\n{note}\n===== The input =====\n\n{input}",
        instructions.trim_end()
    )
}

/// What one tool's run needs, decided before its first try.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The model the run names, as the tool names it.
    pub model: String,
    /// The arguments after the executable, without the prompt for a tool
    /// that takes it on its command line.
    pub args: Vec<OsString>,
    /// Whether the prompt goes on standard input; otherwise it is the last
    /// argument.
    pub prompt_on_stdin: bool,
}

/// The Claude Code arguments.
pub fn claude_plan() -> Plan {
    let args = [
        "-p",
        "--model",
        CLAUDE_MODEL,
        "--tools",
        "",
        "--setting-sources",
        runner::SETTING_SOURCES,
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--strict-mcp-config",
        "--no-session-persistence",
        "--output-format",
        "json",
    ];
    Plan {
        model: CLAUDE_MODEL.to_owned(),
        args: args.into_iter().map(OsString::from).collect(),
        prompt_on_stdin: true,
    }
}

/// A Codex model chosen from its catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexModel {
    pub slug: String,
    /// Whether it supports the `low` reasoning effort.
    pub low_effort: bool,
}

/// Whether a Codex with these listed features runs with its shell surely
/// off: the list names `shell_tool`, which [`codex_plan`] then turns off
/// (with `unified_exec` when listed). An unread or empty list, or one
/// without `shell_tool`, is not enough: Codex is not run.
pub fn codex_shell_off(available: &[String]) -> bool {
    available.iter().any(|name| name == "shell_tool")
}

/// The Codex arguments: `chosen` as `-m` when there is one, and every
/// feature of [`CODEX_DISABLED`] that `available` lists turned off. Only
/// used when [`codex_shell_off`] holds.
pub fn codex_plan(chosen: Option<&CodexModel>, available: &[String]) -> Plan {
    let mut args: Vec<OsString> = vec!["exec".into()];
    if let Some(model) = chosen {
        args.extend(["-m".into(), model.slug.clone().into()]);
        if model.low_effort {
            args.extend(["-c".into(), r#"model_reasoning_effort="low""#.into()]);
        }
    }
    args.extend(["-c".into(), r#"web_search="disabled""#.into()]);
    for feature in CODEX_DISABLED {
        if available.iter().any(|name| name == feature) {
            args.extend(["--disable".into(), feature.into()]);
        }
    }
    for flag in [
        "--sandbox",
        "read-only",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--ephemeral",
        "--color",
        "never",
        "-",
    ] {
        args.push(flag.into());
    }
    Plan {
        model: chosen.map_or_else(|| "default".to_owned(), |model| model.slug.clone()),
        args,
        prompt_on_stdin: true,
    }
}

/// The Cursor arguments, the prompt added last.
pub fn cursor_plan(model: Option<&str>) -> Plan {
    let mut args: Vec<OsString> = [
        "-p",
        "--mode",
        "ask",
        "--sandbox",
        "enabled",
        "--output-format",
        "text",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    Plan {
        model: model.unwrap_or("auto").to_owned(),
        args,
        prompt_on_stdin: false,
    }
}

/// The small, fast model of a `codex debug models` catalog: a listed model
/// whose description says "fast" and does not call it older or previous,
/// the one Codex itself ranks first (lowest `priority`). None when the
/// catalog cannot be read or names no such model.
pub fn pick_codex_model(catalog: &[u8]) -> Option<CodexModel> {
    let value: serde_json::Value = serde_json::from_slice(catalog).ok()?;
    let models = value.get("models")?.as_array()?;
    models
        .iter()
        .filter_map(|model| {
            let slug = model.get("slug")?.as_str()?;
            let description = model
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let listed =
                model.get("visibility").and_then(serde_json::Value::as_str) == Some("list");
            let fast = description.contains("fast");
            let old = ["older", "previous", "legacy", "deprecated"]
                .iter()
                .any(|word| description.contains(word));
            let slug_ok = !slug.is_empty()
                && slug.len() <= 64
                && slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
            if !(listed && fast && !old && slug_ok) {
                return None;
            }
            let priority = model
                .get("priority")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(i64::MAX);
            let low_effort = model
                .get("supported_reasoning_levels")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|levels| {
                    levels.iter().any(|level| {
                        level.get("effort").and_then(serde_json::Value::as_str) == Some("low")
                    })
                });
            Some((
                priority,
                CodexModel {
                    slug: slug.to_owned(),
                    low_effort,
                },
            ))
        })
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, model)| model)
}

/// The feature names a `codex features list` prints, one per line as
/// `<name> <stage> <true|false>`.
pub fn codex_features(list: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(list)
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [name, .., state] if matches!(*state, "true" | "false") && words.len() >= 3 => {
                    Some((*name).to_owned())
                }
                _ => None,
            }
        })
        .collect()
}

/// A small model from `cursor-agent --list-models`: the first listed name
/// that says it is small or fast. None otherwise, and Cursor then picks.
pub fn pick_cursor_model(list: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(list);
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(|word| word.trim_matches(|c: char| matches!(c, '-' | '*' | ',' | ':')))
        .find(|name| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
                && ["haiku", "mini", "flash", "fast", "nano"]
                    .iter()
                    .any(|word| name.to_ascii_lowercase().contains(word))
        })
        .map(str::to_owned)
}
/// What a sign-in check printed, whatever its exit.
pub struct SignInStatus {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Whether `claude auth status --json` clearly says signed out: its output
/// is one JSON object whose `loggedIn` is `false`. Anything else (an older
/// Claude Code without the command, unreadable output, no such field) is
/// unknown, not signed out.
pub fn claude_signed_out(status: &SignInStatus) -> bool {
    serde_json::from_slice::<serde_json::Value>(&status.stdout)
        .ok()
        .and_then(|value| value.get("loggedIn").and_then(serde_json::Value::as_bool))
        == Some(false)
}

/// Whether `codex login status` clearly says signed out: it exited with a
/// failure and one of its own lines says "not logged in" (whole words).
/// A success, or a failure that says nothing of the kind, is not.
pub fn codex_signed_out(status: &SignInStatus) -> bool {
    !status.success
        && [&status.stdout, &status.stderr].iter().any(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .any(|line| has_phrase(&line.to_ascii_lowercase(), "not logged in"))
        })
}

/// Whether a tool's own error text says it is not signed in: a line of it
/// that is not part of the prompt (a tool may echo the prompt, and the
/// messages in it may say anything) holds one of [`SIGN_IN_WORDS`] as whole
/// words ("catalog in" and "14012" do not count).
pub fn says_not_signed_in(error_text: &str, prompt: &str) -> bool {
    error_text.lines().any(|line| {
        let line = line.trim();
        !line.is_empty()
            && !prompt.contains(line)
            && SIGN_IN_WORDS
                .iter()
                .any(|phrase| has_phrase(&line.to_ascii_lowercase(), phrase))
    })
}

/// Whether `phrase` occurs in `text` with no letter or digit right before or
/// after it.
fn has_phrase(text: &str, phrase: &str) -> bool {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    text.match_indices(phrase).any(|(at, _)| {
        !word(text[..at].chars().next_back()) && !word(text[at + phrase.len()..].chars().next())
    })
}

/// A child's whole environment: the grading runner's allow-list (`HOME`,
/// `USER`, `LOGNAME`, `LANG`, `TMPDIR`, `LC_*`), `CODEX_HOME` for Codex
/// (where its sign-in lives), a `PATH` of absolute entries (the tool's own
/// folder, the inherited ones, then the usual system folders),
/// `DISABLE_AUTOUPDATER=1` and `NO_COLOR=1`. Every other variable is
/// dropped.
pub fn agent_env(
    tool: Tool,
    binary: &Path,
    inherited: &[(OsString, OsString)],
) -> Vec<(OsString, OsString)> {
    let extra: &[&str] = match tool {
        Tool::Codex => &CODEX_KEPT_ENV,
        Tool::ClaudeCode | Tool::Cursor => &[],
    };
    let mut env: Vec<(OsString, OsString)> = inherited
        .iter()
        .filter(|(key, _)| {
            key.to_str().is_some_and(|key| {
                runner::KEPT_ENV.contains(&key) || key.starts_with("LC_") || extra.contains(&key)
            })
        })
        .cloned()
        .collect();
    let entries: Vec<PathBuf> = xt_probes::executable::claude_child_path_entries(
        binary,
        runner::inherited_var(inherited, "PATH"),
    )
    .into_iter()
    .filter(|entry| entry.is_absolute())
    .collect();
    if let Some(path) = xt_probes::executable::join_path(&entries) {
        env.push(("PATH".into(), path));
    }
    env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
    env.push(("NO_COLOR".into(), "1".into()));
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codex_model_is_the_listed_fast_one_codex_ranks_first() {
        let catalog = br#"{"models":[
          {"slug":"big","description":"Frontier intelligence.","visibility":"list","priority":1},
          {"slug":"hidden-fast","description":"Fast and affordable.","visibility":"hide","priority":2},
          {"slug":"old-fast","description":"Older fast and efficient model.","visibility":"list","priority":3},
          {"slug":"small-b","description":"Fast model.","visibility":"list","priority":9,
           "supported_reasoning_levels":[{"effort":"medium"}]},
          {"slug":"small-a","description":"Fast and affordable model for easier tasks.","visibility":"list","priority":5,
           "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"}]}
        ]}"#;
        assert_eq!(
            pick_codex_model(catalog),
            Some(CodexModel {
                slug: "small-a".into(),
                low_effort: true
            })
        );
        assert_eq!(pick_codex_model(b"not json"), None);
        assert_eq!(
            pick_codex_model(
                br#"{"models":[{"slug":"x","description":"Frontier","visibility":"list"}]}"#
            ),
            None
        );
        // A slug that is not a plain name is never passed on.
        assert_eq!(
            pick_codex_model(
                br#"{"models":[{"slug":"a b","description":"fast","visibility":"list"}]}"#
            ),
            None
        );
    }

    #[test]
    fn codex_turns_off_only_the_features_it_lists() {
        let list =
            b"apps                stable  true\nshell_tool  stable  true\nhooks stable true\n\
                     weird line\nnotours under development false\n";
        let features = codex_features(list);
        assert_eq!(features, ["apps", "shell_tool", "hooks", "notours"]);
        let plan = codex_plan(
            Some(&CodexModel {
                slug: "small-a".into(),
                low_effort: true,
            }),
            &features,
        );
        let args: Vec<String> = plan
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "exec",
                "-m",
                "small-a",
                "-c",
                "model_reasoning_effort=\"low\"",
                "-c",
                "web_search=\"disabled\"",
                "--disable",
                "shell_tool",
                "--disable",
                "apps",
                "--disable",
                "hooks",
                "--sandbox",
                "read-only",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "--ephemeral",
                "--color",
                "never",
                "-"
            ]
        );
        assert!(plan.prompt_on_stdin);
        assert!(codex_shell_off(&features));
        assert!(!codex_shell_off(&[]));
        assert!(!codex_shell_off(&codex_features(
            b"apps stable true\nunified_exec stable true\n"
        )));
        let plain = codex_plan(None, &[]);
        assert_eq!(plain.model, "default");
        assert!(!plain.args.iter().any(|a| a == "-m"));
    }

    #[test]
    fn claude_and_cursor_command_lines_are_fixed() {
        let claude = claude_plan();
        let args: Vec<String> = claude
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-p",
                "--model",
                "haiku",
                "--tools",
                "",
                "--setting-sources",
                "",
                "--settings",
                "{\"disableAllHooks\":true}",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--output-format",
                "json"
            ]
        );
        assert!(claude.prompt_on_stdin);
        let cursor = cursor_plan(Some("small"));
        assert!(!cursor.prompt_on_stdin);
        assert_eq!(cursor.model, "small");
        assert_eq!(cursor_plan(None).model, "auto");
        assert_eq!(
            pick_cursor_model(b"Available models\n\ngpt-6 - GPT 6\nclaude-haiku-5 - Haiku\n"),
            Some("claude-haiku-5".into())
        );
        assert_eq!(pick_cursor_model(b"auto\nsonnet-5\n"), None);
    }

    #[test]
    fn only_a_clear_signed_out_status_skips_a_tool() {
        let status = |success, stdout: &str, stderr: &str| SignInStatus {
            success,
            stdout: stdout.into(),
            stderr: stderr.into(),
        };
        assert!(claude_signed_out(&status(
            true,
            r#"{"loggedIn": false, "authMethod": "none"}"#,
            ""
        )));
        for unknown in [
            r#"{"loggedIn": true, "authMethod": "claude.ai"}"#,
            r#"{"authMethod": "none"}"#,
            r#"{"loggedIn": "false"}"#,
            "error: unknown command 'auth'",
            "",
        ] {
            assert!(!claude_signed_out(&status(false, unknown, "")), "{unknown}");
        }
        assert!(codex_signed_out(&status(false, "", "Not logged in\n")));
        assert!(codex_signed_out(&status(false, "Not logged in", "")));
        assert!(!codex_signed_out(&status(
            true,
            "Logged in using ChatGPT",
            ""
        )));
        assert!(!codex_signed_out(&status(true, "Not logged in", "")));
        assert!(!codex_signed_out(&status(
            false,
            "",
            "error: unrecognized subcommand 'login'"
        )));
    }

    #[test]
    fn not_signed_in_is_read_only_from_the_tools_own_error_lines() {
        let prompt = "RULES\n  please login to the staging site first\n";
        assert!(says_not_signed_in(
            "Error: Not logged in. Please run /login",
            prompt
        ));
        assert!(says_not_signed_in("HTTP 401 from the API", prompt));
        assert!(says_not_signed_in(
            "Failed to authenticate: OAuth session expired",
            prompt
        ));
        // Echoed prompt lines never count.
        assert!(!says_not_signed_in(
            "user\nplease login to the staging site first\n",
            prompt
        ));
        // Whole words only.
        assert!(!says_not_signed_in(
            "error: the catalog in use is stale",
            prompt
        ));
        assert!(!says_not_signed_in("exit status 14012", prompt));
        assert!(!says_not_signed_in("loginShell missing", prompt));
    }

    #[test]
    fn the_prompt_is_the_instructions_then_the_reason_then_the_input() {
        let first = prompt("RULES\n", "INPUT", None);
        assert_eq!(first, "RULES\n\n===== The input =====\n\nINPUT");
        let second = prompt("RULES", "INPUT", Some(&AnswerError::Keys));
        assert!(second.starts_with("RULES\n\n\nYour previous answer"));
        assert!(second.ends_with("===== The input =====\n\nINPUT"));
    }
}
