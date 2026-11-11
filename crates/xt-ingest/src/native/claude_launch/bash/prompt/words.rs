//! Finding the simple commands of one recorded `Bash` command, for the
//! literal-prompt launch match. It never runs or expands anything.
//!
//! The reviewed readers ([`super::super::script`], the restricted shell
//! reader) prove a launch from the command alone, so they accept only a
//! closed grammar and refuse the rest; an agent's real commands (variables,
//! redirections to expanded paths, here-documents, loops, pipes) fall outside
//! it. This reader proves nothing by itself: it only finds where commands
//! start and which of their words are literal. The launch it finds is then
//! read by the shared option map ([`super::super::super::options`]) and
//! proved by its child's first input.
//!
//! A command is split into simple commands at `;`, `&`, `&&`, `|`, `||`,
//! `|&`, newlines, and parentheses. A word is plain characters,
//! `'…'`, `"…"` (where `\` escapes only `$`, `` ` ``, `"`, `\` and a newline)
//! and `\`-escaped characters; it is literal unless it holds an expansion
//! (`$NAME`, `${…}`, `$(…)`, `` `…` ``, `$((…))`, `$'…'`, `$"…"`), a glob
//! character, a brace expansion, a process substitution or a leading `~`
//! (kept apart, for a program path). A word that is not literal is one word
//! ([`Word::Opaque`]) only when the shell cannot split it: its expansions are
//! quoted (but `"$@"`, a `"${…}"` holding `@` or another `$` or starting
//! with a zsh flag such as `(f)` or `=`, a `"$NAME[…]"` subscript and zsh's
//! `$=`, `$~` and `$^`), `$'…'`, `$"…"`, a tilde,
//! a process substitution, or a `$NAME` or `${NAME}` of a variable the
//! command set to a plain value at its start and never touches again, and
//! that no command of it may set without naming it ([`one_word_names`],
//! [`sets_unnamed`]); any other may become no word or several
//! ([`Word::Fields`]). A redirection and its target are set aside; a
//! here-document's body is skipped, never read as commands, also inside a
//! substitution. A command or process substitution's (`<(…)`, `>(…)`, zsh's
//! `=(…)`) body
//! is read as commands too, up to a small depth. Comments and array literals
//! (`x=( … )`) are skipped. An
//! unterminated quote, substitution or here-document, a function definition,
//! or an array literal holding a substitution refuses the whole command.
//!
//! Text the shell runs but this reader does not read as commands refuses the
//! whole command when it names the Claude program ([`names_claude`]): a
//! parameter expansion's body (`${V:-$(…)}`), an arithmetic expansion's
//! (`$((…))`, `$[…]`), the substitutions of a here-document whose delimiter
//! is not quoted, and a substitution or `sh -c` script deeper than the depth
//! read. Text a command is given as data — a here-document body or
//! here-string, the literal parts of a word holding an expansion, `$'…'`,
//! an array's words — is kept with that command ([`Command::data`]), for the
//! launch reader to judge by the program that is given it.
//!
//! Each simple command says whether it reads only the input the whole
//! command was given ([`Command::own_input`]): not when it, or a group or
//! loop around it, has a `<` redirection or a here-document, or follows a `|`
//! in a pipeline or an `exec` that redirects the script's input, or is inside a process substitution (whose input is what
//! the command writes to it, or what the command reads), or inside a
//! substitution or `sh -c` script of a command that does not.

use std::collections::HashSet;

/// Longest command read; a longer one is refused.
pub(super) const MAX_COMMAND: usize = 256 * 1024;
/// How deep substitutions and `sh -c` scripts are read.
const MAX_DEPTH: usize = 3;
/// Simple commands one command may hold.
const MAX_COMMANDS: usize = 4096;

/// One word of a simple command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Word {
    /// Literal text, quotes removed.
    Lit(String),
    /// A literal path after an unexpanded leading `~/`.
    Home(String),
    /// A word the shell supplies only when it runs, always as exactly one
    /// word.
    Opaque,
    /// A word the shell supplies when it runs that may become no word or
    /// several: an unquoted expansion, glob or brace expansion, or `"$@"`.
    Fields,
    /// A word that starts `NAME=`: an assignment before the program, and
    /// its text when it is literal.
    Assign(Option<String>),
}

/// The words of one simple command, redirections set aside.
pub(super) type Simple = Vec<Word>;

/// One simple command and where its input comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Command {
    pub(super) words: Simple,
    /// It reads only the input the whole recorded command was given: no `<`
    /// redirection or here-document of its own or of a group or loop around
    /// it, not after a `|`, not inside a process substitution (`<(…)`,
    /// `>(…)`), and not inside a substitution or `sh -c` script of a command
    /// that does read other input.
    pub(super) own_input: bool,
    /// Text the command is given that is in none of its literal words and
    /// was not read as commands: its here-document bodies and here-strings,
    /// the literal parts of its words holding an expansion, `$'…'` and array
    /// words.
    pub(super) data: Vec<String>,
}

/// Whether `text` names the Claude program as a command would: a word
/// `claude`, or a path ending in `/claude`, between spaces, shell operators,
/// quotes, braces, `$`, `=`, `:` or `\`, also after a parameter expansion's
/// `-`, `+` or `?` (`${C:-claude}`). `~/.claude`, `claude-code` and `Claude`
/// do not. This is the one test of whether text a reader skips or takes as
/// data could hold a Claude launch.
pub(super) fn names_claude(text: &str) -> bool {
    /// What a word naming the program may stand between, besides spaces.
    const AROUND: &str = ";&|()<>'\"`{}$=:\\";
    text.split(|c: char| c.is_whitespace() || AROUND.contains(c))
        .map(|token| token.trim_start_matches(['-', '+', '?']))
        .any(|token| token == "claude" || token.ends_with("/claude"))
}

/// Shell words that open a compound command; one after a `|` reads the pipe.
const COMPOUND: &[&str] = &["{", "while", "until", "for", "if", "case", "select"];
/// Shell words that close one; a `<` after it feeds the whole group.
const CLOSERS: &[&str] = &["}", "done", "fi", "esac"];

/// Every simple command `command` holds, in order, with those in
/// substitutions after the command that holds them; `None` when it cannot be
/// split.
pub(super) fn simple_commands(command: &str) -> Option<Vec<Command>> {
    if command.len() > MAX_COMMAND {
        return None;
    }
    let known = one_word_names(command);
    let mut out = Vec::new();
    split(command, 0, true, &known, &mut out)?;
    // A command that may set a variable without naming it in its text is
    // read again with no variable known to be one word.
    if !known.is_empty() && out.iter().any(|command| sets_unnamed(&command.words)) {
        out.clear();
        split(command, 0, true, &HashSet::new(), &mut out)?;
    }
    Some(out)
}

/// Builtins that may set or unset a variable whose name the text does not
/// spell: one given by an expansion (`read "$N"`, `declare "$N=a b"`), or
/// in text they run (`eval`, `source`, `.`, `trap`).
const SETS_NAMES: &[&str] = &[
    "eval",
    "source",
    ".",
    "read",
    "declare",
    "typeset",
    "local",
    "export",
    "readonly",
    "unset",
    "mapfile",
    "readarray",
    "getopts",
    "let",
    "builtin",
    "trap",
    // zsh, its precommand modifiers among them.
    "noglob",
    "nocorrect",
    "integer",
    "float",
    "vared",
    "zparseopts",
];

