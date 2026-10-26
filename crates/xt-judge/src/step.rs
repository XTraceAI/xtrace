//! The step a headless Claude Code run is on, for the progress shown while it
//! runs: the one rule ([`step_of_line`]) that turns one line of Claude Code's
//! `--output-format stream-json` output into a short label such as
//! "Running analyze.py", and the cell a run keeps its latest label in
//! ([`StepCell`]).
//!
//! Claude Code writes one JSON object per line: `system` (`init` first),
//! `assistant` messages whose content may hold `tool_use` blocks, `user`
//! messages carrying tool results, and a final `result`. Only the start, a
//! tool use and the result change the step; everything else, and any line
//! that is not such an object, leaves it as it was.
//!
//! A label holds a verb and at most one base name (of a file, a script or a
//! program), cleaned of anything but letters, digits and `._-+@` and cut to
//! [`MAX_NAME_CHARS`]. Message text, file contents, search patterns, whole
//! commands and folder paths are never kept.

use serde::Deserialize;
use std::sync::{Arc, Mutex, PoisonError};

/// Most characters of a name in a label; a longer one ends in "…".
pub const MAX_NAME_CHARS: usize = 40;

/// Extensions that make a command word the script a command runs.
const SCRIPT_EXTENSIONS: [&str; 10] = [
    "py", "sh", "bash", "zsh", "js", "mjs", "cjs", "ts", "rb", "pl",
];

/// The latest step of one run, shared between the run (which writes it from
/// [`step_of_line`] only) and whoever reports its progress. Empty until the
/// first line that names a step.
#[derive(Clone, Debug, Default)]
pub struct StepCell(Arc<Mutex<Option<String>>>);

impl StepCell {
    pub fn new() -> Self {
        Self::default()
    }

