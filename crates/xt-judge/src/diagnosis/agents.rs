//! One diagnosis run: a small hosted model reads one built input
//! ([`super::input`]) with the bundled skill's instructions and answers with
//! one JSON object, checked by [`super::answer`].
//!
//! **Which tool.** Claude Code first, then Codex ([`ORDER`]). A tool is
//! skipped, with the reason kept ([`Tried`]), only while it has not been
//! given the messages: it is not installed, too old, its own sign-in check
//! says it is signed out ([`claude_signed_out`], [`codex_signed_out`];
//! Cursor: its model list fails), its shell cannot be turned off (Codex),
//! or it cannot be started at all; the next one is tried. A sign-in check
//! that gives no clear answer is not a reason to skip. Once a tool has been
//! started with the prompt, it is the only one that ever gets these
//! messages: its failure or timeout ends the run ([`DiagnoseError::Run`]),
//! and its one retry after a broken answer is its own. When none can be used
//! the run says so ([`DiagnoseError::NoTool`]).
//!
//! **Cursor is not tried automatically.** Its run is kept
//! ([`cursor_plan`], [`pick_cursor_model`], [`diagnose_in_order`]) but is not
//! in [`ORDER`], because: the prompt, with the messages, goes on its command
//! line, where other programs on this Mac can read it; it was never seen
//! working end to end; ask mode still has read tools and may load the
//! person's MCP servers and rules; and its chat may be saved and then indexed
//! again by XTrace.
//!
//! **How each is run.** From an empty private folder
//! (`<tmp>/xtrace-grading/diagnose/<run id>/`, removed afterwards), with no
//! standard input other than the prompt, in its own process group, with an
//! allow-listed environment ([`agent_env`]), a deadline, capped output and
//! cancel. The exact command lines:
//!
//! ```text
//! claude -p --model haiku --tools "" --setting-sources ""
//!   --settings {"disableAllHooks":true} --strict-mcp-config
//!   --no-session-persistence --output-format json        (prompt on stdin)
//!
//! codex exec [-m <small model>] [-c model_reasoning_effort="low"]
//!   -c web_search="disabled" [--disable <feature>]…
//!   --sandbox read-only --ignore-user-config --ignore-rules
//!   --skip-git-repo-check --ephemeral --color never -    (prompt on stdin)
//!
//! cursor-agent -p --mode ask --sandbox enabled --output-format text
//!   [--model <small model>] <prompt>
//! ```
//!
//! * Claude Code: no tools at all, no settings file (so none of the
//!   person's permission rules, hooks or plugins), hooks off, no MCP server,
//!   and the run's transcript not saved, so XTrace never indexes it. The
//!   version is checked first with the grading runner's own check
//!   ([`runner::check_claude_version`]), then its sign-in with
//!   `claude auth status --json`, run like the run itself (same
//!   environment, same empty folder). Its JSON result names the model
//!   that answered when its `modelUsage` lists exactly one.
//! * Codex: its model is chosen from what `codex debug models` reports
//!   ([`pick_codex_model`]): the listed model whose description calls it
//!   fast, not an older one, first by Codex's own priority; with none, no
//!   `-m` is passed and Codex uses its default. `codex login status` is
//!   checked first, like Claude Code's sign-in. Its features list
//!   (`codex features list`) is read and every one of [`CODEX_DISABLED`]
//!   that this Codex has is turned off (an unknown name would stop Codex),
//!   so it has no shell, no apps, plugins, hooks or other agents. Codex is
//!   run only when that list was read and names `shell_tool`, so its shell
//!   is surely off ([`codex_shell_off`]); otherwise it is skipped
//!   ([`Skip::ShellNotOff`]). Read-only sandbox, no user `config.toml`, no
//!   rules files, nothing saved.
//!   **Known limit:** Codex still loads the person's `~/.codex/AGENTS.md`
//!   and its list of installed skills ([`Limit::CodexLoadsUserInstructions`]).
//! * Cursor (only through [`diagnose_in_order`]): `cursor-agent
//!   --list-models` must succeed first; it fails when Cursor's agent is not
//!   signed in (`cursor-agent login`), and the tool is then skipped as not
//!   signed in. Ask mode is read-only. **Known limits:**
//!   the prompt, with the messages, is passed on the command line, where
//!   other programs on this Mac can read it while it runs
//!   ([`Limit::CursorPromptOnCommandLine`]); and this path was not seen
//!   working end to end here ([`Limit::CursorUnverified`]).
//!
//! The prompt ([`prompt`]) is the skill's instructions, then (on the second
//! try) why the first answer was refused, then the input.
//!
//! Only Unix hosts are supported.

