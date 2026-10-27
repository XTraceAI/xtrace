//! Diagnosis runs against fake `claude`, `codex` and `cursor-agent`.
#![cfg(unix)]
//!
//! Every case writes synthetic tools (small Python programs) into a fresh
//! folder under Cargo's target temp folder. Each fake records, per call, its
//! arguments, working folder, the names of its environment variables and
//! what it read on standard input, then behaves as the case says. No real
//! coding tool is run; the inherited environment given to the run is
//! synthetic.

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use xt_judge::{
    CancelFlag,
    diagnosis::{
        self,
        agents::{
            self, AgentLimits, DiagnoseError, DiagnoseJob, Model, RunFailure, Skip, Tool, Tools,
            Tried,
        },
        answer::{AnswerError, RealProblem, Suggestion},
    },
};
use xt_probes::executable::Unavailable;

const RUN_ID: &str = "00000000-0000-4000-8000-0000000000d1";

fn python3() -> PathBuf {
    xt_probes::executable::resolve_within(
        "python3",
        None,
        std::env::var_os("PATH").as_deref(),
        &[Path::new("/usr/bin"), Path::new("/opt/homebrew/bin")],
    )
    .expect("python3 is required for these tests")
}

const NOTHING: &str = r#"{"what_happened": "The person declined an offer.", "real_problem": "no", "suggestion": "nothing", "draft": "", "why": "In [2] the no answers [1]."}"#;
const TOO_LONG_LINE: &str = r#"{"what_happened": "npm was used.", "real_problem": "yes", "suggestion": "add_to_claude_md", "draft": "LONGLINE", "why": "[3] and [5] fail."}"#;
const LINE: &str = r#"{"what_happened": "npm was used.", "real_problem": "yes", "suggestion": "add_to_claude_md", "draft": "This project uses pnpm, never npm.", "why": "[3] and [5] fail."}"#;

const CATALOG: &str = r#"{"models":[{"slug":"big","description":"Frontier.","visibility":"list","priority":1},{"slug":"small-fast","description":"Fast and affordable model.","visibility":"list","priority":5,"supported_reasoning_levels":[{"effort":"low"}]}]}"#;

struct Case {
    _root: tempfile::TempDir,
    root: PathBuf,
    temp: PathBuf,
    home: PathBuf,
}

/// The Python every fake starts with: record this call, then `body`. `N` is
/// how many times this tool was called with its run's arguments before
/// (counting from 0), `STDIN` what it read.
fn fake_source(name: &str, root: &Path, body: &str) -> String {
    let python = python3();
    let log = root.join(format!("{name}.calls"));
    format!(
        r#"#!{python}
import json, os, sys, time
args = sys.argv[1:]
stdin = sys.stdin.read() if not sys.stdin.isatty() else ""
log = {log:?}
calls = []
if os.path.exists(log):
    with open(log) as f:
        calls = json.load(f)
N = sum(1 for c in calls if c["run"])
run = not (args[:1] == ["--version"] or args[:2] in (["debug", "models"], ["features", "list"], ["auth", "status"], ["login", "status"]) or args[:1] == ["--list-models"])
calls.append({{"args": args, "cwd": os.getcwd(), "cwd_entries": len(os.listdir(".")), "cwd_mode": os.stat(".").st_mode & 0o777, "env": sorted(os.environ.keys()), "stdin": stdin, "run": run}})
with open(log, "w") as f:
    json.dump(calls, f)
{body}
"#,
        python = python.display(),
        log = log.display().to_string(),
    )
}

impl Case {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("diagnose-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let temp = root.join("tmp");
        let home = root.join("home");
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir_all(&home).unwrap();
        Self {
            _root: dir,
            root,
            temp,
            home,
        }
    }

    fn tool(&self, name: &str, body: &str) -> PathBuf {
        self.raw_tool(name, &fake_source(name, &self.root, body))
    }

    /// A fake whose whole source is `source`: it records nothing and reads
    /// standard input only if `source` does.
    fn raw_tool(&self, name: &str, source: &str) -> PathBuf {
        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let path = bin.join(name);
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn calls(&self, name: &str) -> Vec<serde_json::Value> {
        let path = self.root.join(format!("{name}.calls"));
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap(),
            Err(_) => Vec::new(),
        }
    }

