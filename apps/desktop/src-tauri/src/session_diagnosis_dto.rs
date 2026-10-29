//! Generated IPC shapes for diagnosing a warning and saving a suggested
//! skill, and every sentence the screen shows about them.
//!
//! What the model answered is the model's; everything else here — which
//! tool ran, which messages left the computer, why a tool was skipped, what
//! is known to reach a tool beyond the prompt — is worded once here, so the
//! warnings UI shows it and works none of it out.
use serde::Serialize;
use ts_rs::TS;
use xt_judge::diagnosis::{
    agents::{Limit, RunFailure, Skip, Tool, Tried},
    answer::{RealProblem, Suggestion},
    input,
};

/// A coding tool a diagnosis can run with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisTool {
    ClaudeCode,
    Codex,
    Cursor,
}

impl From<Tool> for DiagnosisTool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::ClaudeCode => Self::ClaudeCode,
            Tool::Codex => Self::Codex,
            Tool::Cursor => Self::Cursor,
        }
    }
}

/// The tool's name as the screen says it.
pub fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::ClaudeCode => "Claude Code",
        Tool::Codex => "Codex",
        Tool::Cursor => "Cursor",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisRealProblem {
    Yes,
    No,
    NotSure,
}

impl From<RealProblem> for DiagnosisRealProblem {
    fn from(value: RealProblem) -> Self {
        match value {
            RealProblem::Yes => Self::Yes,
            RealProblem::No => Self::No,
            RealProblem::NotSure => Self::NotSure,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisSuggestion {
    MakeSkill,
    AddToClaudeMd,
    Nothing,
}

/// What the draft is, so the screen knows which action it offers: save a
/// skill (`save_suggested_skill`), copy one instruction line, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisDraftKind {
    /// A whole `SKILL.md`.
    Skill,
    /// One line for the project's instruction file, to copy.
    InstructionLine,
    None,
}

impl From<Suggestion> for DiagnosisSuggestion {
    fn from(value: Suggestion) -> Self {
        match value {
            Suggestion::MakeSkill => Self::MakeSkill,
            Suggestion::AddToClaudeMd => Self::AddToClaudeMd,
            Suggestion::Nothing => Self::Nothing,
        }
    }
}

pub fn draft_kind(suggestion: Suggestion) -> DiagnosisDraftKind {
    match suggestion {
        Suggestion::MakeSkill => DiagnosisDraftKind::Skill,
        Suggestion::AddToClaudeMd => DiagnosisDraftKind::InstructionLine,
        Suggestion::Nothing => DiagnosisDraftKind::None,
    }
}

/// Why a tool was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DiagnosisSkip {
    NotInstalled,
    TooOld,
    NotSignedIn,
    CouldNotStart,
    Failed {
        code: Option<i32>,
    },
    TimedOut,
    /// Codex: its shell could not surely be turned off, so it was not run.
    ShellNotOff,
}

impl From<Skip> for DiagnosisSkip {
    fn from(skip: Skip) -> Self {
        match skip {
            Skip::NotInstalled => Self::NotInstalled,
            Skip::TooOld => Self::TooOld,
            Skip::NotSignedIn => Self::NotSignedIn,
            Skip::CouldNotStart => Self::CouldNotStart,
            Skip::Failed { code } => Self::Failed { code },
            Skip::TimedOut => Self::TimedOut,
            Skip::ShellNotOff => Self::ShellNotOff,
        }
    }
}

/// One tool that was not used, and why, in words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DiagnosisToolTried {
    pub tool: DiagnosisTool,
    pub skip: DiagnosisSkip,
    /// Whether the messages were given to it before it failed.
    pub given_messages: bool,
    /// "Claude Code is not signed in. Run `claude` and log in." …
    pub text: String,
}