/// Whether a simple command may set a variable without naming it in the
/// text ([`one_word_names`]): its program is one of [`SETS_NAMES`],
/// `printf` or `print` given `-v`, `wait` given `-p` (or a word that may be
/// such an option), a program the
/// shell supplies (`$CMD …`), or one this reader cannot find (`timeout`
/// with no literal duration).
fn sets_unnamed(words: &[Word]) -> bool {
    let Some((program, args)) = program(words) else {
        return words
            .iter()
            .any(|word| matches!(word, Word::Lit(text) if text == "timeout"));
    };
    let name = match program {
        Word::Lit(name) => name.as_str(),
        Word::Opaque | Word::Fields => return true,
        Word::Home(_) | Word::Assign(_) => return false,
    };
    if SETS_NAMES.contains(&name) {
        return true;
    }
    let flag = match name {
        "printf" | "print" => 'v',
        "wait" => 'p',
        _ => return false,
    };
    // Its options come before its first other word.
    for word in args {
        match word {
            Word::Lit(text) if text == "--" || !text.starts_with('-') => return false,
            Word::Lit(text) if text.contains(flag) => return true,
            Word::Lit(_) => {}
            _ => return true,
        }
    }
    false
}

/// Variables that cannot be read here with a name the shell may still
/// change or that are not plain values.
const UNSAFE_NAMES: &[&str] = &[
    "IFS",
    "_",
    "PWD",
    "OLDPWD",
    "REPLY",
    "OPTARG",
    "OPTIND",
    "RANDOM",
    "SRANDOM",
    "SECONDS",
    "LINENO",
    "HISTCMD",
    "EPOCHSECONDS",
    "EPOCHREALTIME",
    "BASHPID",
    "PPID",
    "UID",
    "EUID",
    "PATH",
    "HOME",
    // An assignment bash 3.2 ignores, and zsh's directory stack.
    "DIRSTACK",
    "dirstack",
];

/// The variables whose unquoted `$NAME` or `${NAME}` the shell supplies as
/// exactly one word in `command`'s own script: those the command sets, in a
/// first simple command of nothing but assignments (`S=/w/run; …`), to a
/// plain value of characters the shell never splits or expands, and never
/// names again except as `$NAME` or `${NAME}`. None when the command holds
/// the text `IFS`; and [`simple_commands`] reads the command again with
/// none when one of its commands may set a variable without naming it
/// ([`sets_unnamed`]).
fn one_word_names(command: &str) -> HashSet<String> {
    let mut known = HashSet::new();
    if command.contains("IFS") {
        return known;
    }
    let Some((assigned, rest)) = leading_assignments(command) else {
        return known;
    };
    let mut seen = HashSet::new();
    // The last assignment of a name is the value it keeps.
    for (name, value) in assigned.into_iter().rev() {
        if !seen.insert(name) {
            continue;
        }
        if !value.is_empty()
            && value.chars().all(one_word_char)
            && !UNSAFE_NAMES.contains(&name)
            && !name.starts_with("BASH")
            && only_expanded(rest, name)
        {
            known.insert(name.to_owned());
        }
    }
    known
}

/// A character a variable's value may hold for its unquoted expansion to be
/// one word: never one the shell splits at or expands.
fn one_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "/._-:+,=@%".contains(c)
}

/// The `NAME=VALUE` assignments of the simple command `command` starts with,
/// after optional spaces or tabs, when it holds nothing else and ends at `;`,
/// `&&`, a newline or the end; and the text after it. A value is plain
/// [`one_word_char`]s, or one `'…'`, or one `"…"` holding no `$`, `` ` `` or
/// `\`; anything else reads no assignment at all.
fn leading_assignments(command: &str) -> Option<(Vec<(&str, &str)>, &str)> {
    let mut rest = command;
    let mut assigned = Vec::new();
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest
            .strip_prefix(';')
            .or_else(|| rest.strip_prefix('\n'))
            .or_else(|| rest.strip_prefix("&&"))
        {
            rest = after;
            break;
        }
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty()
            || name.starts_with(|c: char| c.is_ascii_digit())
            || !rest[name_end..].starts_with('=')
        {
            return None;
        }
        let after = &rest[name_end + 1..];
        let (value, end) = match after.chars().next() {
            Some(quote @ ('\'' | '"')) => {
                let close = after[1..].find(quote)? + 1;
                let value = &after[1..close];
                if quote == '"' && value.contains(['$', '`', '\\']) {
                    return None;
                }
                (value, close + 1)
            }
            _ => {
                let end = after
                    .find(|c: char| !one_word_char(c))
                    .unwrap_or(after.len());
                (&after[..end], end)
            }
        };
        // A value ends at a space, a tab or what ends the command.
        let tail = &after[end..];
        if !(tail.is_empty() || tail.starts_with([' ', '\t', '\n', ';', '&'])) {
            return None;
        }
        assigned.push((name, value));
        rest = tail;
    }
    (!assigned.is_empty()).then_some((assigned, rest))
}

/// Whether every identifier `name` in `text` (a whole run of letters,
/// digits and `_`) is `$name` or `${name}`.
fn only_expanded(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if !ident(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && ident(bytes[i]) {
            i += 1;
        }
        if &bytes[start..i] != name.as_bytes() {
            continue;
        }
        let before = &bytes[..start];
        let plain = before.ends_with(b"$");
        let braced = before.ends_with(b"${") && bytes.get(i) == Some(&b'}');
        if !(plain || braced) {
            return false;
        }
    }
    true
}

/// The script an `sh -c`, `bash -c` or `zsh -c` simple command runs, when it
/// is one literal word.
pub(super) fn shell_script(words: &[Word]) -> Option<&str> {
    match words {
        [Word::Lit(shell), Word::Lit(flag), Word::Lit(script), ..]
            if matches!(
                shell.as_str(),
                "sh" | "bash" | "zsh" | "/bin/sh" | "/bin/bash" | "/bin/zsh"
            ) && matches!(flag.as_str(), "-c" | "-lc") =>
        {
            Some(script)
        }
        _ => None,
    }
}

/// Read `script` as commands into `out`, then the substitutions it holds.
/// `inherited` says whether the script itself reads only the input the whole
/// recorded command was given; `known` holds the variables whose unquoted
/// expansion is one word in it ([`one_word_names`]).
fn split(
    script: &str,
    depth: usize,
    inherited: bool,
    known: &HashSet<String>,
    out: &mut Vec<Command>,
) -> Option<()> {
    let chars: Vec<char> = script.chars().collect();
    let mut reader = Reader {
        chars: &chars,
        known,
        i: 0,
        commands: Vec::new(),
        current: Vec::new(),
        word: None,
        target: None,
        heredocs: Vec::new(),
        subs: Vec::new(),
        current_subs: Vec::new(),
        current_data: Vec::new(),
        redirected: false,
        piped: false,
        closer: false,
        group_input: false,
        refused: false,
    };
    reader.run()?;
    let Reader {
        commands,
        subs,
        group_input,
        refused,
        ..
    } = reader;
    if refused {
        return None;
    }
    let inherited = inherited && !group_input;
    for mut command in commands {
        if out.len() >= MAX_COMMANDS {
            return None;
        }
        command.own_input &= inherited;
        let own = command.own_input;
        let nested = shell_script(&command.words).map(str::to_owned);
        out.push(command);
        if let Some(nested) = nested {
            // A new shell, which a variable this one set is not exported to.
            nest(&nested, depth, own, &HashSet::new(), out)?;
        }
    }
    // A substitution runs in a copy of this shell, with its variables.
    for (body, own) in subs {
        nest(&body, depth, inherited && own, known, out)?;
    }
    Some(())
}