use super::answer::{self, Answer, AnswerError};
use crate::runner::{self, GradeError, Launch, Limits};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};
use xt_probes::{executable::Unavailable, process::CancelFlag};

/// The tools a run uses, in the order they are tried. Cursor is left out on
/// purpose (see the module's notes).
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
/// Retained output of a check.
const MAX_CHECK_STDOUT_BYTES: usize = 2 * 1024 * 1024;
/// How long a sign-in check may take, at most.
pub const SIGN_IN_CHECK_DEADLINE: Duration = Duration::from_secs(10);
/// Output of a sign-in check read, at most, per stream.
const MAX_SIGN_IN_BYTES: usize = 16 * 1024;
/// Retained standard error; only looked at for a sign-in failure.
const MAX_STDERR_BYTES: usize = 64 * 1024;
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

/// Run one diagnosis with the tools of [`ORDER`], as the module says.
pub fn diagnose(
    job: &DiagnoseJob<'_>,
    cancel: Option<&CancelFlag>,
) -> Result<Diagnosis, DiagnoseError> {
    diagnose_in_order(job, &ORDER, cancel)
}

/// Run one diagnosis trying `order`'s tools, under [`diagnose`]'s rules.
/// The app uses [`ORDER`]; another order (Cursor's run) is for tests and a
/// later, explicit choice.
pub fn diagnose_in_order(
    job: &DiagnoseJob<'_>,
    order: &[Tool],
    cancel: Option<&CancelFlag>,
) -> Result<Diagnosis, DiagnoseError> {
    if !cfg!(unix) {
        return Err(DiagnoseError::UnsupportedPlatform);
    }
    let folder = RunFolder::create(job.temp_dir, job.run_id)?;
    let mut tried = Vec::new();
    for &tool in order {
        if cancelled(cancel) {
            return Err(DiagnoseError::Cancelled);
        }
        let Ok(binary) = job.tools.get(tool) else {
            tried.push(Tried {
                tool,
                skip: Skip::NotInstalled,
                given_prompt: false,
            });
            continue;
        };
        let context = Context {
            binary,
            cwd: folder.path(),
            env: agent_env(tool, binary, job.inherited_env),
            limits: job.limits,
            cancel,
        };
        let plan = match plan(tool, &context) {
            Ok(plan) => plan,
            Err(Prepared::Skip(skip)) => {
                tried.push(Tried {
                    tool,
                    skip,
                    given_prompt: false,
                });
                continue;
            }
            Err(Prepared::Cancelled) => return Err(DiagnoseError::Cancelled),
        };
        let mut previous: Option<AnswerError> = None;
        let mut model = Model {
            asked: plan.model.clone(),
            answered: None,
        };
        let mut given = false;
        for index in 0..TRIES {
            if cancelled(cancel) {
                return Err(DiagnoseError::Cancelled);
            }
            let text = prompt(job.instructions, job.input, previous.as_ref());
            let output = match run_try(tool, &plan, &context, &text) {
                Ok(output) => output,
                Err(TryEnd::Cancelled) => return Err(DiagnoseError::Cancelled),
                // Never started, so never given the messages on this try:
                // the next tool may be asked only if no earlier try was.
                Err(TryEnd::NotStarted) if !given => {
                    tried.push(Tried {
                        tool,
                        skip: Skip::CouldNotStart,
                        given_prompt: false,
                    });
                    break;
                }
                Err(TryEnd::NotStarted) => {
                    return Err(DiagnoseError::Run {
                        tool,
                        model,
                        failure: RunFailure::Exited { code: None },
                        tried,
                    });
                }
                // Started with the prompt: this tool's failure is final, so
                // the same messages never go to a second company.
                Err(TryEnd::Failed(failure)) => {
                    return Err(DiagnoseError::Run {
                        tool,
                        model,
                        failure,
                        tried,
                    });
                }
            };
            given = true;
            model.answered = output.model;
            match answer::parse(&output.text) {
                Ok(answer) => {
                    return Ok(Diagnosis {
                        tool,
                        model,
                        answer,
                        tries: index + 1,
                        tried,
                    });
                }
                Err(error) if index + 1 == TRIES => {
                    return Err(DiagnoseError::Refused {
                        tool,
                        model,
                        error,
                        tried,
                    });
                }
                Err(error) => previous = Some(error),
            }
        }
    }
    Err(DiagnoseError::NoTool { tried })
}