/// One skipped tool in words; with `instead`, the tool that then answered
/// is named ("Claude Code is not signed in. … Codex was used instead.").
pub fn tried(tried: &Tried, instead: Option<Tool>) -> DiagnosisToolTried {
    let name = tool_name(tried.tool);
    let mut text = match tried.skip {
        Skip::NotInstalled => format!("{name} is not installed."),
        Skip::TooOld => format!("{name} is older than XTrace needs. Update it."),
        Skip::NotSignedIn => not_signed_in_text(tried.tool).to_owned(),
        Skip::CouldNotStart => format!("{name} could not be started."),
        Skip::Failed { .. } => format!("{name} failed its check before the run."),
        Skip::TimedOut => format!("{name} did not finish its check in time."),
        Skip::ShellNotOff => {
            format!("{name} was not used: XTrace could not make sure its shell tool is turned off.")
        }
    };
    if let Some(used) = instead {
        text.push_str(&format!(" {} was used instead.", tool_name(used)));
    }
    DiagnosisToolTried {
        tool: tried.tool.into(),
        skip: tried.skip.into(),
        given_messages: tried.given_prompt,
        text,
    }
}

/// How to sign a tool in.
fn not_signed_in_text(tool: Tool) -> &'static str {
    match tool {
        Tool::ClaudeCode => "Claude Code is not signed in. Run `claude` and log in.",
        Tool::Codex => "Codex is not signed in. Run `codex login`.",
        Tool::Cursor => "Cursor's agent is not signed in. Run `cursor-agent login`.",
    }
}

/// The sentence for a run that the tool given the messages ended.
pub fn run_failed_text(tool: Tool, failure: RunFailure) -> String {
    match failure {
        RunFailure::NotSignedIn => format!(
            "{} The messages were not given to another tool.",
            not_signed_in_text(tool)
        ),
        _ => RUN_FAILED.to_owned(),
    }
}

/// Something known to reach a tool beyond the prompt, or not seen working.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DiagnosisLimit {
    pub kind: DiagnosisLimitKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisLimitKind {
    CodexLoadsUserInstructions,
    CursorPromptOnCommandLine,
    CursorUnverified,
}

pub fn limit(limit: Limit) -> DiagnosisLimit {
    match limit {
        Limit::CodexLoadsUserInstructions => DiagnosisLimit {
            kind: DiagnosisLimitKind::CodexLoadsUserInstructions,
            text: "Codex also reads your own ~/.codex/AGENTS.md and the list of skills you \
                   installed for it."
                .to_owned(),
        },
        Limit::CursorPromptOnCommandLine => DiagnosisLimit {
            kind: DiagnosisLimitKind::CursorPromptOnCommandLine,
            text: "Cursor gets the messages on its command line, where other programs on this \
                   Mac can see them while it runs."
                .to_owned(),
        },
        Limit::CursorUnverified => DiagnosisLimit {
            kind: DiagnosisLimitKind::CursorUnverified,
            text: "Diagnosing with Cursor has not been tested end to end yet.".to_owned(),
        },
    }
}

/// Which messages left this computer, and to which tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DiagnosisSent {
    /// Messages sent.
    pub messages: u32,
    /// The flagged message's number among them, from 1.
    pub flagged: u32,
    /// How many of them were cut because they were long.
    pub cut: u32,
    #[ts(type = "number | null")]
    pub first_at_ms: Option<i64>,
    #[ts(type = "number | null")]
    pub last_at_ms: Option<i64>,
    /// Every tool they were given to, the one that answered last.
    pub tools: Vec<DiagnosisTool>,
    /// "20 messages from this session (14:02 to 14:31) were sent to Claude
    /// Code."
    pub text: String,
}

/// The exact bounded message body supplied to this diagnosis, not a fresh source read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DiagnosisMessage {
    pub number: u32,
    pub label: String,
    pub text: String,
    pub cut: bool,
    pub flagged: bool,
    #[ts(type = "number | null")]
    pub at_ms: Option<i64>,
}

/// Only references validated against this run's sent messages can be opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosisReferencePart {
    Text { text: String },
    Message { text: String, number: u32 },
    Unavailable { text: String },
}