/// Read a substitution's body or an `sh -c` script found at `depth` as
/// commands, while it is not too deep; one too deep that names `claude`
/// refuses the command.
fn nest(
    script: &str,
    depth: usize,
    inherited: bool,
    known: &HashSet<String>,
    out: &mut Vec<Command>,
) -> Option<()> {
    if depth < MAX_DEPTH {
        split(script, depth + 1, inherited, known, out)
    } else if names_claude(script) {
        None
    } else {
        Some(())
    }
}

/// A word being read.
#[derive(Debug, Default)]
struct Building {
    text: String,
    literal: bool,
    /// The leading characters read unquoted and literal, until the first
    /// that is not: what an assignment or a keyword is told by.
    bare: String,
    plain: bool,
    /// An unquoted `{` or `}` was read.
    brace: bool,
    /// It starts with an unquoted `~`.
    tilde: bool,
    /// Part of it was quoted: as a here-document's delimiter, the body is
    /// then text the shell does not expand.
    quoted: bool,
    /// The shell may turn it into no word or several ([`Word::Fields`]).
    splits: bool,
}

impl Building {
    fn new() -> Self {
        Self {
            literal: true,
            plain: true,
            ..Self::default()
        }
    }

    fn push(&mut self, c: char, quoted: bool) {
        if quoted {
            self.plain = false;
            self.quoted = true;
        } else if self.plain {
            self.bare.push(c);
        }
        self.text.push(c);
    }

    fn opaque(&mut self) {
        self.literal = false;
        self.plain = false;
    }

    /// Part of it the shell supplies, which may split it.
    fn fields(&mut self) {
        self.opaque();
        self.splits = true;
    }

    fn finish(self) -> Word {
        let braced = self.brace && self.text != "{" && self.text != "}";
        if is_assignment(&self.bare) {
            return Word::Assign((self.literal && !braced && !self.tilde).then_some(self.text));
        }
        if self.splits || braced {
            return Word::Fields;
        }
        if !self.literal {
            return Word::Opaque;
        }
        if self.tilde {
            return match self.text.strip_prefix("~/") {
                Some(rest) if self.bare.starts_with("~/") => Word::Home(rest.to_owned()),
                _ => Word::Opaque,
            };
        }
        Word::Lit(self.text)
    }
}

/// Whether the unquoted start of a word is `NAME=`.
fn is_assignment(bare: &str) -> bool {
    let Some((name, _)) = bare.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a word read so far is exactly `NAME=` or `NAME+=`: a `(` right
/// after it opens an array literal, whose words are not commands.
fn array_start(word: &Building) -> bool {
    word.plain
        && word.text == word.bare
        && word.bare.strip_suffix('=').is_some_and(|name| {
            let name = name.strip_suffix('+').unwrap_or(name);
            is_assignment(&format!("{name}="))
        })
}

/// What the next word ends up as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// A redirection's file: set aside.
    Redirect,
    /// A here-string's word: input the command is given, kept as data.
    HereString,
    /// A here-document's delimiter; `true` strips leading tabs.
    Heredoc(bool),
}

/// A here-document whose body is still to come.
#[derive(Debug)]
struct Heredoc {
    delimiter: String,
    /// Leading tabs are stripped (`<<-`).
    strip: bool,
    /// The delimiter was quoted, so the body is not expanded.
    quoted: bool,
    /// The index the command it was opened by has among the commands read,
    /// or `None` when that command turned out empty.
    owner: Option<usize>,
}

struct Reader<'a> {
    chars: &'a [char],
    /// The variables whose unquoted expansion is one word here.
    known: &'a HashSet<String>,
    i: usize,
    commands: Vec<Command>,
    current: Simple,
    word: Option<Building>,
    target: Option<Target>,
    heredocs: Vec<Heredoc>,
    /// Substitution bodies, and whether the command holding each reads only
    /// its own input.
    subs: Vec<(String, bool)>,
    /// The substitutions of the command being read, and whether each body
    /// reads the input the command does: not a process substitution's.
    current_subs: Vec<(String, bool)>,
    /// The data text of the command being read ([`Command::data`]).
    current_data: Vec<String>,
    /// The command being read has a `<` redirection or a here-document.
    redirected: bool,
    /// A `|` came before the command being read.
    piped: bool,
    /// The command being read follows a `)`: its redirections are the
    /// group's.
    closer: bool,
    /// A group or loop of this script reads a pipe or a redirection, so no
    /// command in it is known to read only its own input.
    group_input: bool,
    /// Text that is not run as it stands was found: a function definition,
    /// or text the shell runs that this reader does not read, naming
    /// `claude`.
    refused: bool,
}

