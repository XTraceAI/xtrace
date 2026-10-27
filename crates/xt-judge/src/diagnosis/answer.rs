//! The strict reader of a diagnosis answer: one JSON object with exactly the
//! five keys the bundled skill names, every rule of the skill checked.
//!
//! The answer may be wrapped in one code fence (```` ```json ```` … ```` ``` ````),
//! which a small model sometimes adds although the skill says not to; nothing
//! else may be around the object. What is refused carries a short reason
//! ([`AnswerError`]) that a second try is given ([`retry_note`]).
//!
//! Which suggestion fits a borderline case (a skill or one instruction line)
//! is the model's call; the reader checks only that the draft has the form
//! its suggestion needs.

use crate::rubric::is_slug;
use serde_json::{Map, Value};

/// The five keys, in the order the skill writes them.
pub const KEYS: [&str; 5] = [
    "what_happened",
    "real_problem",
    "suggestion",
    "draft",
    "why",
];
/// Characters of `what_happened`, at most.
pub const MAX_WHAT_HAPPENED_CHARS: usize = 400;
/// Characters of `why`, at most.
pub const MAX_WHY_CHARS: usize = 300;
/// Characters of an instruction line, at most.
pub const MAX_LINE_CHARS: usize = 200;
/// Lines of a skill draft, at most.
pub const MAX_SKILL_LINES: usize = 40;
/// Bytes of a skill draft, at most.
pub const MAX_SKILL_BYTES: usize = 8 * 1024;
/// Bytes of a whole answer read, at most.
pub const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// Characters of a skill's description, at most.
pub const MAX_DESCRIPTION_CHARS: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealProblem {
    Yes,
    No,
    NotSure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suggestion {
    MakeSkill,
    AddToClaudeMd,
    Nothing,
}

/// One accepted answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub what_happened: String,
    pub real_problem: RealProblem,
    pub suggestion: Suggestion,
    /// A whole `SKILL.md` for [`Suggestion::MakeSkill`], one line for
    /// [`Suggestion::AddToClaudeMd`], empty for [`Suggestion::Nothing`].
    pub draft: String,
    pub why: String,
}

impl Answer {
    /// The skill's name, for a skill draft.
    pub fn skill_name(&self) -> Option<String> {
        (self.suggestion == Suggestion::MakeSkill)
            .then(|| skill_draft_name(&self.draft).ok())
            .flatten()
    }
}

/// Why an answer was refused. Each message is one sentence a second try can
/// be told as it is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error("the answer was empty")]
    Empty,
    #[error("the answer was longer than {MAX_ANSWER_BYTES} bytes")]
    TooLarge,
    #[error("the answer was not one JSON object with nothing before or after it")]
    NotOneObject,
    #[error("the JSON object must have exactly the keys {}", KEYS.join(", "))]
    Keys,
    #[error("\"{0}\" must be a string")]
    NotAString(&'static str),
    #[error("\"real_problem\" must be \"yes\", \"no\" or \"not_sure\"")]
    RealProblem,
    #[error("\"suggestion\" must be \"make_skill\", \"add_to_claude_md\" or \"nothing\"")]
    Suggestion,
    #[error("\"suggestion\" must be \"nothing\" unless \"real_problem\" is \"yes\"")]
    SuggestionWithoutProblem,
    #[error("\"draft\" must be empty when \"suggestion\" is \"nothing\", and only then")]
    DraftEmptiness,
    #[error("\"what_happened\" must be plain text of 1 to {MAX_WHAT_HAPPENED_CHARS} characters")]
    WhatHappened,
    #[error(
        "\"why\" must be one sentence of 1 to {MAX_WHY_CHARS} characters naming message numbers in square brackets"
    )]
    Why,
    #[error(
        "an \"add_to_claude_md\" draft must be one line of at most {MAX_LINE_CHARS} characters"
    )]
    Line,
    #[error("{0}")]
    Skill(SkillDraftError),
    #[error(
        "the draft must not contain an email address, a key, a token or a sign-in inside a link; leave such details out"
    )]
    Secret,
}

