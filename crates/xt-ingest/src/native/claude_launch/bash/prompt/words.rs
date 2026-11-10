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
//! character, a brace expansion or a leading `~` (kept apart, for a program
//! path). A redirection and its target are set aside; a here-document's body
//! is skipped, never read as commands, also inside a substitution. A command
//! or process substitution's body is read as commands too, up to a small
//! depth. Comments and array literals (`x=( … )`) are skipped. An
//! unterminated quote, substitution or here-document, a function definition,
//! or an array literal holding a substitution refuses the whole command.
//!
//! Each simple command says whether it reads only the input the whole
//! command was given ([`Command::own_input`]): not when it, or a group or
//! loop around it, has a `<` redirection or a here-document, or follows a `|`
//! in a pipeline, or is inside a substitution or `sh -c` script of a command
//! that does not.

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
    /// A word the shell supplies only when it runs.
    Opaque,
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
    /// it, not after a `|`, and not inside a substitution or `sh -c` script of
    /// a command that does read other input.
    pub(super) own_input: bool,
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
    let mut out = Vec::new();
    split(command, 0, true, &mut out)?;
    Some(out)
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
/// recorded command was given.
fn split(script: &str, depth: usize, inherited: bool, out: &mut Vec<Command>) -> Option<()> {
    let chars: Vec<char> = script.chars().collect();
    let mut reader = Reader {
        chars: &chars,
        i: 0,
        commands: Vec::new(),
        current: Vec::new(),
        word: None,
        target: None,
        heredocs: Vec::new(),
        subs: Vec::new(),
        current_subs: Vec::new(),
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
        if let Some(nested) = nested
            && depth < MAX_DEPTH
        {
            split(&nested, depth + 1, own, out)?;
        }
    }
    if depth < MAX_DEPTH {
        for (body, own) in subs {
            split(&body, depth + 1, inherited && own, out)?;
        }
    }
    Some(())
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
        } else if self.plain {
            self.bare.push(c);
        }
        self.text.push(c);
    }

    fn opaque(&mut self) {
        self.literal = false;
        self.plain = false;
    }

    fn finish(self) -> Word {
        let braced = self.brace && self.text != "{" && self.text != "}";
        if is_assignment(&self.bare) {
            return Word::Assign((self.literal && !braced && !self.tilde).then_some(self.text));
        }
        if !self.literal || braced {
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
    /// A here-document's delimiter; `true` strips leading tabs.
    Heredoc(bool),
}

struct Reader<'a> {
    chars: &'a [char],
    i: usize,
    commands: Vec<Command>,
    current: Simple,
    word: Option<Building>,
    target: Option<Target>,
    heredocs: Vec<(String, bool)>,
    /// Substitution bodies, and whether the command holding each reads only
    /// its own input.
    subs: Vec<(String, bool)>,
    /// The substitutions of the command being read.
    current_subs: Vec<String>,
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
    /// Text that is not run as it stands was found: a function definition.
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
            Some(Target::Heredoc(strip)) => self.heredocs.push((building.text, strip)),
            Some(Target::Redirect) => {}
            None => self.current.push(building.finish()),
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
        let piped = self.piped && !self.current.is_empty();
        if piped && first.is_some_and(|word| COMPOUND.contains(&word)) {
            self.group_input = true;
        }
        // `function name …` defines a function: its body is not run here.
        if first == Some("function") {
            self.refused = true;
        }
        let own = !self.redirected && !piped;
        for body in self.current_subs.drain(..) {
            self.subs.push((body, own));
        }
        if !self.current.is_empty() {
            self.commands.push(Command {
                words: std::mem::take(&mut self.current),
                own_input: own,
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
                        // `$'…'`: escapes this reader does not decode.
                        self.word().opaque();
                        self.i += 2;
                        loop {
                            match self.peek(0)? {
                                '\\' => self.i += 2,
                                '\'' => break,
                                _ => self.i += 1,
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
                '`' => self.backquote()?,
                '*' | '?' | '[' => {
                    let word = self.word();
                    word.push(c, false);
                    word.opaque();
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
                self.target = Some(Target::Redirect);
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
        self.word().plain = false;
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
                '`' => self.backquote()?,
                c => {
                    self.word().push(c, true);
                    self.i += 1;
                }
            }
        }
    }

    /// One `$` expansion, at `$`. A `$` that starts none is literal.
    fn expansion(&mut self, quoted: bool) -> Option<()> {
        match self.peek(1) {
            Some('(') => {
                let arithmetic = self.peek(2) == Some('(');
                let start = self.i + 2;
                let end = close(self.chars, start, '(', ')', !arithmetic)?;
                if !arithmetic {
                    self.current_subs
                        .push(self.chars[start..end].iter().collect());
                }
                self.word().opaque();
                self.i = end + 1;
            }
            Some('{') => {
                let end = close(self.chars, self.i + 2, '{', '}', false)?;
                self.word().opaque();
                self.i = end + 1;
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                self.word().opaque();
                self.i += 1;
                while self
                    .peek(0)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.i += 1;
                }
            }
            Some(c)
                if c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
            {
                self.word().opaque();
                self.i += 2;
            }
            _ => {
                self.word().push('$', quoted);
                self.i += 1;
            }
        }
        Some(())
    }

    /// One `` `…` `` substitution, at its opening backquote.
    fn backquote(&mut self) -> Option<()> {
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
        self.current_subs.push(body);
        self.word().opaque();
        Some(())
    }

    /// Skip the bodies of the here-documents the line just ended opened.
    fn skip_heredocs(&mut self) {
        for (delimiter, strip) in std::mem::take(&mut self.heredocs) {
            while self.i < self.chars.len() {
                let start = self.i;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                let line: String = self.chars[start..self.i].iter().collect();
                // Past the newline, if any.
                self.i = (self.i + 1).min(self.chars.len());
                let line = if strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == delimiter {
                    break;
                }
            }
        }
    }
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
            Word::Home(_) | Word::Opaque => break,
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
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                Word::Opaque,
                lit("$f"),
                Word::Opaque,
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
}