impl Reader<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).copied()
    }

    fn word(&mut self) -> &mut Building {
        self.word.get_or_insert_with(Building::new)
    }

    fn end_word(&mut self) {
        let Some(building) = self.word.take() else {
            return;
        };
        match self.target.take() {
            Some(Target::Heredoc(strip)) => self.heredocs.push(Heredoc {
                delimiter: building.text,
                strip,
                quoted: building.quoted,
                owner: Some(self.commands.len()),
            }),
            Some(Target::Redirect) => {}
            Some(Target::HereString) => self.current_data.push(building.text),
            None => {
                let text = building.text.clone();
                let word = building.finish();
                // What a word holding an expansion says literally is data.
                if matches!(word, Word::Opaque | Word::Fields | Word::Assign(None)) {
                    self.current_data.push(text);
                }
                self.current.push(word);
            }
        }
    }

    /// Refuse the command when text the shell runs, which this reader does
    /// not read, names `claude`.
    fn unread(&mut self, start: usize, end: usize) {
        if names_claude(&self.chars[start..end].iter().collect::<String>()) {
            self.refused = true;
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        self.target = None;
        let first = match self.current.first() {
            Some(Word::Lit(first)) => Some(first.as_str()),
            _ => None,
        };
        // A redirection of a group's closing word feeds the whole group, and
        // a group after a `|` reads the pipe.
        if self.redirected && (self.closer || first.is_some_and(|word| CLOSERS.contains(&word))) {
            self.group_input = true;
        }
        // `exec < f` (also `builtin exec < f`), with no program, makes the
        // file the input of every later command of the script.
        let builtin = Word::Lit("builtin".to_owned());
        let unwrapped: Vec<Word> = self
            .current
            .iter()
            .filter(|word| **word != builtin)
            .cloned()
            .collect();
        if self.redirected
            && program(&unwrapped).is_none()
            && unwrapped.contains(&Word::Lit("exec".to_owned()))
        {
            self.group_input = true;
        }
        let piped = self.piped && !self.current.is_empty();
        if piped && first.is_some_and(|word| COMPOUND.contains(&word)) {
            self.group_input = true;
        }
        // `function name …` defines a function: its body is not run here.
        if first == Some("function") {
            self.refused = true;
        }
        let own = !self.redirected && !piped;
        for (body, reads) in self.current_subs.drain(..) {
            self.subs.push((body, own && reads));
        }
        if self.current.is_empty() {
            // A here-document opened by no command is given to none.
            let index = self.commands.len();
            for heredoc in &mut self.heredocs {
                if heredoc.owner == Some(index) {
                    heredoc.owner = None;
                }
            }
            if self.current_data.drain(..).any(|text| names_claude(&text)) {
                self.refused = true;
            }
        } else {
            self.commands.push(Command {
                words: std::mem::take(&mut self.current),
                own_input: own,
                data: std::mem::take(&mut self.current_data),
            });
            self.piped = false;
        }
        self.redirected = false;
        self.closer = false;
    }

    /// End the command at a separator that is not a pipe.
    fn end_list(&mut self) {
        self.end_command();
        self.piped = false;
    }

    fn run(&mut self) -> Option<()> {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\r' => {
                    self.end_word();
                    self.i += 1;
                }
                '\n' => {
                    // A line ending after a `|` continues the pipeline.
                    self.end_command();
                    self.i += 1;
                    self.skip_heredocs();
                }
                '#' if self.word.is_none() => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '(' if self.word.as_ref().is_some_and(array_start) => {
                    // `x=( … )`: an array's words, never run, unless it holds
                    // a substitution, which this reader does not read there.
                    let end = close(self.chars, self.i + 1, '(', ')', false)?;
                    let body = &self.chars[self.i + 1..end];
                    if body.iter().any(|&c| matches!(c, '$' | '`' | '<' | '>')) {
                        return None;
                    }
                    self.current_data.push(body.iter().collect());
                    self.word().opaque();
                    self.i = end + 1;
                }
                '(' => {
                    // `name ()` defines a function, whose body is not run here.
                    let mut next = self.i + 1;
                    while matches!(self.chars.get(next), Some(' ' | '\t')) {
                        next += 1;
                    }
                    if self.chars.get(next) == Some(&')') {
                        return None;
                    }
                    // A subshell after a `|` reads the pipe.
                    if self.piped && self.current.is_empty() && self.word.is_none() {
                        self.group_input = true;
                    }
                    self.end_list();
                    self.i += 1;
                }
                ')' => {
                    self.end_list();
                    self.closer = true;
                    self.i += 1;
                }
                ';' => {
                    self.end_list();
                    self.i += 1;
                }
                '&' => match self.peek(1) {
                    Some('&') => {
                        self.end_list();
                        self.i += 2;
                    }
                    Some('>') => {
                        self.end_word();
                        self.i += 2;
                        if self.peek(0) == Some('>') {
                            self.i += 1;
                        }
                        self.target = Some(Target::Redirect);
                    }
                    _ => {
                        self.end_list();
                        self.i += 1;
                    }
                },
                '|' => {
                    let or = self.peek(1) == Some('|');
                    self.end_list();
                    // `|` and `|&` feed the next command; `||` does not.
                    self.piped = !or;
                    self.i += if matches!(self.peek(1), Some('|' | '&')) {
                        2
                    } else {
                        1
                    };
                }
                '<' | '>' if self.peek(1) == Some('(') => self.process_substitution()?,
                // zsh's `=(…)`, at the start of a word: a temporary file the
                // body writes, run like `<(…)`.
                '=' if self.word.is_none() && self.peek(1) == Some('(') => {
                    self.process_substitution()?;
                }
                '<' | '>' => self.redirection(c),
                '\'' => {
                    self.i += 1;
                    let start = self.i;
                    while self.peek(0)? != '\'' {
                        self.i += 1;
                    }
                    let text: String = self.chars[start..self.i].iter().collect();
                    self.i += 1;
                    let word = self.word();
                    for c in text.chars() {
                        word.push(c, true);
                    }
                    // `''` is a word even when empty.
                    word.plain = false;
                    word.quoted = true;
                }
                '"' => {
                    self.i += 1;
                    self.double_quoted()?;
                }
                '\\' => match self.peek(1) {
                    Some('\n') => self.i += 2,
                    Some(next) => {
                        self.word().push(next, true);
                        self.i += 2;
                    }
                    None => {
                        self.word().push('\\', true);
                        self.i += 1;
                    }
                },
                '$' => match self.peek(1) {
                    Some('\'') => {
                        // `$'…'`: escapes this reader does not decode; its
                        // text is kept as data.
                        self.word().opaque();
                        self.i += 2;
                        loop {
                            let c = self.peek(0)?;
                            match c {
                                '\\' => {
                                    let next = self.peek(1)?;
                                    let word = self.word();
                                    word.push(c, true);
                                    word.push(next, true);
                                    self.i += 2;
                                }
                                '\'' => break,
                                _ => {
                                    self.word().push(c, true);
                                    self.i += 1;
                                }
                            }
                        }
                        self.i += 1;
                    }
                    Some('"') => {
                        self.word().opaque();
                        self.i += 2;
                        self.double_quoted()?;
                    }
                    _ => self.expansion(false)?,
                },
                '`' => self.backquote(false)?,
                '*' | '?' | '[' => {
                    let word = self.word();
                    word.push(c, false);
                    word.fields();
                    self.i += 1;
                }
                '~' if self.word.is_none() => {
                    let word = self.word();
                    word.tilde = true;
                    word.push(c, false);
                    self.i += 1;
                }
                '{' | '}' => {
                    let word = self.word();
                    word.brace = true;
                    word.push(c, false);
                    self.i += 1;
                }
                _ => {
                    self.word().push(c, false);
                    self.i += 1;
                }
            }
        }
        self.end_command();
        Some(())
    }

    /// `<(…)`, `>(…)` or zsh's `=(…)`, at its first character: one word, a
    /// path the body reads from or writes to. The body never reads only the
    /// whole command's input: it may read what the command writes to it, or
    /// what the command reads (a pipe into it).
    fn process_substitution(&mut self) -> Option<()> {
        let start = self.i + 2;
        let end = close(self.chars, start, '(', ')', true)?;
        self.current_subs
            .push((self.chars[start..end].iter().collect(), false));
        self.word().opaque();
        self.i = end + 1;
        Some(())
    }

    /// `<`, `>` and their forms, after an optional file-descriptor number.
    fn redirection(&mut self, c: char) {
        // A word of digits right before it names a descriptor.
        if self.word.as_ref().is_some_and(|word| {
            word.literal
                && word.plain
                && !word.text.is_empty()
                && word.text.bytes().all(|b| b.is_ascii_digit())
        }) {
            self.word = None;
        } else {
            self.end_word();
        }
        // Any `<` form gives the command other input: a file, a here-string,
        // a here-document or another descriptor.
        if c == '<' {
            self.redirected = true;
        }
        if c == '<' && self.peek(1) == Some('<') {
            if self.peek(2) == Some('<') {
                self.i += 3;
                self.target = Some(Target::HereString);
            } else {
                self.i += 2;
                let strip = self.peek(0) == Some('-');
                if strip {
                    self.i += 1;
                }
                self.target = Some(Target::Heredoc(strip));
            }
            return;
        }
        self.i += 1;
        let duplicate = self.peek(0) == Some('&');
        if matches!(self.peek(0), Some('>' | '|' | '&')) || (c == '<' && self.peek(0) == Some('>'))
        {
            self.i += 1;
        }
        if duplicate {
            // `>&2`, `<&0`, `>&-`: no file.
            let start = self.i;
            while self
                .peek(0)
                .is_some_and(|next| next.is_ascii_digit() || next == '-')
            {
                self.i += 1;
            }
            if self.i > start {
                return;
            }
        }
        self.target = Some(Target::Redirect);
    }

    /// The rest of a `"…"` word, after its opening quote.
    fn double_quoted(&mut self) -> Option<()> {
        let word = self.word();
        word.plain = false;
        word.quoted = true;
        loop {
            match self.peek(0)? {
                '"' => {
                    self.i += 1;
                    return Some(());
                }
                '\\' => match self.peek(1)? {
                    '\n' => self.i += 2,
                    next @ ('$' | '`' | '"' | '\\') => {
                        self.word().push(next, true);
                        self.i += 2;
                    }
                    _ => {
                        self.word().push('\\', true);
                        self.i += 1;
                    }
                },
                '$' => self.expansion(true)?,
                '`' => self.backquote(true)?,
                c => {
                    self.word().push(c, true);
                    self.i += 1;
                }
            }
        }
    }

    /// One `$` expansion, at `$`. A `$` that starts none is literal. Unquoted,
    /// every expansion may split its word, but a `$NAME` or `${NAME}` of a
    /// [`Self::known`] variable; quoted, only `"$@"`, a `${…}` holding `@`
    /// (`"${a[@]}"`, `"${@:2}"`) or another expansion (`"${U:-$=M}"`) or
    /// starting with a zsh flag (`"${(f)M}"`, `"${=M}"`, `"${~M}"`,
    /// `"${^M}"`), a `"$NAME[…]"` subscript and zsh's `$=M`, `$~M` and `$^M`
    /// may.
    fn expansion(&mut self, quoted: bool) -> Option<()> {
        let splits = match self.peek(1) {
            Some('(') => {
                let arithmetic = self.peek(2) == Some('(');
                let start = self.i + 2;
                let end = close(self.chars, start, '(', ')', !arithmetic)?;
                if arithmetic {
                    // The shell runs the substitutions an arithmetic
                    // expansion holds; this reader does not read them.
                    self.unread(start, end);
                } else {
                    self.current_subs
                        .push((self.chars[start..end].iter().collect(), true));
                }
                self.i = end + 1;
                !quoted
            }
            Some('{') => {
                // `${V:-$(…)}`: the shell may run what a parameter
                // expansion holds; this reader does not read it.
                let end = close(self.chars, self.i + 2, '{', '}', false)?;
                self.unread(self.i + 2, end);
                let body: String = self.chars[self.i + 2..end].iter().collect();
                self.i = end + 1;
                if quoted {
                    // zsh's flags (`"${(f)M}"`, `"${=M}"`, `"${~M}"`) split
                    // it too, also in a default (`"${U:-$=M}"`).
                    body.contains(['@', '$']) || body.starts_with(['(', '=', '~', '^'])
                } else {
                    !self.known.contains(&body)
                }
            }
            Some('[') => {
                // `$[…]`, the old arithmetic form.
                let end = close(self.chars, self.i + 2, '[', ']', false)?;
                self.unread(self.i + 2, end);
                self.i = end + 1;
                !quoted
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let start = self.i + 1;
                self.i += 1;
                while self
                    .peek(0)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.i += 1;
                }
                let name: String = self.chars[start..self.i].iter().collect();
                // zsh reads `"$M[@]"` as every element of an array.
                if quoted {
                    self.peek(0) == Some('[')
                } else {
                    !self.known.contains(&name)
                }
            }
            Some('=' | '~' | '^') => {
                // zsh's `$=M`, `$~M` and `$^M`: split, globbed or expanded
                // over an array, also quoted (`"$=M"`).
                self.i += 2;
                true
            }
            Some(c)
                if c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
            {
                self.i += 2;
                !quoted || c == '@'
            }
            _ => {
                self.word().push('$', quoted);
                self.i += 1;
                return Some(());
            }
        };
        if splits {
            self.word().fields();
        } else {
            self.word().opaque();
        }
        Some(())
    }

    /// One `` `…` `` substitution, at its opening backquote; unquoted, it may
    /// split its word.
    fn backquote(&mut self, quoted: bool) -> Option<()> {
        self.i += 1;
        let mut body = String::new();
        loop {
            match self.peek(0)? {
                '`' => break,
                '\\' => {
                    let next = self.peek(1)?;
                    if !matches!(next, '`' | '\\' | '$') {
                        body.push('\\');
                    }
                    body.push(next);
                    self.i += 2;
                }
                c => {
                    body.push(c);
                    self.i += 1;
                }
            }
        }
        self.i += 1;
        self.current_subs.push((body, true));
        if quoted {
            self.word().opaque();
        } else {
            self.word().fields();
        }
        Some(())
    }

    /// Skip the bodies of the here-documents the line just ended opened,
    /// keeping each as data of the command that opened it. The shell runs
    /// the substitutions of a body whose delimiter is not quoted; one naming
    /// `claude` refuses the command, since this reader does not read them as
    /// commands.
    fn skip_heredocs(&mut self) {
        for heredoc in std::mem::take(&mut self.heredocs) {
            let body_start = self.i;
            let mut body_end = self.chars.len();
            while self.i < self.chars.len() {
                let start = self.i;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                let line: String = self.chars[start..self.i].iter().collect();
                // Past the newline, if any.
                self.i = (self.i + 1).min(self.chars.len());
                let line = if heredoc.strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == heredoc.delimiter {
                    body_end = start;
                    break;
                }
            }
            let body = &self.chars[body_start..body_end];
            if !heredoc.quoted && substitutes_claude(body) {
                self.refused = true;
            }
            let text: String = body.iter().collect();
            match heredoc.owner.and_then(|owner| self.commands.get_mut(owner)) {
                Some(command) => command.data.push(text),
                None if names_claude(&text) => self.refused = true,
                None => {}
            }
        }
    }
}