pub fn reference_parts(text: &str, messages: &[input::SentMessage]) -> Vec<DiagnosisReferencePart> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || after.as_bytes().get(digits) != Some(&b']') {
            let end = start + 1;
            parts.push(DiagnosisReferencePart::Text {
                text: rest[..end].to_owned(),
            });
            rest = &rest[end..];
            continue;
        }
        if start > 0 {
            parts.push(DiagnosisReferencePart::Text {
                text: rest[..start].to_owned(),
            });
        }
        let end = start + digits + 2;
        let token = rest[start..end].to_owned();
        let number = after[..digits].parse::<u32>().ok().filter(|number| {
            messages
                .iter()
                .any(|message| message.number == *number as usize)
        });
        parts.push(match number {
            Some(number) => DiagnosisReferencePart::Message {
                text: token,
                number,
            },
            None => DiagnosisReferencePart::Unavailable { text: token },
        });
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        parts.push(DiagnosisReferencePart::Text {
            text: rest.to_owned(),
        });
    }
    parts
}

pub fn message_dto(message: input::SentMessage) -> DiagnosisMessage {
    let flagged = if message.flagged { " · flagged" } else { "" };
    DiagnosisMessage {
        number: message.number as u32,
        label: format!(
            "Message {} · {}{flagged}",
            message.number, message.sender_label
        ),
        text: message.text,
        cut: message.cut,
        flagged: message.flagged,
        at_ms: message.at_ms,
    }
}

/// One diagnosis, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionDiagnosis {
    Diagnosed {
        tool: DiagnosisTool,
        /// The model that answered, as the tool names it, only when the
        /// tool said so unambiguously; never guessed.
        model: Option<String>,
        /// The model the run asked for: `haiku`, a Codex model or `default`.
        asked_model: String,
        /// Which tool and model answered, in words ([`ran_text`]).
        ran: String,
        /// 1, or 2 when the first answer broke a rule.
        tries: u32,
        what_happened: String,
        real_problem: DiagnosisRealProblem,
        /// The native-written verdict, distinct from the suggested next action.
        real_problem_text: String,
        suggestion: DiagnosisSuggestion,
        /// The suggested next action, without inferring why no change was suggested.
        suggestion_text: String,
        draft_kind: DiagnosisDraftKind,
        /// The whole `SKILL.md` for a skill, the line for an instruction
        /// line, empty otherwise.
        draft: String,
        /// The skill's name from its draft, for a skill.
        skill_name: Option<String>,
        /// Where saving the skill writes it:
        /// `<project>/.claude/skills/<name>/SKILL.md`, for a skill in a
        /// session whose project folder is known.
        skill_path: Option<String>,
        /// For a skill from a session not run in Claude Code: that
        /// `.claude/skills` is read by Claude Code only.
        skill_note: Option<String>,
        /// For an instruction line: the file it belongs in for the session's
        /// tool (`CLAUDE.md` for Claude Code, `AGENTS.md` for Codex); `null`
        /// when XTrace does not know one.
        instruction_file: Option<String>,
        why: String,
        why_parts: Vec<DiagnosisReferencePart>,
        messages: Vec<DiagnosisMessage>,
        sent: Box<DiagnosisSent>,
        tried: Vec<DiagnosisToolTried>,
        limits: Vec<DiagnosisLimit>,
    },
    /// No coding tool could run it. `sent` says whether any tool was still
    /// given the messages before failing.
    NoAgentTool {
        tried: Vec<DiagnosisToolTried>,
        sent: Option<DiagnosisSent>,
        text: String,
    },
    /// The tool answered twice and broke the rules both times.
    AnswerRefused {
        tool: DiagnosisTool,
        model: Option<String>,
        asked_model: String,
        /// Which tool and model ran, in words ([`ran_text`]).
        ran: String,
        /// The rule the last answer broke.
        reason: String,
        sent: DiagnosisSent,
        tried: Vec<DiagnosisToolTried>,
        text: String,
    },
    /// The tool given the messages failed or timed out; no other tool was
    /// asked, so the messages went to one company only.
    RunFailed {
        tool: DiagnosisTool,
        model: Option<String>,
        asked_model: String,
        /// Which tool and model ran, in words ([`ran_text`]).
        ran: String,
        reason: String,
        sent: DiagnosisSent,
        tried: Vec<DiagnosisToolTried>,
        text: String,
    },
    /// The session's original source could not be read; nothing was sent.
    SourceUnavailable {
        reason: crate::dto::SessionSourceReason,
        text: String,
    },
    /// The session has no such warning now; nothing was sent.
    WarningNotFound { text: String },
    /// The flagged message is not in the session's source; nothing was sent.
    FlaggedMessageNotFound { text: String },
    /// Another diagnosis is running; nothing was started.
    Busy { text: String },
    /// Cancelled; any tool still running was stopped.
    Cancelled { text: String },
    /// The run's private folder could not be prepared; nothing was sent.
    Failed { text: String },
}

