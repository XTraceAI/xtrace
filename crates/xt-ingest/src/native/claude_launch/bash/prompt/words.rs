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
//! is skipped, never read as commands. A command or process substitution's
//! body is read as commands too, up to a small depth. Comments are skipped.
//! An unterminated quote or substitution refuses the whole command.

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

/// Every simple command `command` holds, in order, with those in
/// substitutions after the command that holds them; `None` when it cannot be
/// split.
pub(super) fn simple_commands(command: &str) -> Option<Vec<Simple>> {
    if command.len() > MAX_COMMAND {
        return None;
    }
    let mut out = Vec::new();
    split(command, 0, &mut out)?;
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
pub(super) fn split(script: &str, depth: usize, out: &mut Vec<Simple>) -> Option<()> {
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
    };
    reader.run()?;
    let Reader { commands, subs, .. } = reader;
    for simple in commands {
        if out.len() >= MAX_COMMANDS {
            return None;
        }
        let nested = shell_script(&simple).map(str::to_owned);
        out.push(simple);
        if let Some(nested) = nested
            && depth < MAX_DEPTH
        {
            split(&nested, depth + 1, out)?;
        }
    }
    if depth < MAX_DEPTH {
        for body in subs {
            split(&body, depth + 1, out)?;
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
    commands: Vec<Simple>,
    current: Simple,
    word: Option<Building>,
    target: Option<Target>,
    heredocs: Vec<(String, bool)>,
    subs: Vec<String>,
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
        if !self.current.is_empty() {
            self.commands.push(std::mem::take(&mut self.current));
        }
    }

    fn run(&mut self) -> Option<()> {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\r' => {
                    self.end_word();
                    self.i += 1;
                }
                '\n' => {
                    self.end_command();
                    self.i += 1;
                    self.skip_heredocs();
                }
                '#' if self.word.is_none() => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                ';' | '(' | ')' => {
                    self.end_command();
                    self.i += 1;
                }
                '&' => match self.peek(1) {
                    Some('&') => {
                        self.end_command();
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
                        self.end_command();
                        self.i += 1;
                    }
                },
                '|' => {
                    self.end_command();
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
                let end = close(self.chars, start, '(', ')')?;
                if !arithmetic {
                    self.subs.push(self.chars[start..end].iter().collect());
                }
                self.word().opaque();
                self.i = end + 1;
            }
            Some('{') => {
                let end = close(self.chars, self.i + 2, '{', '}')?;
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
        self.subs.push(body);
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
/// skipping quotes and nested pairs; `None` when it never ends.
fn close(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 1_usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
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
}