    fn runs(&self, name: &str) -> Vec<serde_json::Value> {
        self.calls(name)
            .into_iter()
            .filter(|call| call["run"] == true)
            .collect()
    }

    fn env(&self) -> Vec<(OsString, OsString)> {
        vec![
            ("HOME".into(), self.home.clone().into()),
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("ANTHROPIC_API_KEY".into(), "secret".into()),
            ("CLAUDECODE".into(), "1".into()),
            ("CODEX_HOME".into(), self.home.join(".codex").into()),
            ("OPENAI_API_KEY".into(), "secret".into()),
        ]
    }

    fn run(
        &self,
        tools: &Tools,
        cancel: Option<&CancelFlag>,
    ) -> Result<agents::Diagnosis, DiagnoseError> {
        self.run_with(
            tools,
            cancel,
            AgentLimits {
                deadline: Duration::from_secs(20),
                check_deadline: Duration::from_secs(20),
                ..AgentLimits::default()
            },
        )
    }

    fn run_with(
        &self,
        tools: &Tools,
        cancel: Option<&CancelFlag>,
        limits: AgentLimits,
    ) -> Result<agents::Diagnosis, DiagnoseError> {
        self.run_in(
            tools,
            &agents::ORDER,
            "XTRACE SESSION DIAGNOSIS INPUT\nTHE-INPUT\n",
            cancel,
            limits,
        )
    }

    fn run_in(
        &self,
        tools: &Tools,
        order: &[Tool],
        input: &str,
        cancel: Option<&CancelFlag>,
        limits: AgentLimits,
    ) -> Result<agents::Diagnosis, DiagnoseError> {
        let env = self.env();
        agents::diagnose_in_order(
            &DiagnoseJob {
                tools,
                instructions: diagnosis::instructions(),
                input,
                temp_dir: &self.temp,
                run_id: RUN_ID,
                inherited_env: &env,
                limits,
            },
            order,
            cancel,
        )
    }
}

fn absent() -> Result<PathBuf, Unavailable> {
    Err(Unavailable::NotFound)
}

/// A fake Claude Code: `--version` answers 2.1.304; a run prints `result` as
/// the JSON result's text.
fn claude_body(results: &[&str]) -> String {
    format!(
        r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
if args[:2] == ["auth", "status"]:
    print(json.dumps({{"loggedIn": True, "authMethod": "claude.ai"}}))
    sys.exit(0)
results = {results:?}
text = results[min(N, len(results) - 1)].replace("LONGLINE", "x" * 250)
print(json.dumps({{"type": "result", "is_error": False, "result": text, "modelUsage": {{"claude-haiku-test": {{}}}}}}))
"#,
        results = results
    )
}