/// Which tool and model ran, in words: the model only as the tool named it,
/// otherwise the one asked for. `answered` is false when no usable answer
/// came back.
pub fn ran_text(tool: Tool, asked: &str, answered_model: Option<&str>, answered: bool) -> String {
    let name = tool_name(tool);
    match (answered, answered_model) {
        (true, Some(model)) => format!("Answered by {name}, model {model}."),
        (true, None) => {
            format!("Answered by {name}; it did not say which model answered (asked for {asked}).")
        }
        (false, Some(model)) => format!("Ran with {name}, model {model}."),
        (false, None) => format!("Ran with {name} (asked for {asked})."),
    }
}

/// The native-written verdict, distinct from the suggested next action.
pub fn real_problem_text(value: RealProblem) -> &'static str {
    match value {
        RealProblem::Yes => "The model found a problem",
        RealProblem::No => "The model found no problem",
        RealProblem::NotSure => "The messages were not enough to tell",
    }
}

/// The suggested next action, without inferring why no change was suggested.
pub fn suggestion_text(value: Suggestion) -> &'static str {
    match value {
        Suggestion::MakeSkill => "Save a skill to help prevent this next time.",
        Suggestion::AddToClaudeMd => "Copy this line into the project's instructions.",
        Suggestion::Nothing => "No skill or instruction change suggested.",
    }
}

/// Every fixed word the Diagnose controls on a session page show, exported
/// with the warnings' words (`SessionWarningsText.ts`, `diagnose`).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct DiagnoseText {
    /// A warning's button that starts a diagnosis.
    pub button: &'static str,
    /// What the button is named for a screen reader; `{label}` is the
    /// warning's label.
    pub button_name: &'static str,
    /// The button that sends the messages after the sentence before sending.
    pub confirm: &'static str,
    /// Closes the sentence before sending without sending anything.
    pub not_now: &'static str,
    /// While a diagnosis runs.
    pub running: &'static str,
    /// Stops a running diagnosis.
    pub stop: &'static str,
    /// Puts a result away.
    pub close: &'static str,
    /// The heading over a result.
    pub heading: &'static str,
    pub what_happened: &'static str,
    pub why: &'static str,
    pub details: &'static str,
    pub next_action: &'static str,
    pub evidence: &'static str,
    pub evidence_note: &'static str,
    pub reference_unavailable: &'static str,

    /// Over the `sent` sentence and the tools tried.
    pub sent: &'static str,
    /// Over the skill draft.
    pub skill_draft: &'static str,
    /// Before the path a skill is saved to.
    pub skill_path: &'static str,
    /// Said when a skill has a name but the session names no project
    /// folder, so there is nowhere to save it.
    pub skill_no_path: &'static str,
    pub save_skill: &'static str,
    pub saving: &'static str,
    /// Said when saving could not be asked for at all.
    pub save_failed: &'static str,
    /// Over the instruction line; `{file}` is the instruction file.
    pub line_for_file: &'static str,
    /// Over the instruction line when no file is known.
    pub line_no_file: &'static str,
    pub copy: &'static str,
    pub copied: &'static str,
    pub copy_failed: &'static str,
    /// Over the rule an answer broke, or why a run stopped.
    pub reason: &'static str,
    /// Said when the diagnosis could not be asked for at all.
    pub failed: &'static str,
    /// A warning dot's hint while its diagnosis runs; `{elapsed}` is the
    /// time since it was sent, as `m:ss`.
    pub dot_running: &'static str,
    /// Added to a warning dot's name while its diagnosis runs.
    pub dot_running_name: &'static str,
    /// A warning dot's hint, and an addition to its name, once its answer is
    /// kept: whatever it says, the diagnosis has finished (a run you stopped
    /// is not shown).
    pub dot_finished: &'static str,
    /// Beside a waiting Diagnose button while another warning's diagnosis
    /// runs: one runs at a time.
    pub busy: &'static str,
}