fn cancelled(cancel: Option<&CancelFlag>) -> bool {
    cancel.is_some_and(CancelFlag::is_cancelled)
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

struct Context<'a> {
    binary: &'a Path,
    cwd: &'a Path,
    env: Vec<(OsString, OsString)>,
    limits: AgentLimits,
    cancel: Option<&'a CancelFlag>,
}

enum Prepared {
    Skip(Skip),
    Cancelled,
}

enum TryEnd {
    /// The tool could not be started, so it was not given this prompt.
    NotStarted,
    /// It was started with the prompt and failed.
    Failed(RunFailure),
    Cancelled,
}

fn plan(tool: Tool, context: &Context<'_>) -> Result<Plan, Prepared> {
    match tool {
        Tool::ClaudeCode => {
            // The runner's own version check, with the environment that
            // reaches this tool's child.
            let launch = Launch {
                claude: context.binary,
                argv: Vec::new(),
                cwd: context.cwd,
                grade_dir: context.cwd,
                inherited_env: &context.env,
                limits: Limits {
                    version_deadline: context.limits.check_deadline,
                    exit_grace: context.limits.exit_grace,
                    term_grace: context.limits.term_grace,
                    ..Limits::default()
                },
                leader_record: None,
                step: None,
            };
            match runner::check_claude_version(&launch, context.cancel) {
                Ok(()) => {}
                Err(GradeError::Cancelled) => return Err(Prepared::Cancelled),
                Err(GradeError::ClaudeTooOld) => return Err(Prepared::Skip(Skip::TooOld)),
                Err(GradeError::Timeout) => return Err(Prepared::Skip(Skip::TimedOut)),
                Err(_) => return Err(Prepared::Skip(Skip::CouldNotStart)),
            }
            match sign_in_check(context, &["auth", "status", "--json"])? {
                Some(status) if claude_signed_out(&status) => {
                    Err(Prepared::Skip(Skip::NotSignedIn))
                }
                _ => Ok(claude_plan()),
            }
        }
        Tool::Codex => {
            if sign_in_check(context, &["login", "status"])?
                .is_some_and(|status| codex_signed_out(&status))
            {
                return Err(Prepared::Skip(Skip::NotSignedIn));
            }
            let features = check(context, &["features", "list"])?;
            let available = features.map(|out| codex_features(&out)).unwrap_or_default();
            if !codex_shell_off(&available) {
                return Err(Prepared::Skip(Skip::ShellNotOff));
            }
            let models = check(context, &["debug", "models"])?;
            let chosen = models.and_then(|out| pick_codex_model(&out));
            Ok(codex_plan(chosen.as_ref(), &available))
        }
        Tool::Cursor => match check(context, &["--list-models"])? {
            Some(out) => Ok(cursor_plan(pick_cursor_model(&out).as_deref())),
            None => Err(Prepared::Skip(Skip::NotSignedIn)),
        },
    }
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

/// Run one check before a tool's run: its standard output when it exited
/// well, `None` when it exited with a failure.
fn check(context: &Context<'_>, args: &[&str]) -> Result<Option<Vec<u8>>, Prepared> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let bounds = Bounds {
        deadline: context.limits.check_deadline,
        max_stdout_bytes: MAX_CHECK_STDOUT_BYTES,
        max_stderr_bytes: MAX_STDERR_BYTES,
        exit_grace: context.limits.exit_grace,
        term_grace: context.limits.term_grace,
    };
    match run_child(context, &args, None, bounds) {
        Ok(finished) if finished.success => Ok(Some(finished.stdout)),
        Ok(_) => Ok(None),
        Err(ChildEnd::Cancelled) => Err(Prepared::Cancelled),
        Err(ChildEnd::Spawn) => Err(Prepared::Skip(Skip::CouldNotStart)),
        Err(ChildEnd::Timeout) => Err(Prepared::Skip(Skip::TimedOut)),
        Err(ChildEnd::TooLarge | ChildEnd::Unobservable) => Ok(None),
    }
}