/// Whether an unquoted here-document body holds a command substitution
/// (`$(…)` or `` `…` ``) naming `claude` ([`names_claude`]). One that never
/// ends counts when the rest of the body names `claude`.
fn substitutes_claude(body: &[char]) -> bool {
    let names = |text: &[char]| names_claude(&text.iter().collect::<String>());
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            // `\$`, `\`` and `\\` are literal in the body.
            '\\' => i += 1,
            '$' if body.get(i + 1) == Some(&'(') => {
                let start = i + 2;
                let Some(end) = close(body, start, '(', ')', true) else {
                    return names(&body[start..]);
                };
                if names(&body[start..end]) {
                    return true;
                }
                i = end;
            }
            '`' => {
                let start = i + 1;
                let mut end = start;
                while end < body.len() && body[end] != '`' {
                    end += if body[end] == '\\' { 2 } else { 1 };
                }
                let end = end.min(body.len());
                if names(&body[start..end]) {
                    return true;
                }
                i = end;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// The index of the `close` that ends what opened just before `start`,
/// skipping quotes and nested pairs; `None` when it never ends. Inside
/// a `script` (a command substitution's body), comments and here-document
/// bodies are skipped too: a `)` in them closes nothing.
fn close(chars: &[char], start: usize, open: char, close: char, script: bool) -> Option<usize> {
    let parens = script;
    let mut depth = 1_usize;
    let mut i = start;
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    // Whether the character at `i` starts a word, where `#` opens a comment.
    let mut word_start = true;
    while i < chars.len() {
        let starts = word_start;
        word_start = matches!(chars[i], ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')');
        match chars[i] {
            '\\' => i += 1,
            '#' if parens && starts => {
                while chars.get(i + 1).is_some_and(|&c| c != '\n') {
                    i += 1;
                }
            }
            '<' if parens && chars.get(i + 1) == Some(&'<') => {
                if chars.get(i + 2) == Some(&'<') {
                    // A here-string: its word is read as any other.
                    i += 2;
                } else {
                    i += 2;
                    let strip = chars.get(i) == Some(&'-');
                    if strip {
                        i += 1;
                    }
                    while matches!(chars.get(i), Some(' ' | '\t')) {
                        i += 1;
                    }
                    let (delimiter, end) = heredoc_delimiter(chars, i)?;
                    heredocs.push((delimiter, strip));
                    i = end;
                    continue;
                }
            }
            '\n' if parens && !heredocs.is_empty() => {
                i += 1;
                for (delimiter, strip) in std::mem::take(&mut heredocs) {
                    loop {
                        let line_start = i;
                        while chars.get(i).is_some_and(|&c| c != '\n') {
                            i += 1;
                        }
                        let line: String = chars[line_start..i].iter().collect();
                        let last = i >= chars.len();
                        i += 1;
                        let line = if strip {
                            line.trim_start_matches('\t')
                        } else {
                            line.as_str()
                        };
                        if line == delimiter {
                            break;
                        }
                        if last {
                            // The body never ends.
                            return None;
                        }
                    }
                }
                word_start = true;
                continue;
            }
            '\'' if open == '(' => {
                i += 1;
                while *chars.get(i)? != '\'' {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                loop {
                    match *chars.get(i)? {
                        '\\' => i += 1,
                        '"' => break,
                        _ => {}
                    }
                    i += 1;
                }
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A here-document's delimiter word at `start`, quotes removed, and the
/// index just after it; `None` when it is empty or holds an expansion.
fn heredoc_delimiter(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut text = String::new();
    let mut i = start;
    while let Some(&c) = chars.get(i) {
        match c {
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')' => break,
            '\'' | '"' => {
                i += 1;
                while *chars.get(i)? != c {
                    if c == '"' && matches!(chars[i], '$' | '`' | '\\') {
                        return None;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                i += 1;
                text.push(*chars.get(i)?);
            }
            '$' | '`' => return None,
            _ => text.push(c),
        }
        i += 1;
    }
    (!text.is_empty()).then_some((text, i))
}

/// The program word of a simple command and the words after it, with the
/// shell keywords and prefixes that run the next word set aside: `if`,
/// `then`, `else`, `elif`, `do`, `while`, `until`, `!`, `{`, `}`, `time`,
/// `exec`, `command`, `nohup`, `env` and leading assignments, and `timeout`
/// with one literal duration. `None` when nothing is run here.
pub(super) fn program(words: &[Word]) -> Option<(&Word, &[Word])> {
    let mut k = 0;
    while let Some(word) = words.get(k) {
        match word {
            Word::Assign(_) => k += 1,
            Word::Lit(text) => match text.as_str() {
                "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "!" | "{" | "}"
                | "time" | "exec" | "command" | "nohup" | "env" | "fi" | "done" => k += 1,
                "timeout" => {
                    k += 1;
                    if let Some(Word::Lit(duration)) = words.get(k)
                        && duration_word(duration)
                    {
                        k += 1;
                    } else {
                        return None;
                    }
                }
                _ => break,
            },
            Word::Home(_) | Word::Opaque | Word::Fields => break,
        }
    }
    let (first, rest) = words[k..].split_first()?;
    Some((first, rest))
}

/// A `timeout` duration: digits, an optional fraction and unit.
fn duration_word(word: &str) -> bool {
    let number = word.trim_end_matches(['s', 'm', 'h', 'd']);
    number.len() + 1 >= word.len()
        && !number.is_empty()
        && number.split_once('.').map_or(
            number.bytes().all(|b| b.is_ascii_digit()),
            |(whole, fraction)| {
                !whole.is_empty()
                    && whole.bytes().all(|b| b.is_ascii_digit())
                    && fraction.bytes().all(|b| b.is_ascii_digit())
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(text: &str) -> Word {
        Word::Lit(text.to_owned())
    }

    /// The words of every simple command, where their input comes from set
    /// aside.
    fn simple_commands(command: &str) -> Option<Vec<Simple>> {
        super::simple_commands(command)
            .map(|commands| commands.into_iter().map(|command| command.words).collect())
    }

    /// How many simple commands run `claude`, and of those, how many read
    /// only their own input; `None` when the command is refused.
    fn claude_runs(command: &str) -> Option<(usize, usize)> {
        let commands = super::simple_commands(command)?;
        let runs: Vec<&Command> = commands
            .iter()
            .filter(|command| {
                program(&command.words).is_some_and(|(first, _)| *first == lit("claude"))
            })
            .collect();
        Some((
            runs.len(),
            runs.iter().filter(|command| command.own_input).count(),
        ))
    }

    #[test]
    fn splits_lists_pipes_and_keeps_quoted_words_whole() {
        let commands = simple_commands(
            "S=/w/run && cd \"$S\" && claude -p \"Say 'hi' now\" --name t > \"$S/out\" 2> e; echo \"exit $?\"",
        )
        .unwrap();
        assert_eq!(
            commands,
            vec![
                vec![Word::Assign(Some("S=/w/run".to_owned()))],
                vec![lit("cd"), Word::Opaque],
                vec![
                    lit("claude"),
                    lit("-p"),
                    lit("Say 'hi' now"),
                    lit("--name"),
                    lit("t")
                ],
                vec![lit("echo"), Word::Opaque],
            ]
        );
    }

    #[test]
    fn heredoc_bodies_and_comments_are_not_commands() {
        let commands = simple_commands(
            "cat > /w/a.py <<'EOF'\nclaude -p \"inside\"\nEOF\n# claude -p \"comment\"\npython3 - x <<-END\n\tclaude -p y\n\tEND\nclaude -p 'after'",
        )
        .unwrap();
        assert_eq!(
            commands,
            vec![
                vec![lit("cat")],
                vec![lit("python3"), lit("-"), lit("x")],
                vec![lit("claude"), lit("-p"), lit("after")],
            ]
        );
    }

    #[test]
    fn expansions_globs_and_tildes_are_not_literal() {
        let commands =
            simple_commands("claude -p $P \"a$B\" ${C} $(d) `e` '$f' * ~/bin/claude ~x \\$g")
                .unwrap();
        assert_eq!(
            commands[0],
            vec![
                lit("claude"),
                lit("-p"),
                Word::Fields,
                Word::Opaque,
                Word::Fields,
                Word::Fields,
                Word::Fields,
                lit("$f"),
                Word::Fields,
                Word::Home("bin/claude".to_owned()),
                Word::Opaque,
                lit("$g"),
            ]
        );
        // The substitutions' bodies are read as commands too.
        assert_eq!(commands[1], vec![lit("d")]);
        assert_eq!(commands[2], vec![lit("e")]);
    }

    #[test]
    fn words_the_shell_may_split_are_told_from_one_word() {
        let commands = simple_commands(
            "S=/w; echo $S ${S} $S/x \"$@\" \"${a[@]}\" \"${@:2}\" \"$*\" \"$1\" $1 $# \
             $((1+1)) \"$((1+1))\" $[1] {a,b} a=$B $'x' $\"y\" ~u <(true) \"`d`\"",
        )
        .unwrap();
        assert_eq!(
            commands[1],
            vec![
                lit("echo"),
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                Word::Fields,
                Word::Fields,
                Word::Fields,
                Word::Opaque,
                Word::Opaque,
                Word::Fields,
                Word::Fields,
                Word::Fields,
                Word::Opaque,
                Word::Fields,
                Word::Fields,
                Word::Assign(None),
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
            ]
        );
    }

    #[test]
    fn only_a_plain_value_set_first_and_never_touched_is_one_word() {
        let known = |command: &str| {
            let mut names: Vec<String> = one_word_names(command).into_iter().collect();
            names.sort();
            names
        };
        assert_eq!(known("S=/w/run; cd $S && x ${S}/y"), ["S"]);
        assert_eq!(
            known(" \tA=1 B='/b' C=\"c\" && echo $A $B $C"),
            ["A", "B", "C"]
        );
        assert_eq!(known("S=/w\necho $S"), ["S"]);
        assert_eq!(known("S=/w"), ["S"]);
        // The last assignment is the value kept.
        assert_eq!(known("S=/w S='a b'; echo $S"), Vec::<String>::new());
        assert_eq!(known("S='a b' S=/w; echo $S"), ["S"]);
        for command in [
            "S='a b'; echo $S",
            "S=; echo $S",
            "S=*; echo $S",
            "S=/w/$X; echo $S",
            "A=x\\ B=/w; echo $B",
            "S=/w & echo $S",
            "S=/w || echo $S",
            "S=/w | echo $S",
            "S=/w echo $S",
            "(S=/w); echo $S",
            "{ S=/w; }; echo $S",
            "# c\nS=/w; echo $S",
            "true; S=/w; echo $S",
            "S=/w; S=/v; echo $S",
            "S=/w; echo ${S:-x}",
            "S=/w; echo ${#S}",
            "S=/w; for S in a; do :; done; echo $S",
            "S=/w; unset S; echo $S",
            "S=/w; echo 'S'; echo $S",
            "S=/w; echo $S; IFS=:",
            "PATH=/w; echo $PATH",
            "BASH_ENV=/w; echo $BASH_ENV",
            "_=/w; echo $_",
            "DIRSTACK=/w; echo $DIRSTACK",
        ] {
            assert_eq!(known(command), Vec::<String>::new(), "{command:?}");
        }
        // Another name set first is still known.
        assert_eq!(known("S=/w T=/t; unset T; echo $S $T"), ["S"]);
    }

    #[test]
    fn a_command_that_may_set_a_variable_unnamed_knows_none() {
        // The word `echo $S` ends up as, with `pre` run before it.
        let word = |pre: &str| {
            let commands = super::simple_commands(&format!("S=/w; {pre}; echo $S/c.json")).unwrap();
            commands
                .iter()
                .rfind(|command| command.words.first() == Some(&lit("echo")))
                .unwrap()
                .words[1]
                .clone()
        };
        for pre in [
            "eval \"$X\"",
            "source /w/env",
            ". ./env",
            "\\. ./env",
            "read \"$N\"",
            "declare \"$N=a b\"",
            "export \"$N\"",
            "unset \"$N\"",
            "builtin read \"$N\"",
            "command eval x",
            "printf -v \"$N\" x",
            "printf -vN x",
            "printf $O \"$N\" x",
            "$CMD",
            "\"$CMD\" a",
            "timeout $T read \"$N\"",
            "(eval \"$X\")",
            "x=$(trap \"$X\" EXIT)",
            "noglob read \"$N\"",
            "wait -p \"$N\"",
        ] {
            assert_eq!(word(pre), Word::Fields, "{pre:?}");
        }
        // Text or other programs that only name these words do not.
        for pre in [
            "echo source eval . read",
            "cd .",
            "ls . && cat ./f",
            "printf '%s' -v",
            "printf -- x",
            "git commit -m \"read the source\"",
        ] {
            assert_eq!(word(pre), Word::Opaque, "{pre:?}");
        }
    }

    #[test]
    fn loops_keywords_and_prefixes_leave_the_program() {
        let commands = simple_commands(
            "for i in 1 2; do timeout 60 env A=1 claude -p x; done; if true; then nohup claude -p y & fi",
        )
        .unwrap();
        let programs: Vec<Option<&Word>> = commands
            .iter()
            .map(|words| program(words).map(|(first, _)| first))
            .collect();
        assert!(programs.contains(&Some(&lit("claude"))));
        assert_eq!(
            programs
                .iter()
                .filter(|p| **p == Some(&lit("claude")))
                .count(),
            2
        );
    }

    #[test]
    fn a_shell_script_word_is_read_as_commands() {
        let commands = simple_commands("bash -lc 'cd /w && claude -p \"deep\"'").unwrap();
        assert_eq!(
            commands.last().unwrap(),
            &vec![lit("claude"), lit("-p"), lit("deep")]
        );
    }

    #[test]
    fn unterminated_quotes_and_substitutions_refuse_the_command() {
        assert_eq!(simple_commands("claude -p \"open"), None);
        assert_eq!(simple_commands("claude -p 'open"), None);
        assert_eq!(simple_commands("x=$(claude -p y"), None);
    }

    #[test]
    fn redirections_and_descriptors_are_set_aside() {
        let commands =
            simple_commands("claude -p x >out 2>&1 <in 3>>log &>all >| f 1> o <<< here").unwrap();
        assert_eq!(commands, vec![vec![lit("claude"), lit("-p"), lit("x")]]);
    }

    #[test]
    fn here_documents_in_substitutions_are_not_commands() {
        // A `)` inside a here-document body inside `$(…)` closes nothing.
        for command in [
            "x=$(cat <<'EOF'\n) ; claude -p \"fake\" ; (\nEOF\n)",
            "msg=$(cat <<'EOF'\nfoo) claude -p 'zzz'\nEOF\n)",
            "git commit -m \"$(cat <<\"END\"\nfix) claude -p q\nEND\n)\"",
            "y=$(cat <<-EOF\n\t) claude -p tab\n\tEOF\n)",
            "z=$(echo a # ) claude -p comment\n)",
        ] {
            assert_eq!(claude_runs(command), Some((0, 0)), "{command:?}");
        }
        // A here-document that never ends refuses the command.
        assert_eq!(claude_runs("x=$(cat <<EOF\n) claude -p y\n"), None);
        // A launch after the substitution is still read.
        assert_eq!(
            claude_runs("m=$(cat <<'EOF'\nbody\nEOF\n); claude -p after"),
            Some((1, 1))
        );
        // A shift in arithmetic is no here-document.
        assert_eq!(claude_runs("n=$((1 << 2)); claude -p after"), Some((1, 1)));
    }

    #[test]
    fn an_unquoted_here_document_running_claude_refuses_the_command() {
        // The shell runs the substitutions of an unquoted body: one naming
        // `claude` may be a launch, which this reader does not read there.
        for command in [
            "cat <<EOF\n$(claude -p 'same prompt')\nEOF",
            "cat <<EOF\n`claude -p same`\nEOF",
            "cat <<-EOF\n\tx $(cd /w && claude -p tab) y\n\tEOF",
            "cat <<$D\n$(claude -p d)\n$D",
            "x=$(cat <<EOF\n$(claude -p inner)\nEOF\n)",
            "cat <<EOF\n${v:-$(claude -p deep)}\nEOF\nclaude -p after",
            "cat <<EOF\n$(echo \"never ends\" claude\nEOF",
        ] {
            assert_eq!(claude_runs(command), None, "{command:?}");
        }
        // A quoted delimiter keeps the body as text, and a body whose
        // substitutions do not name `claude` runs no launch.
        for command in [
            "cat <<'EOF'\n$(claude -p 'same prompt')\nEOF",
            "cat <<\"EOF\"\n`claude -p same`\nEOF",
            "cat <<\\EOF\n$(claude -p escaped)\nEOF",
            "cat <<-'EOF'\n\t$(claude -p tab)\n\tEOF",
            "cat <<E\"O\"F\n$(claude -p part)\nEOF",
            "cat <<EOF\n$(date) claude -p text `pwd`\nEOF",
            "cat <<EOF\n\\$(claude -p escaped)\nEOF",
            "cat <<-EOF\n\tclaude -p plain\n\tEOF",
        ] {
            assert_eq!(claude_runs(command), Some((0, 0)), "{command:?}");
        }
    }

    #[test]
    fn arrays_and_function_bodies_are_not_launches() {
        assert_eq!(claude_runs("x=( claude -p arr )"), Some((0, 0)));
        assert_eq!(
            claude_runs("x+=( claude -p arr ); claude -p real"),
            Some((1, 1))
        );
        assert_eq!(claude_runs("local x=(claude -p arr)"), Some((0, 0)));
        // An array holding a substitution, and a function, refuse the command.
        assert_eq!(claude_runs("x=( $(claude -p arr) )"), None);
        assert_eq!(claude_runs("f() { claude -p inner; }"), None);
        assert_eq!(claude_runs("f () { claude -p inner; }; f"), None);
        assert_eq!(claude_runs("function f { claude -p inner; }"), None);
        // A subshell is run.
        assert_eq!(claude_runs("( claude -p sub )"), Some((1, 1)));
    }

    #[test]
    fn a_launch_given_other_input_does_not_read_only_its_own() {
        for command in [
            "git diff | claude -p \"Review this\"",
            "git diff |& claude -p x",
            "git diff |\n claude -p x",
            "claude -p \"x\" < file",
            "< file claude -p x",
            "claude -p x <<EOF\nmore\nEOF",
            "claude -p x <<< more",
            "claude -p x 0<&3",
            "echo a | { claude -p x; }",
            "echo a | ( cd /w && claude -p x )",
            "echo a | while read l; do claude -p x; done",
            "while read l; do claude -p x; done < list",
            "for i in 1; do claude -p x; done <<EOF\na\nEOF",
            "{ claude -p x; } < in",
            "( claude -p x ) < in",
            "if true; then claude -p x; fi < in",
            "echo a | bash -c 'claude -p x'",
            "echo a | echo \"$(claude -p x)\"",
        ] {
            assert_eq!(claude_runs(command), Some((1, 0)), "{command:?}");
        }
        // Output redirections, a pipe after it, and other lists keep it.
        for command in [
            "claude -p x > out 2> err",
            "claude -p x 2>&1 | tee log",
            "claude -p x >> out 2>&1 &",
            "a || claude -p x",
            "a | b; claude -p x",
            "a | b && claude -p x",
            "a | b\nclaude -p x",
            "cat <<EOF > f\nbody\nEOF\nclaude -p x > out",
            "out=$(claude -p x 2>&1); echo \"$out\" | grep y",
            "bash -c 'claude -p x' > out",
            "( claude -p x ) > out 2>&1",
        ] {
            assert_eq!(claude_runs(command), Some((1, 1)), "{command:?}");
        }
    }

    #[test]
    fn a_redirected_exec_feeds_every_later_command() {
        // `exec < f` makes the file the standard input of the rest.
        for command in [
            "exec < /w/f; claude -p x",
            "exec 0</w/f; claude -p x",
            "exec <<<x; claude -p x",
            "exec <<EOF\nx\nEOF\nclaude -p x",
            "exec </w/f\nclaude -p x",
            "builtin exec < /w/f; claude -p x",
        ] {
            assert_eq!(claude_runs(command), Some((1, 0)), "{command:?}");
        }
        // Output redirected, or a program run with its own input, keeps it.
        for command in [
            "exec > /w/log 2>&1; claude -p x",
            "exec cat < /w/f; claude -p x",
        ] {
            assert_eq!(claude_runs(command), Some((1, 1)), "{command:?}");
        }
    }

    #[test]
    fn a_launch_in_a_process_substitution_does_not_read_only_its_own() {
        // `>(…)` reads what the command writes to it; `<(…)` reads what the
        // command holding it reads, a pipe among it.
        for command in [
            "printf extra > >(claude -p 'x')",
            "tee >(claude -p x) < f",
            "printf x | tee >(claude -p x)",
            "cat f | cat <(claude -p 'x')",
            "diff <(claude -p a) b",
            "while read l; do :; done < <(claude -p x)",
            "echo 2>(claude -p x)",
            "x=$(cat <(claude -p x))",
            "cat <(echo \"$(claude -p x)\")",
            // zsh's `=(…)`, a temporary file the body writes.
            "printf extra | cat =(claude -p x)",
            "cat =(claude -p x)",
        ] {
            assert_eq!(claude_runs(command), Some((1, 0)), "{command:?}");
        }
        // It is one word of the command holding it, and its body is read.
        assert_eq!(
            simple_commands("cat <(echo a) f >(tee b) c").unwrap(),
            vec![
                vec![lit("cat"), Word::Opaque, lit("f"), Word::Opaque, lit("c")],
                vec![lit("echo"), lit("a")],
                vec![lit("tee"), lit("b")],
            ]
        );
        // Quoted, it is text.
        assert_eq!(claude_runs("echo '>(claude -p x)'"), Some((0, 0)));
    }
}