/// Why a skill draft was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillDraftError {
    #[error(
        "a \"make_skill\" draft must start with a line \"---\", then \"name: <name>\", then \"description: <one sentence>\", then \"---\""
    )]
    Frontmatter,
    #[error(
        "the skill name must use only lowercase letters a-z, digits and \"-\", start and end with a letter or digit, and be at most 64 characters"
    )]
    Name,
    #[error(
        "the skill description must be one sentence of at most {MAX_DESCRIPTION_CHARS} characters"
    )]
    Description,
    #[error("after its header the skill must have a \"# Title\" line and some steps")]
    Body,
    #[error("the skill must be at most {MAX_SKILL_LINES} lines and {MAX_SKILL_BYTES} bytes")]
    TooLong,
    #[error(
        "the skill must be plain text without control characters other than line breaks and tabs"
    )]
    Control,
}

/// Read one answer.
pub fn parse(text: &str) -> Result<Answer, AnswerError> {
    if text.len() > MAX_ANSWER_BYTES {
        return Err(AnswerError::TooLarge);
    }
    let body = unfence(text.trim().trim_start_matches('\u{feff}').trim());
    if body.is_empty() {
        return Err(AnswerError::Empty);
    }
    let value: Value = serde_json::from_str(body).map_err(|_| AnswerError::NotOneObject)?;
    let Value::Object(object) = value else {
        return Err(AnswerError::NotOneObject);
    };
    if object.len() != KEYS.len() || KEYS.iter().any(|key| !object.contains_key(*key)) {
        return Err(AnswerError::Keys);
    }
    let what_happened = string(&object, "what_happened")?;
    let real_problem = match string(&object, "real_problem")?.as_str() {
        "yes" => RealProblem::Yes,
        "no" => RealProblem::No,
        "not_sure" => RealProblem::NotSure,
        _ => return Err(AnswerError::RealProblem),
    };
    let suggestion = match string(&object, "suggestion")?.as_str() {
        "make_skill" => Suggestion::MakeSkill,
        "add_to_claude_md" => Suggestion::AddToClaudeMd,
        "nothing" => Suggestion::Nothing,
        _ => return Err(AnswerError::Suggestion),
    };
    let draft = string(&object, "draft")?;
    let why = string(&object, "why")?;
    if suggestion != Suggestion::Nothing && real_problem != RealProblem::Yes {
        return Err(AnswerError::SuggestionWithoutProblem);
    }
    if draft.trim().is_empty() != (suggestion == Suggestion::Nothing) {
        return Err(AnswerError::DraftEmptiness);
    }
    let what_happened =
        plain_line(&what_happened, MAX_WHAT_HAPPENED_CHARS).ok_or(AnswerError::WhatHappened)?;
    let why = plain_line(&why, MAX_WHY_CHARS)
        .filter(|why| names_a_message(why))
        .ok_or(AnswerError::Why)?;
    let draft = match suggestion {
        Suggestion::Nothing => String::new(),
        Suggestion::AddToClaudeMd => {
            let line = draft.trim();
            if line.chars().count() > MAX_LINE_CHARS
                || line.chars().any(|c| c.is_control() || is_line_break(c))
            {
                return Err(AnswerError::Line);
            }
            line.to_owned()
        }
        Suggestion::MakeSkill => {
            let draft = normalise_draft(&draft);
            check_skill_draft(&draft).map_err(AnswerError::Skill)?;
            quote_description(&draft).map_err(AnswerError::Skill)?
        }
    };
    if has_secret(&draft) {
        return Err(AnswerError::Secret);
    }
    Ok(Answer {
        what_happened,
        real_problem,
        suggestion,
        draft,
        why,
    })
}

/// The note a second try gets after `error`.
pub fn retry_note(error: &AnswerError) -> String {
    format!(
        "\n\nYour previous answer to this input was refused by XTrace: {error}. This is the last \
         try. Reply again with exactly one JSON object that follows every rule above, with \
         nothing before or after it.\n"
    )
}