fn codex_body(results: &[&str]) -> String {
    format!(
        r#"if args[:2] == ["login", "status"]:
    print("Logged in using ChatGPT")
    sys.exit(0)
if args[:2] == ["debug", "models"]:
    print({CATALOG:?})
    sys.exit(0)
if args[:2] == ["features", "list"]:
    print("shell_tool   stable  true\nhooks   stable  true\nsomething_else  stable  false")
    sys.exit(0)
results = {results:?}
print(results[min(N, len(results) - 1)].replace("LONGLINE", "x" * 250))
"#,
        results = results
    )
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn claude_code_runs_first_with_no_tools_from_an_empty_folder() {
    let case = Case::new();
    let claude = case.tool("claude", &claude_body(&[NOTHING]));
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::ClaudeCode);
    assert_eq!(
        found.model,
        Model {
            asked: "haiku".into(),
            answered: Some("claude-haiku-test".into())
        }
    );
    assert_eq!(found.tries, 1);
    assert_eq!(found.tried, []);
    assert_eq!(found.answer.real_problem, RealProblem::No);
    assert!(case.runs("codex").is_empty(), "Codex is never asked");
    let calls = case.calls("claude");
    assert_eq!(strings(&calls[0]["args"]), ["--version"]);
    // Its sign-in check runs like the run: same empty folder, same
    // environment.
    assert_eq!(strings(&calls[1]["args"]), ["auth", "status", "--json"]);
    assert_eq!(calls[1]["cwd"], calls[2]["cwd"]);
    assert_eq!(calls[1]["env"], calls[2]["env"]);
    let run = &calls[2];
    assert_eq!(
        strings(&run["args"]),
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
    // The prompt is on standard input, never on the command line.
    let stdin = run["stdin"].as_str().unwrap();
    assert!(stdin.starts_with("# XTrace session diagnosis"));
    assert!(
        stdin.ends_with("===== The input =====\n\nXTRACE SESSION DIAGNOSIS INPUT\nTHE-INPUT\n")
    );
    // From an empty private folder, removed afterwards.
    let cwd = PathBuf::from(run["cwd"].as_str().unwrap());
    assert_eq!(run["cwd_entries"], 0);
    assert_eq!(run["cwd_mode"], 0o700);
    assert!(cwd.ends_with(format!("xtrace-grading/diagnose/{RUN_ID}")));
    assert!(!cwd.exists());
    // Only the allow-listed environment reaches it.
    let env = strings(&run["env"]);
    for dropped in [
        "ANTHROPIC_API_KEY",
        "CLAUDECODE",
        "OPENAI_API_KEY",
        "CODEX_HOME",
    ] {
        assert!(
            !env.iter().any(|k| k == dropped),
            "{dropped} reached Claude Code"
        );
    }
    for kept in ["HOME", "PATH", "DISABLE_AUTOUPDATER", "NO_COLOR"] {
        assert!(env.iter().any(|k| k == kept), "{kept}");
    }
}

#[test]
fn without_claude_code_codex_runs_with_its_small_model_and_features_off() {
    let case = Case::new();
    let codex = case.tool("codex", &codex_body(&[LINE]));
    let tools = Tools {
        claude: absent(),
        codex: Ok(codex),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::Codex);
    // Codex's plain answer does not say which model answered.
    assert_eq!(
        found.model,
        Model {
            asked: "small-fast".into(),
            answered: None
        }
    );
    assert_eq!(found.answer.suggestion, Suggestion::AddToClaudeMd);
    assert_eq!(
        found.tried,
        [Tried {
            tool: Tool::ClaudeCode,
            skip: Skip::NotInstalled,
            given_prompt: false,
        }]
    );
    let run = &case.runs("codex")[0];
    assert_eq!(
        strings(&run["args"]),
        [
            "exec",
            "-m",
            "small-fast",
            "-c",
            "model_reasoning_effort=\"low\"",
            "-c",
            "web_search=\"disabled\"",
            "--disable",
            "shell_tool",
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
    assert!(run["stdin"].as_str().unwrap().contains("THE-INPUT"));
    let env = strings(&run["env"]);
    assert!(env.iter().any(|k| k == "CODEX_HOME"), "its sign-in folder");
    assert!(!env.iter().any(|k| k == "OPENAI_API_KEY"));
}

#[test]
fn codex_whose_shell_cannot_surely_be_turned_off_is_skipped() {
    for features in [
        // Listed, but no `shell_tool` to turn off.
        "print(\"unified_exec  stable  true\\nhooks  stable  true\")\n    sys.exit(0)",
        // The list cannot be read.
        "sys.exit(2)",
    ] {
        let case = Case::new();
        let codex = case.tool(
            "codex",
            &format!(
                "if args[:2] == [\"features\", \"list\"]:\n    {features}\n\
                 if args[:2] == [\"debug\", \"models\"]:\n    print({CATALOG:?})\n    sys.exit(0)\n\
                 print({NOTHING:?})\n"
            ),
        );
        let tools = Tools {
            claude: absent(),
            codex: Ok(codex),
            cursor: absent(),
        };
        match case.run(&tools, None) {
            Err(DiagnoseError::NoTool { tried }) => assert_eq!(
                tried[1],
                Tried {
                    tool: Tool::Codex,
                    skip: Skip::ShellNotOff,
                    given_prompt: false,
                },
                "{features}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(case.runs("codex").is_empty(), "never given the messages");
    }
}

#[test]
fn a_tool_given_the_messages_is_the_only_one_ever_given_them() {
    // Claude Code is started with the prompt and says it is not signed in:
    // the run ends there, Codex is never asked.
    let case = Case::new();
    let claude = case.tool(
        "claude",
        r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
print(json.dumps({"type": "result", "is_error": True, "result": "Failed to authenticate: OAuth session expired"}))
sys.exit(1)
"#,
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    match case.run(&tools, None) {
        Err(DiagnoseError::Run {
            tool,
            failure,
            tried,
            ..
        }) => {
            assert_eq!(tool, Tool::ClaudeCode);
            assert_eq!(failure, RunFailure::NotSignedIn);
            assert_eq!(tried, []);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(case.runs("claude").len(), 1);
    assert!(case.calls("codex").is_empty(), "Codex is never asked");

    // Codex fails after being given the prompt: final too. Its echoed
    // prompt saying "log in" is not read as a sign-in failure.
    let case = Case::new();
    let codex = case.tool(
        "codex",
        &format!(
            r#"if args[:2] == ["debug", "models"]:
    print({CATALOG:?})
    sys.exit(0)
if args[:2] == ["features", "list"]:
    print("shell_tool stable true")
    sys.exit(0)
sys.stderr.write("user\n" + stdin + "\nmodel error\n")
sys.exit(3)
"#
        ),
    );
    let cursor = case.tool("cursor-agent", &format!("print({NOTHING:?})\n"));
    let tools = Tools {
        claude: absent(),
        codex: Ok(codex),
        cursor: Ok(cursor),
    };
    let outcome = case.run_in(
        &tools,
        &agents::ORDER,
        "XTRACE SESSION DIAGNOSIS INPUT\n[1] person:\n  please log in to the admin page\n",
        None,
        AgentLimits {
            deadline: Duration::from_secs(20),
            check_deadline: Duration::from_secs(20),
            ..AgentLimits::default()
        },
    );
    match outcome {
        Err(DiagnoseError::Run { tool, failure, .. }) => {
            assert_eq!(tool, Tool::Codex);
            assert_eq!(failure, RunFailure::Exited { code: Some(3) });
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(case.runs("codex").len(), 1);
    // Cursor is never tried on its own.
    assert!(case.calls("cursor-agent").is_empty());
}

#[test]
fn a_tool_that_cannot_start_or_is_too_old_is_skipped_before_the_messages() {
    // Claude Code passes its version check, then cannot be started for the
    // run: never given the messages, so Codex is asked.
    let case = Case::new();
    let claude = case.tool(
        "claude",
        r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    os.chmod(sys.argv[0], 0o644)
    sys.exit(0)
print("unreachable")
"#,
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::Codex);
    assert_eq!(
        found.tried,
        [Tried {
            tool: Tool::ClaudeCode,
            skip: Skip::CouldNotStart,
            given_prompt: false,
        }]
    );
    assert!(case.runs("claude").is_empty());

    let old = Case::new();
    let claude = old.tool(
        "claude",
        r#"print("2.0.1 (Claude Code)")
"#,
    );
    let tools = Tools {
        claude: Ok(claude),
        codex: absent(),
        cursor: absent(),
    };
    match old.run(&tools, None) {
        Err(DiagnoseError::NoTool { tried }) => assert_eq!(
            tried,
            [
                Tried {
                    tool: Tool::ClaudeCode,
                    skip: Skip::TooOld,
                    given_prompt: false,
                },
                Tried {
                    tool: Tool::Codex,
                    skip: Skip::NotInstalled,
                    given_prompt: false,
                },
            ]
        ),
        other => panic!("{other:?}"),
    }
    assert!(old.runs("claude").is_empty(), "never run");
}

#[test]
fn a_signed_out_tool_is_skipped_before_it_gets_the_messages() {
    // Claude Code's own check says signed out: Codex answers, and Claude
    // Code is never given the prompt.
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &format!(
            r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
if args[:2] == ["auth", "status"]:
    print(json.dumps({{"loggedIn": False, "authMethod": "none"}}))
    sys.exit(0)
print(json.dumps({{"type": "result", "is_error": False, "result": {NOTHING:?}}}))
"#
        ),
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::Codex);
    assert_eq!(
        found.tried,
        [Tried {
            tool: Tool::ClaudeCode,
            skip: Skip::NotSignedIn,
            given_prompt: false,
        }]
    );
    assert!(case.runs("claude").is_empty(), "never given the prompt");
    assert!(
        case.calls("claude")
            .iter()
            .all(|call| call["stdin"].as_str() == Some("")),
        "no call read any message"
    );

    // Codex signed out too: no tool, nothing sent.
    let case = Case::new();
    let claude = case.tool(
        "claude",
        r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
print(json.dumps({"loggedIn": False}))
"#,
    );
    let codex = case.tool(
        "codex",
        r#"if args[:2] == ["login", "status"]:
    sys.stderr.write("Not logged in\n")
    sys.exit(1)
print("ran")
"#,
    );
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    match case.run(&tools, None) {
        Err(DiagnoseError::NoTool { tried }) => assert_eq!(
            tried,
            [
                Tried {
                    tool: Tool::ClaudeCode,
                    skip: Skip::NotSignedIn,
                    given_prompt: false,
                },
                Tried {
                    tool: Tool::Codex,
                    skip: Skip::NotSignedIn,
                    given_prompt: false,
                },
            ]
        ),
        other => panic!("{other:?}"),
    }
    assert!(case.runs("claude").is_empty() && case.runs("codex").is_empty());
}

#[test]
fn a_sign_in_check_with_no_clear_answer_lets_the_run_go_ahead() {
    // An older Claude Code without `auth status`: the run goes ahead.
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &format!(
            r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
if args[:2] == ["auth", "status"]:
    sys.stderr.write("error: unknown command 'auth'\n")
    sys.exit(1)
print(json.dumps({{"type": "result", "is_error": False, "result": {NOTHING:?}}}))
"#
        ),
    );
    let tools = Tools {
        claude: Ok(claude),
        codex: absent(),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::ClaudeCode);
    assert_eq!(case.runs("claude").len(), 1);

    // A Codex whose status check fails without saying "not logged in".
    let case = Case::new();
    let codex = case.tool(
        "codex",
        &format!(
            r#"if args[:2] == ["login", "status"]:
    print("something unexpected")
    sys.exit(2)
if args[:2] == ["features", "list"]:
    print("shell_tool stable true")
    sys.exit(0)
if args[:2] == ["debug", "models"]:
    sys.exit(1)
print({NOTHING:?})
"#
        ),
    );
    let tools = Tools {
        claude: absent(),
        codex: Ok(codex),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tool, Tool::Codex);
    assert_eq!(case.runs("codex").len(), 1);
}

#[test]
fn several_models_in_claudes_usage_name_no_answering_model() {
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &format!(
            r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
print(json.dumps({{"type": "result", "is_error": False, "result": {NOTHING:?}, "modelUsage": {{"model-a": {{}}, "model-b": {{}}}}}}))
"#
        ),
    );
    let tools = Tools {
        claude: Ok(claude),
        codex: absent(),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.model.answered, None);
    assert_eq!(found.model.asked, "haiku");
}

#[test]
fn cursor_runs_only_when_asked_for_with_the_prompt_last() {
    let case = Case::new();
    let cursor = case.tool(
        "cursor-agent",
        &format!(
            r#"if args[:1] == ["--list-models"]:
    print("gpt-6\nclaude-haiku-5\n")
    sys.exit(0)
print({NOTHING:?})
"#
        ),
    );
    let tools = Tools {
        claude: absent(),
        codex: absent(),
        cursor: Ok(cursor),
    };
    let found = case
        .run_in(
            &tools,
            &[Tool::Cursor],
            "XTRACE SESSION DIAGNOSIS INPUT\nTHE-INPUT\n",
            None,
            AgentLimits {
                deadline: Duration::from_secs(20),
                check_deadline: Duration::from_secs(20),
                ..AgentLimits::default()
            },
        )
        .unwrap();
    assert_eq!(found.tool, Tool::Cursor);
    assert_eq!(found.model.asked, "claude-haiku-5");
    // Cursor takes the prompt as its last argument.
    let args = strings(&case.runs("cursor-agent")[0]["args"]);
    assert_eq!(
        args[..9],
        [
            "-p",
            "--mode",
            "ask",
            "--sandbox",
            "enabled",
            "--output-format",
            "text",
            "--model",
            "claude-haiku-5"
        ]
    );
    assert!(args[9].ends_with("THE-INPUT\n"));
    assert_eq!(args.len(), 10);
    // The app's own order never reaches it.
    match case.run(&tools, None) {
        Err(DiagnoseError::NoTool { tried }) => assert_eq!(tried.len(), 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(case.runs("cursor-agent").len(), 1);
}

#[test]
fn cursor_that_is_not_signed_in_is_skipped_and_none_left_is_said_plainly() {
    let case = Case::new();
    let cursor = case.tool(
        "cursor-agent",
        r#"sys.stderr.write("Error: Authentication required. Run 'agent login'\n")
sys.exit(1)
"#,
    );
    let tools = Tools {
        claude: absent(),
        codex: absent(),
        cursor: Ok(cursor),
    };
    let order = [Tool::ClaudeCode, Tool::Codex, Tool::Cursor];
    let limits = AgentLimits {
        deadline: Duration::from_secs(20),
        check_deadline: Duration::from_secs(20),
        ..AgentLimits::default()
    };
    match case.run_in(&tools, &order, "INPUT\n", None, limits) {
        Err(error @ DiagnoseError::NoTool { .. }) => {
            assert_eq!(
                error.to_string(),
                "no coding tool that can run a diagnosis is available"
            );
            let DiagnoseError::NoTool { tried } = error else {
                unreachable!()
            };
            assert_eq!(
                tried[2],
                Tried {
                    tool: Tool::Cursor,
                    skip: Skip::NotSignedIn,
                    given_prompt: false,
                }
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(case.runs("cursor-agent").is_empty());
}

#[test]
fn a_refused_answer_is_retried_once_with_the_reason() {
    let case = Case::new();
    let claude = case.tool("claude", &claude_body(&[TOO_LONG_LINE, LINE]));
    let tools = Tools {
        claude: Ok(claude),
        codex: absent(),
        cursor: absent(),
    };
    let found = case.run(&tools, None).unwrap();
    assert_eq!(found.tries, 2);
    assert_eq!(found.answer.draft, "This project uses pnpm, never npm.");
    let runs = case.runs("claude");
    assert_eq!(runs.len(), 2);
    assert!(
        !runs[0]["stdin"]
            .as_str()
            .unwrap()
            .contains("refused by XTrace")
    );
    let second = runs[1]["stdin"].as_str().unwrap();
    assert!(second.contains(&format!("refused by XTrace: {}", AnswerError::Line)));
    assert!(second.ends_with("THE-INPUT\n"));
}

#[test]
fn two_refused_answers_fail_without_asking_another_tool() {
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &claude_body(&["Sure! Here is the answer.", TOO_LONG_LINE]),
    );
    let codex = case.tool("codex", &codex_body(&[LINE]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    match case.run(&tools, None) {
        Err(DiagnoseError::Refused {
            tool, model, error, ..
        }) => {
            assert_eq!(tool, Tool::ClaudeCode);
            assert_eq!(model.answered.as_deref(), Some("claude-haiku-test"));
            assert_eq!(error, AnswerError::Line);
        }
        other => panic!("{other:?}"),
    }
    let second = case.runs("claude")[1]["stdin"].as_str().unwrap().to_owned();
    assert!(second.contains(&AnswerError::NotOneObject.to_string()));
    assert!(case.calls("codex").is_empty());
}

#[test]
fn a_tool_past_its_deadline_ends_the_run_and_a_cancel_ends_the_run() {
    let case = Case::new();
    let claude = case.tool(
        "claude",
        r#"if args[:1] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
if args[:2] == ["auth", "status"]:
    print(json.dumps({"loggedIn": True}))
    sys.exit(0)
time.sleep(30)
"#,
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let outcome = case.run_with(
        &tools,
        None,
        AgentLimits {
            deadline: Duration::from_secs(2),
            check_deadline: Duration::from_secs(20),
            term_grace: Duration::from_millis(200),
            exit_grace: Duration::from_millis(200),
            ..AgentLimits::default()
        },
    );
    match outcome {
        Err(DiagnoseError::Run { tool, failure, .. }) => {
            assert_eq!(tool, Tool::ClaudeCode);
            assert_eq!(failure, RunFailure::Timeout);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        case.calls("codex").is_empty(),
        "the messages go nowhere else"
    );

    let cancel = CancelFlag::new();
    cancel.cancel();
    assert_eq!(
        case.run(&tools, Some(&cancel)),
        Err(DiagnoseError::Cancelled)
    );
    // A run ID that is not a run ID's shape prepares nothing.
    let env = case.env();
    assert_eq!(
        agents::diagnose(
            &DiagnoseJob {
                tools: &tools,
                instructions: "x",
                input: "y",
                temp_dir: &case.temp,
                run_id: "../escape",
                inherited_env: &env,
                limits: AgentLimits::default(),
            },
            None,
        ),
        Err(DiagnoseError::Staging)
    );
}

#[test]
fn a_child_that_never_reads_its_input_cannot_hold_the_run_past_its_deadline() {
    let case = Case::new();
    let python = python3();
    let pid_file = case.root.join("grandchild.pid");
    // Never reads standard input; leaves a descendant in its own session
    // holding it open, out of reach of the group kill.
    let claude = case.raw_tool(
        "claude",
        &format!(
            r#"#!{python}
import os, sys, time
if sys.argv[1:2] == ["--version"]:
    print("2.1.304 (Claude Code)")
    sys.exit(0)
if sys.argv[1:3] == ["auth", "status"]:
    print('{{"loggedIn": true}}')
    sys.exit(0)
if os.fork() == 0:
    os.setsid()
    devnull = os.open(os.devnull, os.O_WRONLY)
    os.dup2(devnull, 1)
    os.dup2(devnull, 2)
    with open({pid:?}, "w") as f:
        f.write(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
time.sleep(30)
"#,
            python = python.display(),
            pid = pid_file.display().to_string(),
        ),
    );
    let tools = Tools {
        claude: Ok(claude),
        codex: absent(),
        cursor: absent(),
    };
    // Far more than a pipe holds, so the writer is left blocked.
    let input = format!("XTRACE SESSION DIAGNOSIS INPUT\n{}\n", "x".repeat(4 << 20));
    let started = std::time::Instant::now();
    let outcome = case.run_in(
        &tools,
        &agents::ORDER,
        &input,
        None,
        AgentLimits {
            deadline: Duration::from_secs(2),
            check_deadline: Duration::from_secs(20),
            term_grace: Duration::from_millis(200),
            exit_grace: Duration::from_millis(200),
            ..AgentLimits::default()
        },
    );
    let elapsed = started.elapsed();
    if let Ok(pid) = fs::read_to_string(&pid_file) {
        let _ = std::process::Command::new("/bin/kill")
            .arg(pid.trim())
            .status();
    }
    assert!(
        matches!(
            outcome,
            Err(DiagnoseError::Run {
                failure: RunFailure::Timeout,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
}

#[test]
fn tools_are_found_from_overrides_path_and_known_folders() {
    let case = Case::new();
    let claude = case.tool("claude", "");
    let codex = case.tool("codex", "");
    let local = case.home.join(".local/bin");
    fs::create_dir_all(&local).unwrap();
    let cursor = local.join("cursor-agent");
    fs::copy(&codex, &cursor).unwrap();
    let env = vec![
        ("HOME".into(), case.home.clone().into()),
        ("PATH".into(), case.root.join("bin").into()),
        (agents::CODEX_ENV.into(), codex.clone().into()),
    ];
    let found = agents::find_tools(&env);
    assert_eq!(found.claude, Ok(claude.clone()));
    assert_eq!(found.codex, Ok(codex.clone()));
    assert_eq!(found.cursor, Ok(cursor));
    let env: Vec<(OsString, OsString)> = vec![
        ("HOME".into(), case.home.clone().into()),
        ("PATH".into(), "/nonexistent".into()),
        (xt_probes::executable::CLAUDE_ENV.into(), claude.into()),
        (agents::CODEX_ENV.into(), codex.into()),
        (agents::CURSOR_ENV.into(), "relative/cursor".into()),
    ];
    assert_eq!(
        agents::find_tools(&env).cursor,
        Err(Unavailable::OverrideNotAbsolute)
    );
}

#[test]
fn stdout_overflow_ends_the_attempt_without_asking_another_provider() {
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &claude_body(&[NOTHING]).replace(
            "results = ",
            "sys.stdout.write('x' * 8192)\nsys.stdout.flush()\ntime.sleep(30)\nresults = ",
        ),
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let started = std::time::Instant::now();
    let outcome = case.run_with(
        &tools,
        None,
        AgentLimits {
            max_stdout_bytes: 1024,
            term_grace: Duration::from_millis(200),
            ..AgentLimits::default()
        },
    );
    assert!(
        matches!(
            outcome,
            Err(DiagnoseError::Run {
                tool: Tool::ClaudeCode,
                failure: RunFailure::OutputTooLarge,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(case.runs("claude").len(), 1);
    assert!(case.calls("codex").is_empty());
    let cwd = PathBuf::from(case.runs("claude")[0]["cwd"].as_str().unwrap());
    assert!(!cwd.exists());
}

#[test]
fn cancelling_a_running_attempt_ends_its_group_and_removes_its_folder() {
    let case = Case::new();
    let ready = case.root.join("running.pid");
    let body = claude_body(&[NOTHING]).replace(
        "results = ",
        &format!(
            "if os.fork() == 0:\n    time.sleep(30)\n    os._exit(0)\nwith open({ready:?}, 'w') as f:\n    f.write(str(os.getpid()))\ntime.sleep(30)\nresults = ",
            ready = ready.display().to_string(),
        ),
    );
    let claude = case.tool("claude", &body);
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let cancel = CancelFlag::new();
    let trigger = cancel.clone();
    let ready_copy = ready.clone();
    let canceller = std::thread::spawn(move || {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !ready_copy.exists() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        let running = ready_copy.exists();
        trigger.cancel();
        running
    });
    let started = std::time::Instant::now();
    let outcome = case.run_with(
        &tools,
        Some(&cancel),
        AgentLimits {
            term_grace: Duration::from_millis(200),
            ..AgentLimits::default()
        },
    );
    assert!(
        canceller.join().unwrap(),
        "cancel waits until the attempt starts"
    );
    assert_eq!(outcome, Err(DiagnoseError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(case.runs("claude").len(), 1);
    assert!(case.calls("codex").is_empty());
    let cwd = PathBuf::from(case.runs("claude")[0]["cwd"].as_str().unwrap());
    assert!(!cwd.exists());
    let pgid = fs::read_to_string(ready).unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let alive = std::process::Command::new("/usr/bin/pgrep")
            .args(["-g", pgid.trim()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(
            std::time::Instant::now() < end,
            "the attempt's group survived"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_retry_that_cannot_start_never_falls_through_to_another_provider() {
    let case = Case::new();
    let claude = case.tool(
        "claude",
        &claude_body(&[TOO_LONG_LINE])
            .replace("results = ", "os.chmod(sys.argv[0], 0o644)\nresults = "),
    );
    let codex = case.tool("codex", &codex_body(&[NOTHING]));
    let tools = Tools {
        claude: Ok(claude),
        codex: Ok(codex),
        cursor: absent(),
    };
    let outcome = case.run(&tools, None);
    assert!(
        matches!(
            outcome,
            Err(DiagnoseError::Run {
                tool: Tool::ClaudeCode,
                failure: RunFailure::Exited { code: None },
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(case.runs("claude").len(), 1);
    assert!(
        case.calls("codex").is_empty(),
        "the first attempt already received the messages"
    );
    let cwd = PathBuf::from(case.runs("claude")[0]["cwd"].as_str().unwrap());
    assert!(!cwd.exists());
}

#[test]
fn an_unreadable_model_catalog_uses_codex_default_without_skipping_it() {
    let case = Case::new();
    for catalog_reply in ["sys.exit(3)", "print('not a model catalog')"] {
        let codex = case.tool(
            "codex",
            &codex_body(&[NOTHING]).replace(&format!("print({CATALOG:?})"), catalog_reply),
        );
        let tools = Tools {
            claude: absent(),
            codex: Ok(codex),
            cursor: absent(),
        };
        let found = case.run(&tools, None).unwrap();
        assert_eq!(found.tool, Tool::Codex);
        assert_eq!(found.model.asked, "default");
        let runs = case.runs("codex");
        let args = strings(&runs.last().unwrap()["args"]);
        assert!(!args.iter().any(|arg| arg == "-m"));
    }
}