/// What a sign-in check printed, whatever its exit.
pub struct SignInStatus {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Run a tool's own sign-in check with the run's environment and empty
/// folder, a short deadline ([`SIGN_IN_CHECK_DEADLINE`] at most) and a small
/// output cap. `None` when it gave nothing readable (could not start, took
/// too long, printed too much): the sign-in is then unknown and the run goes
/// ahead as before.
fn sign_in_check(context: &Context<'_>, args: &[&str]) -> Result<Option<SignInStatus>, Prepared> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let bounds = Bounds {
        deadline: context.limits.check_deadline.min(SIGN_IN_CHECK_DEADLINE),
        max_stdout_bytes: MAX_SIGN_IN_BYTES,
        max_stderr_bytes: MAX_SIGN_IN_BYTES,
        exit_grace: context.limits.exit_grace,
        term_grace: context.limits.term_grace,
    };
    match run_child(context, &args, None, bounds) {
        Ok(finished) => Ok(Some(SignInStatus {
            success: finished.success,
            stdout: finished.stdout,
            stderr: finished.stderr,
        })),
        Err(ChildEnd::Cancelled) => Err(Prepared::Cancelled),
        Err(_) => Ok(None),
    }
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

/// What a try gave back.
struct Output {
    text: String,
    /// The model that answered, when the tool says.
    model: Option<String>,
}

fn run_try(tool: Tool, plan: &Plan, context: &Context<'_>, prompt: &str) -> Result<Output, TryEnd> {
    let mut args = plan.args.clone();
    if !plan.prompt_on_stdin {
        args.push(prompt.into());
    }
    let bounds = Bounds {
        deadline: context.limits.deadline,
        max_stdout_bytes: context.limits.max_stdout_bytes,
        max_stderr_bytes: MAX_STDERR_BYTES,
        exit_grace: context.limits.exit_grace,
        term_grace: context.limits.term_grace,
    };
    let stdin = plan.prompt_on_stdin.then_some(prompt);
    let finished = run_child(context, &args, stdin, bounds).map_err(|end| match end {
        ChildEnd::Cancelled => TryEnd::Cancelled,
        ChildEnd::Timeout => TryEnd::Failed(RunFailure::Timeout),
        ChildEnd::TooLarge => TryEnd::Failed(RunFailure::OutputTooLarge),
        ChildEnd::Spawn => TryEnd::NotStarted,
        ChildEnd::Unobservable => TryEnd::Failed(RunFailure::Unobservable),
    })?;
    match tool {
        Tool::ClaudeCode => claude_output(&finished, prompt),
        Tool::Codex | Tool::Cursor => {
            if !finished.success {
                return Err(TryEnd::Failed(failure_of(&finished, prompt)));
            }
            let text = String::from_utf8(finished.stdout)
                .map_err(|_| TryEnd::Failed(RunFailure::Unreadable))?;
            Ok(Output { text, model: None })
        }
    }
}

/// Claude Code's `--output-format json` result: `result` is the answer's
/// text; `is_error` whose error text says so ([`says_not_signed_in`]) is
/// [`RunFailure::NotSignedIn`]. The model that answered is `modelUsage`'s
/// key only when it has exactly one; otherwise none is named.
fn claude_output(finished: &ChildFinished, prompt: &str) -> Result<Output, TryEnd> {
    let value: Option<serde_json::Value> = serde_json::from_slice(&finished.stdout).ok();
    let Some(value) = value.filter(serde_json::Value::is_object) else {
        return Err(TryEnd::Failed(if finished.success {
            RunFailure::Unreadable
        } else {
            failure_of(finished, prompt)
        }));
    };
    let result = value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let is_error = value
        .get("is_error")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_error || !finished.success {
        return Err(TryEnd::Failed(if says_not_signed_in(result, prompt) {
            RunFailure::NotSignedIn
        } else {
            failure_of(finished, prompt)
        }));
    }
    let model = value
        .get("modelUsage")
        .and_then(serde_json::Value::as_object)
        .filter(|usage| usage.len() == 1)
        .and_then(|usage| usage.keys().next().cloned())
        .filter(|name| {
            name.len() <= 128
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']'))
        });
    Ok(Output {
        text: result.to_owned(),
        model,
    })
}