/// Whether `c` breaks a line for some reader: `\n`, `\r`, a vertical tab, a
/// form feed, NEL (U+0085), or the Unicode line and paragraph separators.
pub fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// A skill draft's line breaks (every one of [`is_line_break`]) as `\n`,
/// without trailing whitespace at the end, ending in one line break.
pub fn normalise_draft(draft: &str) -> String {
    let draft: String = draft
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if is_line_break(c) { '\n' } else { c })
        .collect();
    format!("{}\n", draft.trim_end())
}

/// Lines of a text, every one of [`is_line_break`] ending one.
fn line_count(text: &str) -> usize {
    let breaks = text.chars().filter(|c| is_line_break(*c)).count();
    breaks + usize::from(!text.is_empty() && !text.ends_with(is_line_break))
}

/// Check a whole `SKILL.md` draft: the header, the name, a title and the
/// bounds. The draft's line breaks are `\n` ([`normalise_draft`]); any other
/// line break still counts toward the line limit and is then refused.
pub fn check_skill_draft(draft: &str) -> Result<(), SkillDraftError> {
    if draft.len() > MAX_SKILL_BYTES || line_count(draft) > MAX_SKILL_LINES {
        return Err(SkillDraftError::TooLong);
    }
    if draft
        .chars()
        .any(|c| (c.is_control() || is_line_break(c)) && !matches!(c, '\n' | '\t'))
    {
        return Err(SkillDraftError::Control);
    }
    let name = skill_draft_name(draft)?;
    if !is_slug(&name) {
        return Err(SkillDraftError::Name);
    }
    let mut lines = draft.lines().skip(2);
    let description = lines
        .next()
        .and_then(|line| line.strip_prefix("description:"))
        .ok_or(SkillDraftError::Frontmatter)
        .and_then(description_text)?;
    if description.is_empty() || description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(SkillDraftError::Description);
    }
    if lines.next() != Some("---") {
        return Err(SkillDraftError::Frontmatter);
    }
    let body: Vec<&str> = lines.filter(|line| !line.trim().is_empty()).collect();
    if body.first().is_none_or(|line| !line.starts_with("# ")) || body.len() < 2 {
        return Err(SkillDraftError::Body);
    }
    Ok(())
}

/// The text of a header's `description:` value: a plain value as it is, or
/// the content of a JSON-style `"…"` or a YAML `'…'` quoted one. A quoted
/// value that does not read as one is refused.
fn description_text(value: &str) -> Result<String, SkillDraftError> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str::<String>(value).map_err(|_| SkillDraftError::Description);
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        let inner = &value[1..value.len() - 1];
        if inner.replace("''", "").contains('\'') {
            return Err(SkillDraftError::Description);
        }
        return Ok(inner.replace("''", "'"));
    }
    Ok(value.to_owned())
}

/// A checked skill draft ([`check_skill_draft`]) with its description
/// written as a double-quoted string, so a strict YAML reader takes it as
/// the text it is whatever it holds (`: `, a leading `"`, `[`, `&`, `*`, `!`,
/// `|`, `>`, `%`, `@`, a backtick, a trailing `:`, ` #`). The same draft
/// comes out unchanged when quoted again.
pub fn quote_description(draft: &str) -> Result<String, SkillDraftError> {
    let mut out = String::with_capacity(draft.len() + 8);
    for (index, line) in draft.split_inclusive('\n').enumerate() {
        match line.strip_prefix("description:") {
            Some(value) if index == 2 => {
                let text = description_text(value.trim_end_matches('\n'))?;
                let quoted =
                    serde_json::to_string(&text).map_err(|_| SkillDraftError::Description)?;
                out.push_str("description: ");
                out.push_str(&quoted);
                out.push('\n');
            }
            _ => out.push_str(line),
        }
    }
    Ok(out)
}