pub const DIAGNOSE_TEXT: DiagnoseText = DiagnoseText {
    button: "Diagnose",
    button_name: "Diagnose this warning: {label}",
    confirm: "Send & diagnose",
    not_now: "Not now",
    running: "Diagnosing… This can take a minute.",
    stop: "Stop",
    close: "Hide diagnosis",
    heading: "Diagnosis result",
    what_happened: "What happened",
    why: "Supporting evidence",
    details: "Details",
    next_action: "Next action",
    evidence: "View supporting messages",
    evidence_note: "These are the numbered messages sent for this diagnosis. Long messages were shortened before sending; [cut] marks the end of shortened text.",
    reference_unavailable: "This reference does not name a message sent for this diagnosis.",
    sent: "What was sent",
    skill_draft: "Suggested skill (SKILL.md)",
    skill_path: "Saves to",
    skill_no_path: "This session does not say which project it ran in, so the skill cannot be saved from here.",
    save_skill: "Save as skill",
    saving: "Saving…",
    save_failed: "The skill could not be saved. Nothing was written.",
    line_for_file: "Suggested line for {file}",
    line_no_file: "Suggested line for the project's instruction file",
    copy: "Copy",
    copied: "Copied",
    copy_failed: "Could not copy. Select the line and copy it yourself.",
    reason: "Reason",
    failed: "The diagnosis could not be run. Try again.",
    dot_running: "Diagnosing… {elapsed}",
    dot_running_name: "Diagnosing",
    dot_finished: "Diagnosis finished",
    busy: "Another warning is being diagnosed. One diagnosis runs at a time.",
};

/// Fixed sentences.
pub const NO_TOOL: &str =
    "No coding tool could run the diagnosis. Install Claude Code or Codex and sign in.";
pub const REFUSED: &str = "The diagnosis gave no usable answer. Try again.";
pub const RUN_FAILED: &str = "The diagnosis stopped before it answered. Try again.";
pub const SOURCE_UNAVAILABLE: &str =
    "This session's messages could not be read, so nothing was sent.";
pub const WARNING_NOT_FOUND: &str = "This warning is no longer in the session.";
pub const FLAGGED_NOT_FOUND: &str =
    "The flagged message is not in this session's file, so nothing was sent.";
pub const BUSY: &str = "Another diagnosis is running. Wait for it to finish.";
pub const CANCELLED: &str = "The diagnosis was stopped.";
pub const PREPARE_FAILED: &str = "The diagnosis could not start, so nothing was sent.";
/// What a Diagnose click sends, said before the click. It matches
/// `agents::ORDER` and the input's caps ([`input::MESSAGES_BEFORE`],
/// [`input::MESSAGES_AFTER`], [`input::MAX_MESSAGES_CHARS`]); a test checks.
pub const BEFORE_SENDING: &str = "About 20 messages around this warning (at most 24,000 characters) \
     will be sent to Claude Code's hosted model, or Codex's if Claude Code is not available.";
/// The note for a skill from a session not run in Claude Code.
pub const SKILL_FOR_CLAUDE_CODE: &str =
    "Skills in .claude/skills are read by Claude Code. Codex does not read them.";

/// The instruction file an `add_to_claude_md` line belongs in for a
/// session's tool.
pub fn instruction_file(host: input::HostName) -> Option<&'static str> {
    match host {
        input::HostName::ClaudeCode => Some("CLAUDE.md"),
        input::HostName::Codex => Some("AGENTS.md"),
        input::HostName::Cursor | input::HostName::Other => None,
    }
}