/// A failed exit: not signed in when one of its standard error lines says
/// so ([`says_not_signed_in`]), otherwise the exit code. Standard output is
/// never read for this: it may hold the model's text.
fn failure_of(finished: &ChildFinished, prompt: &str) -> RunFailure {
    if says_not_signed_in(&String::from_utf8_lossy(&finished.stderr), prompt) {
        RunFailure::NotSignedIn
    } else {
        RunFailure::Exited {
            code: finished.code,
        }
    }
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

#[derive(Clone, Copy)]
struct Bounds {
    deadline: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    exit_grace: Duration,
    term_grace: Duration,
}

struct ChildFinished {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum ChildEnd {
    Spawn,
    Cancelled,
    Timeout,
    TooLarge,
    Unobservable,
}

/// Run the tool once from the empty folder, `stdin` written to it and then
/// closed (or no standard input), bounded and cancellable, its group ended
/// and reaped on every path.
#[cfg(unix)]
fn run_child(
    context: &Context<'_>,
    args: &[OsString],
    stdin: Option<&str>,
    bounds: Bounds,
) -> Result<ChildFinished, ChildEnd> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use xt_probes::process::{self, Ended};

    let mut command = Command::new(context.binary);
    command
        .args(args)
        .current_dir(context.cwd)
        .env_clear()
        .envs(
            context
                .env
                .iter()
                .map(|(k, v)| (k.as_os_str(), v.as_os_str())),
        )
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process::spawn_grouped(&mut command).map_err(|_| ChildEnd::Spawn)?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_owned();
        // Written on its own detached thread, never joined: a child (or a
        // descendant that left its group still holding the pipe) that does
        // not read it can then hold neither the bounded wait, nor cancel,
        // nor the one-at-a-time slot past the deadline. The thread ends when
        // the write finishes or fails because every reader is gone.
        drop(std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        }));
    }
    let finished = process::collect(
        child,
        process::Bounds {
            deadline: bounds.deadline,
            max_stdout_bytes: bounds.max_stdout_bytes,
            max_stderr_bytes: bounds.max_stderr_bytes,
            stdout_overflow_ends: true,
            exit_grace: Some(bounds.exit_grace),
            term_grace: Some(bounds.term_grace),
        },
        context.cancel,
    );
    let finished = finished.map_err(|ended| match ended {
        Ended::Timeout => ChildEnd::Timeout,
        Ended::Cancelled => ChildEnd::Cancelled,
        Ended::OutputTooLarge => ChildEnd::TooLarge,
        Ended::Unobservable => ChildEnd::Unobservable,
    })?;
    Ok(ChildFinished {
        success: finished.status.success(),
        code: finished.status.code(),
        stdout: finished.stdout.bytes,
        stderr: finished.stderr.bytes,
    })
}

#[cfg(not(unix))]
fn run_child(
    _: &Context<'_>,
    _: &[OsString],
    _: Option<&str>,
    _: Bounds,
) -> Result<ChildFinished, ChildEnd> {
    Err(ChildEnd::Spawn)
}

/// The run's empty private folder, removed when dropped.
struct RunFolder {
    path: PathBuf,
}

impl RunFolder {
    #[cfg(unix)]
    fn create(temp_dir: &Path, run_id: &str) -> Result<Self, DiagnoseError> {
        use std::os::unix::fs::DirBuilderExt;
        if !crate::staging::is_run_id(run_id) {
            return Err(DiagnoseError::Staging);
        }
        let parent =
            crate::staging::make_private_parent(temp_dir).map_err(|_| DiagnoseError::Staging)?;
        let parent = crate::staging::make_private_folder(&parent.join(DIAGNOSE_DIR))
            .map_err(|_| DiagnoseError::Staging)?;
        let path = parent.join(run_id);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| DiagnoseError::Staging)?;
        Ok(Self { path })
    }

    #[cfg(not(unix))]
    fn create(_: &Path, _: &str) -> Result<Self, DiagnoseError> {
        Err(DiagnoseError::UnsupportedPlatform)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
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