/// The `name:` of a skill draft's header, checked only for its place.
pub fn skill_draft_name(draft: &str) -> Result<String, SkillDraftError> {
    let mut lines = draft.lines();
    if lines.next() != Some("---") {
        return Err(SkillDraftError::Frontmatter);
    }
    let name = lines
        .next()
        .and_then(|line| line.strip_prefix("name:"))
        .map(str::trim)
        .ok_or(SkillDraftError::Frontmatter)?;
    // A quoted name is not the plain slug the skill asks for.
    if !is_slug(name) {
        return Err(SkillDraftError::Name);
    }
    Ok(name.to_owned())
}

/// The body of one fenced block when `text` is exactly that block.
fn unfence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some(rest) = rest.strip_suffix("```") else {
        return text;
    };
    // The opening fence's language word, if any, up to the line break.
    match rest.split_once('\n') {
        Some((language, body)) if language.trim().chars().all(|c| c.is_ascii_alphanumeric()) => {
            body.trim()
        }
        _ => text,
    }
}

fn string(object: &Map<String, Value>, key: &'static str) -> Result<String, AnswerError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(AnswerError::NotAString(key))
}

/// Trimmed text of 1 to `cap` characters with no line break of any kind
/// ([`is_line_break`]) or other control character; tabs become spaces.
fn plain_line(text: &str, cap: usize) -> Option<String> {
    let text = text.trim().replace('\t', " ");
    (!text.is_empty()
        && text.chars().count() <= cap
        && !text.chars().any(|c| c.is_control() || is_line_break(c)))
    .then_some(text)
}

/// Whether the text names a message as `[n]`.
fn names_a_message(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(start, byte)| {
        *byte == b'['
            && bytes[start + 1..]
                .iter()
                .position(|b| *b == b']')
                .is_some_and(|len| {
                    len > 0
                        && len <= 4
                        && bytes[start + 1..start + 1 + len]
                            .iter()
                            .all(u8::is_ascii_digit)
                })
    })
}

/// Whether the draft holds something that looks like a personal detail or a
/// secret: an email address (also inside `KEY=value`, after a `:` as in
/// `mailto:`, or as a link's sign-in), a link with a sign-in
/// (`https://user:pass@host`), a private key block, a Slack webhook, a
/// common key or token prefix followed by a long run of key characters, a
/// JWT, or an AWS secret key's shape. Each word's trailing punctuation is
/// ignored. Passwords in other shapes are not recognised.
pub fn has_secret(draft: &str) -> bool {
    if draft.contains("-----BEGIN") && draft.contains("PRIVATE KEY") {
        return true;
    }
    if draft.contains("hooks.slack.com/services/") {
        return true;
    }
    let separator = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | '(' | ')' | ',' | '<' | '>' | '{' | '}'
            )
    };
    draft.split(separator).map(trim_punctuation).any(|word| {
        if is_email(word) || has_link_sign_in(word) {
            return true;
        }
        // Inside assignments and queries: `EMAIL=ana@…`, `GH=ghp_…`,
        // `?to=ana@…&x`.
        word.split(['=', '?', '&', ';'])
            .map(trim_punctuation)
            .any(|part| {
                let mut pieces = part.split(':').map(trim_punctuation);
                // The part before a `:` may be an SSH remote's `git@host`; an
                // address after one (`mailto:`, `owner:`) is a person's.
                let first = pieces.next().unwrap_or_default();
                is_email(part)
                    || is_key(first)
                    || pieces.any(|piece| is_email(piece) || is_key(piece))
            })
    })
}

/// A word without the punctuation a sentence puts around it.
fn trim_punctuation(word: &str) -> &str {
    word.trim_start_matches(['(', '[', '{', '<', '"', '\''])
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '"', '\''])
}

/// A key or token: a known prefix and a long run of key characters, a JWT,
/// or an AWS secret key's shape.
fn is_key(word: &str) -> bool {
    const PREFIXES: [(&str, usize); 15] = [
        ("sk-", 16),
        ("sk_live_", 16),
        ("ghp_", 16),
        ("gho_", 16),
        ("ghu_", 16),
        ("ghs_", 16),
        ("ghr_", 16),
        ("github_pat_", 16),
        ("xoxb-", 16),
        ("xoxp-", 16),
        ("AKIA", 16),
        ("glpat-", 20),
        ("AIza", 30),
        ("npm_", 30),
        ("hf_", 30),
    ];
    let key_chars = |text: &str| {
        text.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    };
    if PREFIXES.iter().any(|(prefix, min)| {
        word.strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= *min && key_chars(rest))
    }) {
        return true;
    }
    is_jwt(word) || is_aws_secret(word)
}