/// The note on where a saved skill is read, for a session's tool.
pub fn skill_note(host: input::HostName) -> Option<&'static str> {
    match host {
        input::HostName::ClaudeCode => None,
        input::HostName::Codex => Some(SKILL_FOR_CLAUDE_CODE),
        input::HostName::Cursor | input::HostName::Other => {
            Some("Skills in .claude/skills are read by Claude Code.")
        }
    }
}

/// The sentence saying which messages left the computer and where.
pub fn sent_text(messages: usize, span: Option<(String, String)>, tools: &[Tool]) -> String {
    let count = if messages == 1 {
        "1 message".to_owned()
    } else {
        format!("{messages} messages")
    };
    let span = match span {
        Some((from, to)) if from == to => format!(" ({from})"),
        Some((from, to)) => format!(" ({from} to {to})"),
        None => String::new(),
    };
    let names: Vec<&str> = tools.iter().map(|tool| tool_name(*tool)).collect();
    let to = match names.as_slice() {
        [] => "no tool".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let verb = if messages == 1 { "was" } else { "were" };
    format!("{count} from this session{span} {verb} sent to {to}.")
}

/// The answer to `save_suggested_skill`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SavedSkill {
    Saved {
        /// The new `SKILL.md`.
        path: String,
        text: String,
    },
    Refused {
        reason: SaveSkillRefusal,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SaveSkillRefusal {
    /// The name is not a skill name.
    InvalidName,
    /// The draft is not a skill XTrace accepts.
    InvalidDraft,
    /// The draft's own `name:` is not the name given.
    NameMismatch,
    /// The draft holds an email address, key or token.
    Secret,
    /// No indexed session has this identifier.
    SessionNotFound,
    /// The session names no project folder.
    NoProjectFolder,
    /// The project folder is gone, is not a folder, or is the home folder.
    ProjectUnavailable,
    /// A skill with this name already exists there.
    AlreadyExists,
    /// The project's `.claude` or `.claude/skills` is a link or not a folder.
    UnsafeFolder,
    /// Writing failed; every folder this save created is removed again.
    WriteFailed,
}

pub fn refusal_text(reason: SaveSkillRefusal) -> &'static str {
    match reason {
        SaveSkillRefusal::InvalidName => {
            "A skill name uses only lowercase letters, digits and -, at most 64 characters."
        }
        SaveSkillRefusal::InvalidDraft => "This draft is not a skill XTrace can save.",
        SaveSkillRefusal::NameMismatch => "The draft's own name is not the name given.",
        SaveSkillRefusal::Secret => {
            "The draft contains an email address, key or token. Remove it first."
        }
        SaveSkillRefusal::SessionNotFound => "This Mac's index holds no such session.",
        SaveSkillRefusal::NoProjectFolder => "This session does not say which project it ran in.",
        SaveSkillRefusal::ProjectUnavailable => {
            "The session's project folder is missing, or is your home folder."
        }
        SaveSkillRefusal::AlreadyExists => "The project already has a skill with this name.",
        SaveSkillRefusal::UnsafeFolder => {
            "The project's .claude/skills folder is a link or not a folder, so nothing was written."
        }
        SaveSkillRefusal::WriteFailed => "The skill could not be written. Nothing was saved.",
    }
}

pub fn saved_text(name: &str) -> String {
    format!("Saved the skill {name} in this project.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_open_only_this_runs_numbered_messages_and_preserve_all_model_text() {
        let messages = vec![input::SentMessage {
            number: 1,
            sender_label: "person".into(),
            text: "evidence".into(),
            flagged: true,
            cut: false,
            at_ms: None,
        }];
        let text =
            "é [1], [1], [2], [0], [999999999999999999999] [x] [ 1] [1,2] https://example.test <b>";
        let parts = reference_parts(text, &messages);
        let references: Vec<_> = parts
            .iter()
            .filter_map(|part| match part {
                DiagnosisReferencePart::Message { number, .. } => Some(*number),
                _ => None,
            })
            .collect();
        assert_eq!(references, [1, 1]);
        assert_eq!(
            parts
                .iter()
                .filter(|part| matches!(part, DiagnosisReferencePart::Unavailable { .. }))
                .count(),
            3
        );
        let joined: String = parts
            .into_iter()
            .map(|part| match part {
                DiagnosisReferencePart::Text { text }
                | DiagnosisReferencePart::Message { text, .. }
                | DiagnosisReferencePart::Unavailable { text } => text,
            })
            .collect();
        assert_eq!(joined, text);
        assert!(matches!(
            reference_parts("[1]", &[]).as_slice(),
            [DiagnosisReferencePart::Unavailable { .. }]
        ));
    }

    #[test]
    fn verdict_and_next_action_do_not_invent_a_cause_for_a_problem_with_no_suggestion() {
        assert_eq!(
            real_problem_text(RealProblem::Yes),
            "The model found a problem"
        );
        assert_eq!(
            real_problem_text(RealProblem::No),
            "The model found no problem"
        );
        assert_eq!(
            real_problem_text(RealProblem::NotSure),
            "The messages were not enough to tell"
        );
        assert_eq!(
            suggestion_text(Suggestion::Nothing),
            "No skill or instruction change suggested."
        );
        assert!(suggestion_text(Suggestion::MakeSkill).contains("Save a skill"));
        assert!(suggestion_text(Suggestion::AddToClaudeMd).contains("Copy this line"));
    }

    #[test]
    fn the_sentence_before_sending_matches_the_order_and_the_caps() {
        assert_eq!(
            xt_judge::diagnosis::agents::ORDER,
            [Tool::ClaudeCode, Tool::Codex]
        );
        assert_eq!(input::MESSAGES_BEFORE + 1 + input::MESSAGES_AFTER, 20);
        assert_eq!(input::MAX_MESSAGES_CHARS, 24_000);
        assert!(BEFORE_SENDING.starts_with("About 20 messages"));
        assert!(BEFORE_SENDING.contains("at most 24,000 characters"));
        assert!(BEFORE_SENDING.contains("Claude Code's hosted model, or Codex's"));
        assert!(!BEFORE_SENDING.contains("Cursor"));
        assert!(!NO_TOOL.contains("Cursor"));
    }

    #[test]
    fn the_run_names_its_tool_and_only_a_model_the_tool_named() {
        assert_eq!(
            ran_text(Tool::ClaudeCode, "haiku", Some("claude-haiku-4-5"), true),
            "Answered by Claude Code, model claude-haiku-4-5."
        );
        assert_eq!(
            ran_text(Tool::Codex, "gpt-5-mini", None, true),
            "Answered by Codex; it did not say which model answered (asked for gpt-5-mini)."
        );
        assert_eq!(
            ran_text(Tool::ClaudeCode, "haiku", None, false),
            "Ran with Claude Code (asked for haiku)."
        );
        assert_eq!(
            real_problem_text(RealProblem::NotSure),
            "The messages were not enough to tell"
        );
        assert_eq!(
            suggestion_text(Suggestion::Nothing),
            "No skill or instruction change suggested."
        );
    }

    #[test]
    fn the_sent_sentence_says_how_many_when_and_to_which_tools() {
        assert_eq!(
            sent_text(
                20,
                Some(("14:02".into(), "14:31".into())),
                &[Tool::ClaudeCode]
            ),
            "20 messages from this session (14:02 to 14:31) were sent to Claude Code."
        );
        assert_eq!(
            sent_text(1, None, &[Tool::ClaudeCode, Tool::Codex]),
            "1 message from this session was sent to Claude Code and Codex."
        );
        assert_eq!(
            sent_text(
                3,
                Some(("9:00".into(), "9:00".into())),
                &[Tool::ClaudeCode, Tool::Codex, Tool::Cursor]
            ),
            "3 messages from this session (9:00) were sent to Claude Code, Codex and Cursor."
        );
    }
}