    /// The latest step's label; `None` before the first.
    pub fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Read one output line: a line that names a step replaces the label.
    pub(crate) fn observe(&self, line: &[u8]) {
        if let Some(step) = step_of_line(line) {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(step);
        }
    }
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    subtype: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<Vec<Block>>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Block {
    #[serde(rename = "tool_use")]
    ToolUse { name: String, input: Option<Input> },
    #[serde(other)]
    Other,
}

/// The only tool input fields a label is made from. Every other field (a
/// file's contents, a pattern) is skipped unread.
#[derive(Deserialize)]
struct Input {
    command: Option<serde_json::Value>,
    file_path: Option<serde_json::Value>,
}

/// The step one stream-json line names, as a short label; `None` when the
/// line does not change the step (not a JSON object of a known kind, an
/// assistant message with no tool use, a tool result, anything else).
pub fn step_of_line(line: &[u8]) -> Option<String> {
    let line: Line = serde_json::from_slice(line).ok()?;
    match line.kind.as_str() {
        "system" if line.subtype.as_deref() == Some("init") => {
            Some("Starting Claude Code".to_owned())
        }
        "assistant" => {
            let blocks = line.message?.content?;
            blocks.into_iter().rev().find_map(|block| match block {
                Block::ToolUse { name, input } => Some(tool_step(&name, input.as_ref())),
                Block::Other => None,
            })
        }
        "result" => Some("Finishing".to_owned()),
        _ => None,
    }
}

fn tool_step(name: &str, input: Option<&Input>) -> String {
    let file = || {
        input
            .and_then(|input| input.file_path.as_ref())
            .and_then(serde_json::Value::as_str)
            .and_then(|path| clean_name(base_name(path)))
    };
    let with = |verb: &str, name: Option<String>, fallback: &str| match name {
        Some(name) => format!("{verb} {name}"),
        None => format!("{verb} {fallback}"),
    };
    match name {
        "Read" => with("Reading", file(), "a file"),
        "Write" => with("Writing", file(), "a file"),
        "Edit" => with("Editing", file(), "a file"),
        "Bash" => with(
            "Running",
            input
                .and_then(|input| input.command.as_ref())
                .and_then(serde_json::Value::as_str)
                .and_then(command_name),
            "a command",
        ),
        "Glob" => "Finding files".to_owned(),
        "Grep" => "Searching files".to_owned(),
        other => with("Using", clean_name(other), "a tool"),
    }
}

/// What a shell command runs, as one name. Only its first line is read, so
/// a heredoc's body is never looked at. The line is split into parts (at
/// `&&`, `||`, `;`, `|` and parentheses); in each, leading `NAME=value` words
/// are skipped and the next word is its program.
///
/// * A script, when a part runs one: its program is the script itself
///   (`./analyze.py`, `scripts/run.sh`), or an [`INTERPRETERS`] program
///   whose first argument is the script (`python3 scripts/analyze.py`),
///   optionally after Python's standalone `-u` flag. Other interpreter
///   options name no script. A shell given exactly `-c` or `-lc`
///   runs its command string, which is read the same way. A script is a
///   word whose base name ends in a [`SCRIPT_EXTENSIONS`] extension and has
///   nothing a name may not show. A file that is only an argument
///   (`grep x deploy.sh`, `echo > out.py`) is not a script being run.
/// * Otherwise the program of the first part that is not `cd`, when it is a
///   bare name (`git`, `make`); a program given as a path names nothing,
///   since a path with a space in it cannot be told from its arguments.
fn command_name(command: &str) -> Option<String> {
    let parts = programs(command);
    if let Some(script) = parts.iter().find_map(|words| script_run(words, 2)) {
        return Some(script);
    }
    let program = parts
        .iter()
        .map(|words| words[0].as_str())
        .find(|program| *program != "cd")?;
    if program.contains('/') {
        return None;
    }
    clean_name(program)
}

/// The parts of a command's first line, each without its leading
/// `NAME=value` words, so each starts with its program.
fn programs(command: &str) -> Vec<Vec<String>> {
    let first_line = command.lines().next().unwrap_or_default();
    split_parts(first_line)
        .into_iter()
        .map(|words| {
            words
                .into_iter()
                .skip_while(|word| is_assignment(word))
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect()
}

/// Programs that run the script named as their first argument.
const INTERPRETERS: [&str; 11] = [
    "python", "node", "bash", "sh", "zsh", "ruby", "perl", "deno", "bun", "tsx", "ts-node",
];

/// Whether `program` (a base name) is an [`INTERPRETERS`] program; a version
/// after `python` (`python3`, `python3.12`) is allowed.
fn is_interpreter(program: &str) -> bool {
    INTERPRETERS.contains(&program)
        || program.strip_prefix("python").is_some_and(|version| {
            version.starts_with(|c: char| c.is_ascii_digit())
                && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
}

/// The script one part (its program first) runs, as [`command_name`] says;
/// `depth` bounds how many nested `-c` strings are read.
fn script_run(words: &[String], depth: usize) -> Option<String> {
    let program = base_name(&words[0]);
    if let Some(script) = script_name(program) {
        return Some(script);
    }
    if !is_interpreter(program) {
        return None;
    }
    let shell = matches!(program, "bash" | "sh" | "zsh");
    let mut rest = words[1..].iter();
    while let Some(word) = rest.next() {
        if word.starts_with('-') {
            if shell && matches!(word.as_str(), "-c" | "-lc") {
                // The command string a shell runs, read the same way, at most
                // `depth` strings deep.
                let inner = rest.next()?;
                let depth = depth.checked_sub(1)?;
                return programs(inner)
                    .iter()
                    .find_map(|words| script_run(words, depth));
            }
            if program.starts_with("python") && word == "-u" {
                continue;
            }
            // An option may take code, a module or a value as its next word.
            // Only skip the standalone flag whose meaning is known here.
            return None;
        }
        return script_name(base_name(word));
    }
    None
}

/// `base` as a script's name: a plain name ending in a script extension.
fn script_name(base: &str) -> Option<String> {
    let (stem, extension) = base.rsplit_once('.')?;
    let plain = !stem.is_empty() && base.chars().all(allowed);
    (plain && SCRIPT_EXTENSIONS.contains(&extension))
        .then(|| clean_name(base))
        .flatten()
}

/// `NAME=value`, a variable set for the command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// A command line's parts (split at `&&`, `||`, `;`, `|` and parentheses),
/// each as its
/// words, with quotes removed. Simple on purpose: it only has to find a name.
fn split_parts(line: &str) -> Vec<Vec<String>> {
    let mut parts = vec![Vec::new()];
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    let end_word = |parts: &mut Vec<Vec<String>>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            parts
                .last_mut()
                .expect("there is always a part")
                .push(std::mem::take(word));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            (None, c) if c.is_whitespace() => end_word(&mut parts, &mut word, &mut in_word),
            (None, ';' | '|' | '&' | '(' | ')') => {
                end_word(&mut parts, &mut word, &mut in_word);
                if matches!(chars.peek(), Some('|' | '&')) {
                    chars.next();
                }
                parts.push(Vec::new());
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    end_word(&mut parts, &mut word, &mut in_word);
    parts.retain(|words| !words.is_empty());
    parts
}

/// The last component of a path, after its last `/`.
fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

/// Whether a name may show this character.
fn allowed(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '@')
}

/// A name as a label shows it: only letters, digits and `._-+@`, at most
/// [`MAX_NAME_CHARS`] characters (a longer one cut, ending in "…"); `None`
/// when nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let kept: Vec<char> = name.chars().filter(|&c| allowed(c)).collect();
    if kept.is_empty() {
        return None;
    }
    if kept.len() <= MAX_NAME_CHARS {
        return Some(kept.into_iter().collect());
    }
    let mut cut: String = kept[..MAX_NAME_CHARS - 1].iter().collect();
    cut.push('…');
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::{MAX_NAME_CHARS, StepCell, step_of_line};
    use serde_json::json;

    fn step(value: &serde_json::Value) -> Option<String> {
        step_of_line(value.to_string().as_bytes())
    }

    fn tool(name: &str, input: serde_json::Value) -> serde_json::Value {
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "toolu_1", "name": name, "input": input}
        ]}})
    }

    fn bash(command: &str) -> Option<String> {
        step(&tool(
            "Bash",
            json!({"command": command, "description": "Run it"}),
        ))
    }

    #[test]
    fn start_and_finish() {
        let init = json!({"type": "system", "subtype": "init", "cwd": "/Users/me/project",
                          "tools": ["Read"], "model": "m"});
        assert_eq!(step(&init).as_deref(), Some("Starting Claude Code"));
        let result = json!({"type": "result", "subtype": "success", "result": "secret text"});
        assert_eq!(step(&result).as_deref(), Some("Finishing"));
        // Another system line does not change the step.
        assert_eq!(step(&json!({"type": "system", "subtype": "status"})), None);
    }

    #[test]
    fn file_tools_name_only_the_base_name() {
        let path = "/Users/me/secret project/notes/CLAUDE.md";
        assert_eq!(
            step(&tool("Read", json!({"file_path": path}))).as_deref(),
            Some("Reading CLAUDE.md")
        );
        assert_eq!(
            step(&tool(
                "Write",
                json!({"file_path": "/tmp/g/result.json", "content": "{\"score\": 1}"})
            ))
            .as_deref(),
            Some("Writing result.json")
        );
        assert_eq!(
            step(&tool(
                "Edit",
                json!({"file_path": "/tmp/g/report.html", "old_string": "a", "new_string": "b"})
            ))
            .as_deref(),
            Some("Editing report.html")
        );
        // No usable name: the verb alone.
        for input in [
            json!({}),
            json!({"file_path": "/tmp/folder/"}),
            json!({"file_path": "/tmp/   "}),
            json!({"file_path": 7}),
        ] {
            assert_eq!(
                step(&tool("Read", input)).as_deref(),
                Some("Reading a file")
            );
        }
    }

    #[test]
    fn search_tools_never_show_their_pattern() {
        assert_eq!(
            step(&tool(
                "Glob",
                json!({"pattern": "**/secret*.md", "path": "/Users/me"})
            ))
            .as_deref(),
            Some("Finding files")
        );
        assert_eq!(
            step(&tool(
                "Grep",
                json!({"pattern": "password", "path": "/Users/me"})
            ))
            .as_deref(),
            Some("Searching files")
        );
    }

    #[test]
    fn other_tools_are_named() {
        assert_eq!(
            step(&tool("WebFetch", json!({"url": "https://example.com"}))).as_deref(),
            Some("Using WebFetch")
        );
        assert_eq!(
            step(&tool("\u{1b}[]!", json!({}))).as_deref(),
            Some("Using a tool")
        );
        // A tool use without input is still a step.
        let bare = json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Read"}
        ]}});
        assert_eq!(step(&bare).as_deref(), Some("Reading a file"));
    }

    #[test]
    fn a_command_is_named_by_its_script() {
        let cases = [
            ("python3 analyze.py", "Running analyze.py"),
            ("python3 -u analyze.py", "Running analyze.py"),
            (
                "python3 /Users/me/skill/scripts/analyze.py --repo /x",
                "Running analyze.py",
            ),
            ("/usr/bin/python3.12 -u analyze.py", "Running analyze.py"),
            (
                "cd /Users/me/project && python3 scripts/collect.py",
                "Running collect.py",
            ),
            ("FOO=1 BAR='x y' node ./build.mjs", "Running build.mjs"),
            ("python3 analyze.py | head -50", "Running analyze.py"),
            ("./analyze.py --all", "Running analyze.py"),
            ("cd /x; scripts/run.sh", "Running run.sh"),
            ("bash -c 'deploy.sh'", "Running deploy.sh"),
            ("bash -lc \"cd /s && python3 a.py\"", "Running a.py"),
            ("sh -c \"bash -c 'zsh -c x.sh'\"", "Running sh"),
            ("ruby x.rb; perl y.pl", "Running x.rb"),
            ("make && python3 later.py", "Running later.py"),
        ];
        for (command, label) in cases {
            assert_eq!(bash(command).as_deref(), Some(label), "{command}");
        }
    }

    #[test]
    fn interpreter_options_never_name_inline_code_or_option_values() {
        let cases = [
            ("python3 -c 'module.secret.py'", "Running python3"),
            ("node -e 'module.secret.js'", "Running node"),
            ("ruby -e 'module.secret.rb'", "Running ruby"),
            ("perl -e 'module.secret.pl'", "Running perl"),
            ("python3 -X private.py analyze.py", "Running python3"),
            ("python3 -W private.py analyze.py", "Running python3"),
            ("python3 -m private.py analyze.py", "Running python3"),
            ("node --require private.js analyze.js", "Running node"),
            ("python3 --unknown private.py analyze.py", "Running python3"),
            ("python3 -u -X private.py analyze.py", "Running python3"),
            ("bash -secretc 'private.py'", "Running bash"),
            ("bash --command 'private.py'", "Running bash"),
        ];
        for (command, label) in cases {
            assert_eq!(bash(command).as_deref(), Some(label), "{command}");
        }
    }

    #[test]
    fn a_file_that_is_only_an_argument_is_not_run() {
        let cases = [
            ("grep -r password deploy.sh", "Running grep"),
            ("echo hi > /x/out.py", "Running echo"),
            ("cat ~/.config/secret_token.sh", "Running cat"),
            ("ls -la /Users/me/project", "Running ls"),
            ("cd /x; LANG=C wc -l *.py", "Running wc"),
            ("python3 -m pytest tests/test_a.py", "Running python3"),
            (
                "python3 - <<'EOF'\nopen('other.py')\nEOF",
                "Running python3",
            ),
            ("bash <<'EOF'\npython3 a.py\nEOF", "Running bash"),
            ("cd '/a b' && bash \"run all.sh\"", "Running bash"),
            ("(cd /x && make)", "Running make"),
        ];
        for (command, label) in cases {
            assert_eq!(bash(command).as_deref(), Some(label), "{command}");
        }
    }

    #[test]
    fn a_program_given_as_a_path_names_nothing() {
        for command in [
            "/Users/me/Secret Folder/run --x",
            "cd /x && /usr/bin/git status --short",
            "./build",
            "cd /x",
            "A=1",
            "",
            "   ",
        ] {
            assert_eq!(
                bash(command).as_deref(),
                Some("Running a command"),
                "{command}"
            );
        }
        assert_eq!(
            step(&tool("Bash", json!({}))).as_deref(),
            Some("Running a command")
        );
    }

    #[test]
    fn the_last_tool_use_of_a_message_counts() {
        let message = json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": "I will read then run."},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "/a/SKILL.md"}},
            {"type": "tool_use", "name": "Bash", "input": {"command": "python3 s/score.py"}},
            {"type": "thinking", "thinking": "hmm"}
        ]}});
        assert_eq!(step(&message).as_deref(), Some("Running score.py"));
    }

    #[test]
    fn lines_without_a_step_change_nothing() {
        let lines: [&[u8]; 9] = [
            b"",
            b"not json",
            b"{\"type\": \"assistant\", \"message\": {\"content\": [",
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"Reading x"}]}}"#,
            br#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"t"}]}}"#,
            br#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            br#"{"type":"stream_event","event":{}}"#,
            br#"["result"]"#,
            br#"{"type":"assistant","message":{"content":"plain"}}"#,
        ];
        for line in lines {
            assert_eq!(
                step_of_line(line),
                None,
                "{}",
                String::from_utf8_lossy(line)
            );
        }
        assert_eq!(step_of_line(&[0xff, 0xfe, b'{']), None);
    }

    #[test]
    fn names_are_cleaned_and_cut() {
        let long = format!("{}.py", "a".repeat(60));
        let label = bash(&format!("python3 {long}")).unwrap();
        let name = label.strip_prefix("Running ").unwrap();
        assert_eq!(name.chars().count(), MAX_NAME_CHARS);
        assert!(name.ends_with('…'));
        assert!(name.starts_with("aaaa"));
        // Control characters and odd symbols are dropped from a name.
        assert_eq!(
            step(&tool(
                "Read",
                json!({"file_path": "/x/re\u{1b}[2Jport\u{202e}.md"})
            ))
            .as_deref(),
            Some("Reading re2Jport.md")
        );
        assert_eq!(
            step(&tool("Read", json!({"file_path": "/x/my notes (1).md"}))).as_deref(),
            Some("Reading mynotes1.md")
        );
        assert_eq!(
            step(&tool("Read", json!({"file_path": "/x/naïve_v1+2@x.txt"}))).as_deref(),
            Some("Reading naïve_v1+2@x.txt")
        );
    }

    #[test]
    fn no_path_or_command_text_leaks_into_a_label() {
        let secrets = ["/Users/me", "secret", "password", "--token", "sk-"];
        let lines = [
            tool(
                "Bash",
                json!({"command": "cd /Users/me/secret && TOKEN=sk-1 ./run.sh --token x"}),
            ),
            tool(
                "Bash",
                json!({"command": "curl -H 'password: x' https://secret.example"}),
            ),
            tool("Read", json!({"file_path": "/Users/me/secret/a.md"})),
            tool(
                "Grep",
                json!({"pattern": "password", "path": "/Users/me/secret"}),
            ),
            tool(
                "Write",
                json!({"file_path": "/Users/me/x.md", "content": "password"}),
            ),
        ];
        for line in &lines {
            let label = step(line).unwrap();
            for secret in secrets {
                assert!(!label.contains(secret), "{label} holds {secret}");
            }
        }
        assert_eq!(step(&lines[0]).as_deref(), Some("Running run.sh"));
        assert_eq!(step(&lines[1]).as_deref(), Some("Running curl"));
    }

    #[test]
    fn the_cell_keeps_the_latest_step() {
        let cell = StepCell::new();
        assert_eq!(cell.get(), None);
        cell.observe(
            json!({"type": "system", "subtype": "init"})
                .to_string()
                .as_bytes(),
        );
        assert_eq!(cell.get().as_deref(), Some("Starting Claude Code"));
        cell.observe(b"garbage");
        cell.observe(br#"{"type":"user","message":{"content":[]}}"#);
        assert_eq!(cell.get().as_deref(), Some("Starting Claude Code"));
        let shared = cell.clone();
        shared.observe(tool("Glob", json!({"pattern": "*"})).to_string().as_bytes());
        assert_eq!(cell.get().as_deref(), Some("Finding files"));
    }
}