/// Three dot-separated base64url parts, the first a JSON header (`eyJ`).
fn is_jwt(word: &str) -> bool {
    let parts: Vec<&str> = word.split('.').collect();
    word.starts_with("eyJ")
        && parts.len() == 3
        && parts[0].len() >= 10
        && parts[1].len() >= 10
        && !parts[2].is_empty()
        && parts.iter().all(|part| {
            part.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
}

/// Forty characters of base64 with upper and lower case letters and a digit
/// (a lowercase hexadecimal commit ID never matches).
fn is_aws_secret(word: &str) -> bool {
    word.len() == 40
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '+'))
        && word.chars().any(|c| c.is_ascii_uppercase())
        && word.chars().any(|c| c.is_ascii_lowercase())
        && word.chars().any(|c| c.is_ascii_digit())
}

/// A link whose address carries a sign-in: `scheme://user:pass@host`, or an
/// email address as its user (`https://ana@example.com`).
fn has_link_sign_in(word: &str) -> bool {
    let Some((_, rest)) = word.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.rsplit_once('@') {
        Some((user, _)) => user.contains(':') || is_email(authority),
        None => false,
    }
}

fn is_email(word: &str) -> bool {
    let word = word.trim_matches(|c: char| matches!(c, '.' | ';' | ':' | '[' | ']'));
    let Some((local, domain)) = word.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'))
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SKILL: &str = "---\nname: shop-api-integration-tests\ndescription: Use when running \
                         the integration tests in shop-api.\n---\n\n# Run shop-api integration \
                         tests\n\n1. Start the test database: `docker compose up -d db`.\n2. \
                         Apply migrations: `make migrate`.\n3. Run the tests.";

    fn answer(suggestion: &str, real: &str, draft: &str) -> String {
        json!({
            "what_happened": "The agent ran the tests without the database.",
            "real_problem": real,
            "suggestion": suggestion,
            "draft": draft,
            "why": "[4] and [6] show the same error."
        })
        .to_string()
    }

    #[test]
    fn the_skills_own_examples_are_accepted() {
        let sample = include_str!("../../session-diagnosis/examples/sample-output.json");
        let parsed = parse(sample).unwrap();
        assert_eq!(parsed.suggestion, Suggestion::MakeSkill);
        assert_eq!(parsed.real_problem, RealProblem::Yes);
        assert_eq!(
            parsed.skill_name().as_deref(),
            Some("field-notes-e2e-android")
        );
        let skill = parse(&answer("make_skill", "yes", SKILL)).unwrap();
        assert!(skill.draft.ends_with("3. Run the tests.\n"));
        let line = parse(&answer(
            "add_to_claude_md",
            "yes",
            "This project uses pnpm: run pnpm install and pnpm scripts, never npm.",
        ))
        .unwrap();
        assert_eq!(line.suggestion, Suggestion::AddToClaudeMd);
        assert_eq!(line.skill_name(), None);
        let nothing = parse(&answer("nothing", "no", "")).unwrap();
        assert_eq!(nothing.draft, "");
        assert!(parse(&answer("nothing", "not_sure", "")).is_ok());
    }

    #[test]
    fn one_fenced_block_is_accepted_and_nothing_else_around_the_object() {
        let body = answer("nothing", "no", "");
        assert!(parse(&format!("```json\n{body}\n```")).is_ok());
        assert!(parse(&format!("```\n{body}\n```\n")).is_ok());
        assert_eq!(
            parse(&format!("Here it is:\n{body}")),
            Err(AnswerError::NotOneObject)
        );
        assert_eq!(
            parse(&format!("{body}\nDone.")),
            Err(AnswerError::NotOneObject)
        );
        assert_eq!(
            parse(&format!("```json\n{body}\n```\nmore")),
            Err(AnswerError::NotOneObject)
        );
        assert_eq!(parse("  "), Err(AnswerError::Empty));
        assert_eq!(parse("[1]"), Err(AnswerError::NotOneObject));
        assert_eq!(
            parse(&"x".repeat(MAX_ANSWER_BYTES + 1)),
            Err(AnswerError::TooLarge)
        );
    }

    #[test]
    fn each_rule_of_the_skill_is_checked() {
        let mut extra: Value = serde_json::from_str(&answer("nothing", "no", "")).unwrap();
        extra["more"] = json!("x");
        assert_eq!(parse(&extra.to_string()), Err(AnswerError::Keys));
        let mut missing = extra.clone();
        missing.as_object_mut().unwrap().remove("more");
        missing.as_object_mut().unwrap().remove("why");
        assert_eq!(parse(&missing.to_string()), Err(AnswerError::Keys));
        let mut number = extra;
        number.as_object_mut().unwrap().remove("more");
        number["why"] = json!(3);
        assert_eq!(
            parse(&number.to_string()),
            Err(AnswerError::NotAString("why"))
        );
        assert_eq!(
            parse(&answer("nothing", "maybe", "")),
            Err(AnswerError::RealProblem)
        );
        assert_eq!(
            parse(&answer("fix_it", "yes", "x")),
            Err(AnswerError::Suggestion)
        );
        assert_eq!(
            parse(&answer("make_skill", "no", SKILL)),
            Err(AnswerError::SuggestionWithoutProblem)
        );
        assert_eq!(
            parse(&answer("add_to_claude_md", "not_sure", "use pnpm")),
            Err(AnswerError::SuggestionWithoutProblem)
        );
        assert_eq!(
            parse(&answer("nothing", "yes", "something")),
            Err(AnswerError::DraftEmptiness)
        );
        assert_eq!(
            parse(&answer("add_to_claude_md", "yes", " ")),
            Err(AnswerError::DraftEmptiness)
        );
        let long_line = "a".repeat(MAX_LINE_CHARS + 1);
        assert_eq!(
            parse(&answer("add_to_claude_md", "yes", &long_line)),
            Err(AnswerError::Line)
        );
        assert_eq!(
            parse(&answer("add_to_claude_md", "yes", "one\ntwo")),
            Err(AnswerError::Line)
        );
        assert!(
            parse(&answer(
                "add_to_claude_md",
                "yes",
                &"a".repeat(MAX_LINE_CHARS)
            ))
            .is_ok()
        );
        let mut long: Value = serde_json::from_str(&answer("nothing", "no", "")).unwrap();
        long["what_happened"] = json!("w".repeat(MAX_WHAT_HAPPENED_CHARS + 1));
        assert_eq!(parse(&long.to_string()), Err(AnswerError::WhatHappened));
        long["what_happened"] = json!("ok");
        long["why"] = json!("no message numbers here");
        assert_eq!(parse(&long.to_string()), Err(AnswerError::Why));
        long["why"] = json!(format!("[1] {}", "w".repeat(MAX_WHY_CHARS)));
        assert_eq!(parse(&long.to_string()), Err(AnswerError::Why));
        long["why"] = json!("line one [1]\nline two");
        assert_eq!(parse(&long.to_string()), Err(AnswerError::Why));
    }

    #[test]
    fn a_skill_draft_needs_its_header_a_valid_name_a_title_and_bounds() {
        let refused = |draft: &str| parse(&answer("make_skill", "yes", draft));
        assert_eq!(
            refused("# Title\n1. step"),
            Err(AnswerError::Skill(SkillDraftError::Frontmatter))
        );
        assert_eq!(
            refused(&SKILL.replace("shop-api-integration-tests", "Shop_API")),
            Err(AnswerError::Skill(SkillDraftError::Name))
        );
        assert_eq!(
            refused(&SKILL.replace("shop-api-integration-tests", &"a".repeat(65))),
            Err(AnswerError::Skill(SkillDraftError::Name))
        );
        assert_eq!(
            refused(&SKILL.replace("shop-api-integration-tests", "-lead")),
            Err(AnswerError::Skill(SkillDraftError::Name))
        );
        assert_eq!(
            refused(&SKILL.replace("description: Use when running", "summary: x")),
            Err(AnswerError::Skill(SkillDraftError::Frontmatter))
        );
        assert_eq!(
            refused(&SKILL.replace("# Run shop-api", "Run shop-api")),
            Err(AnswerError::Skill(SkillDraftError::Body))
        );
        let long = format!("{SKILL}{}", "\n- more".repeat(MAX_SKILL_LINES));
        assert_eq!(
            refused(&long),
            Err(AnswerError::Skill(SkillDraftError::TooLong))
        );
        assert_eq!(
            refused(&SKILL.replace("Apply", "Apply\u{1b}")),
            Err(AnswerError::Skill(SkillDraftError::Control))
        );
        // Windows line breaks are accepted and stored as `\n`.
        let crlf = parse(&answer("make_skill", "yes", &SKILL.replace('\n', "\r\n"))).unwrap();
        assert!(!crlf.draft.contains('\r'));
    }

    #[test]
    fn every_kind_of_line_break_is_a_line_break() {
        for brk in ['\u{2028}', '\u{2029}', '\u{85}', '\u{0B}', '\u{0C}'] {
            let two = format!("Use pnpm.{brk}Never npm.");
            assert_eq!(
                parse(&answer("add_to_claude_md", "yes", &two)),
                Err(AnswerError::Line),
                "{brk:?}"
            );
            let mut why: Value = serde_json::from_str(&answer("nothing", "no", "")).unwrap();
            why["why"] = json!(format!("[1] one{brk}two"));
            assert_eq!(parse(&why.to_string()), Err(AnswerError::Why), "{brk:?}");
            why["why"] = json!("[1] fine");
            why["what_happened"] = json!(format!("one{brk}two"));
            assert_eq!(
                parse(&why.to_string()),
                Err(AnswerError::WhatHappened),
                "{brk:?}"
            );
            // In a skill they are line breaks: stored as `\n` and counted.
            let skill = parse(&answer(
                "make_skill",
                "yes",
                &SKILL.replace('\n', &brk.to_string()),
            ))
            .unwrap();
            assert!(!skill.draft.contains(brk));
            let long = format!("{SKILL}{}", format!("{brk}- more").repeat(MAX_SKILL_LINES));
            assert_eq!(
                parse(&answer("make_skill", "yes", &long)),
                Err(AnswerError::Skill(SkillDraftError::TooLong)),
                "{brk:?}"
            );
            // Given to the checker as they are, they count and are refused.
            let raw = format!("{}{brk}x\n", SKILL.trim_end());
            assert_eq!(check_skill_draft(&raw), Err(SkillDraftError::Control));
            let raw_long = format!("{SKILL}\n{}", format!("x{brk}").repeat(MAX_SKILL_LINES));
            assert_eq!(check_skill_draft(&raw_long), Err(SkillDraftError::TooLong));
        }
    }

    /// The `description:` value of a draft's header, read as YAML reads a
    /// double-quoted scalar (a JSON string is one).
    fn written_description(draft: &str) -> String {
        let line = draft.lines().nth(2).unwrap();
        let value = line
            .strip_prefix("description: \"")
            .map(|_| &line[13..])
            .unwrap();
        serde_json::from_str(value).unwrap()
    }

    #[test]
    fn the_description_is_written_as_a_quoted_string() {
        for (written, text) in [
            (
                "Use when running: the integration tests.",
                "Use when running: the integration tests.",
            ),
            ("\"Quoted already.\"", "Quoted already."),
            ("'It''s quoted.'", "It's quoted."),
            ("[not a list] when testing.", "[not a list] when testing."),
            (
                "&anchor *alias !tag |pipe >fold %dir @at `tick`",
                "&anchor *alias !tag |pipe >fold %dir @at `tick`",
            ),
            ("Use for builds #1 and #2", "Use for builds #1 and #2"),
            ("Ends with a colon:", "Ends with a colon:"),
            (
                "A \\ backslash and a \"quote\" inside.",
                "A \\ backslash and a \"quote\" inside.",
            ),
        ] {
            let draft = SKILL.replace(
                "description: Use when running the integration tests in shop-api.",
                &format!("description: {written}"),
            );
            let parsed = parse(&answer("make_skill", "yes", &draft)).unwrap();
            assert_eq!(written_description(&parsed.draft), text, "{written}");
            assert!(
                parsed
                    .draft
                    .lines()
                    .nth(2)
                    .unwrap()
                    .starts_with("description: \"")
            );
            // The rest of the draft is unchanged, and quoting again changes
            // nothing.
            assert_eq!(
                parsed.draft.lines().count(),
                normalise_draft(&draft).lines().count()
            );
            assert_eq!(quote_description(&parsed.draft).unwrap(), parsed.draft);
            assert_eq!(check_skill_draft(&parsed.draft), Ok(()));
        }
        assert_eq!(
            parse(&answer(
                "make_skill",
                "yes",
                &SKILL.replace(
                    "description: Use when running the integration tests in shop-api.",
                    "description: 'it's broken.'"
                )
            )),
            Err(AnswerError::Skill(SkillDraftError::Description))
        );
    }

    #[test]
    fn personal_details_and_secrets_are_refused_in_a_draft() {
        // Build synthetic token samples without storing complete tokens in source.
        let registry = ["Use glpat-", "abcdefghij", "klmnopqrstu for the registry."].concat();
        let header = [
            "Send ",
            "eyJhbGciOiJIUzI1NiJ9",
            ".",
            "eyJzdWIiOiIxMjM0In0",
            ".",
            "c2lnbmF0dXJlLXBhcnQ",
            " as the header.",
        ]
        .concat();
        for draft in [
            "Ask ana@example.com before deploying.",
            "Use the key sk-abcdefghijklmnopqrstuvwx to call the API.",
            "export GH=ghp_abcdefghijklmnopqrstuvwxyz0123",
            // Trailing punctuation does not hide a key.
            "Use the key sk-abcdefghijklmnopqrstuvwx.",
            "Set GH=ghp_abcdefghijklmnopqrstuvwxyz0123, then push.",
            // Addresses inside assignments, after a colon, and in links.
            "Set EMAIL=ana@example.com in .env first.",
            "Write to mailto:ana@example.com when it fails.",
            "Ask owner:ana@example.com before merging.",
            "Clone https://ana:hunter2@git.example.com/repo.git once.",
            "Open https://ana@example.com/dashboard to check.",
            // More token shapes.
            registry.as_str(),
            "Call it with AIzaSyA-abcdefghijklmnopqrstuvwxyz01234 set.",
            header.as_str(),
            "Set AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY first.",
            "Publish with npm_abcdefghijklmnopqrstuvwxyz0123456789 set.",
            "Post to https://hooks.slack.com/services/T000/B000/XXXX when done.",
        ] {
            assert_eq!(
                parse(&answer("add_to_claude_md", "yes", draft)),
                Err(AnswerError::Secret),
                "{draft}"
            );
        }
        // Commands, files and scoped package names are fine.
        for draft in [
            "Run pnpm --filter @app/web test before pushing.",
            "Use git@github.com:org/repo.git only through gh.",
            "Read docs/sk-notes.md first.",
            // A commit ID, a short `eyJ` word and a plain link.
            "Check out 0123456789abcdef0123456789abcdef01234567 first.",
            "The header starts with eyJ in base64.",
            "See https://example.com/docs/setup.html before you start.",
            "Run git clone ssh://github.com/org/repo.git once.",
        ] {
            assert!(!has_secret(draft), "{draft}");
        }
    }

    #[test]
    fn the_retry_note_carries_the_reason() {
        let note = retry_note(&AnswerError::Line);
        assert!(note.contains("at most 200 characters"));
        assert!(note.contains("last try"));
    }
}
