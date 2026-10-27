//! Diagnose outcomes, exact conversation messages and explicit project skill saves.

use crate::session_diagnosis_dto::{
    self as dto, DiagnosisSent, SaveSkillRefusal, SavedSkill, SessionDiagnosis,
};
use crate::state::{AppState, StateError};
use crate::transcript_dto::{SourceBlock, SourcePayload, SourceRecord, SourceResultPart};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use xt_ingest::canonical::ParsedRecord;
use xt_judge::diagnosis::{
    agents::{self, DiagnoseError, Tool, Tried},
    answer,
    input::{self, HostName, Message, Sender},
};
use xt_metrics::WarningPoint;

/// Characters of a tool call's input shown in `(ran <tool>: <input>)`.
pub const MAX_CALL_INPUT_CHARS: usize = 200;

/// What a diagnosis answer's wording needs about its session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// The coding tool the session ran in.
    pub host: HostName,
    /// The folder it ran in.
    pub project: Option<String>,
}

/// The project path for a suggested skill named `name`.
pub fn skill_path(project: &str, name: &str) -> PathBuf {
    Path::new(project)
        .join(".claude")
        .join("skills")
        .join(name)
        .join("SKILL.md")
}

/// The DTO for a run's outcome.
pub fn answer_dto(
    outcome: Result<agents::Diagnosis, DiagnoseError>,
    sent: &input::Sent,
    messages: Vec<input::SentMessage>,
    zone: &jiff::tz::TimeZone,
    session: &Session,
) -> SessionDiagnosis {
    let tried_dto = |tried: &[Tried], instead: Option<Tool>| {
        tried
            .iter()
            .map(|tried| dto::tried(tried, instead))
            .collect::<Vec<_>>()
    };
    let sent_to = |tried: &[Tried], last: Option<Tool>| {
        let mut tools: Vec<Tool> = tried
            .iter()
            .filter(|tried| tried.given_prompt)
            .map(|tried| tried.tool)
            .collect();
        tools.extend(last);
        sent_dto(sent, zone, &tools)
    };
    match outcome {
        Ok(found) => {
            let skill_name = found.answer.skill_name();
            let skill_path = skill_name.as_deref().and_then(|name| {
                session
                    .project
                    .as_deref()
                    .filter(|cwd| !cwd.trim().is_empty())
                    .map(|cwd| skill_path(cwd, name).to_string_lossy().into_owned())
            });
            let skill_note = skill_name
                .as_ref()
                .and_then(|_| dto::skill_note(session.host))
                .map(str::to_owned);
            let instruction_file = (found.answer.suggestion == answer::Suggestion::AddToClaudeMd)
                .then(|| dto::instruction_file(session.host))
                .flatten()
                .map(str::to_owned);
            SessionDiagnosis::Diagnosed {
                tool: found.tool.into(),
                ran: dto::ran_text(
                    found.tool,
                    &found.model.asked,
                    found.model.answered.as_deref(),
                    true,
                ),
                model: found.model.answered,
                asked_model: found.model.asked,
                tries: found.tries as u32,
                what_happened: found.answer.what_happened.clone(),
                real_problem: found.answer.real_problem.into(),
                real_problem_text: dto::real_problem_text(found.answer.real_problem).to_owned(),
                suggestion: found.answer.suggestion.into(),
                suggestion_text: dto::suggestion_text(found.answer.suggestion).to_owned(),
                draft_kind: dto::draft_kind(found.answer.suggestion),
                skill_name,
                skill_path,
                skill_note,
                instruction_file,
                draft: found.answer.draft,
                why_parts: dto::reference_parts(&found.answer.why, &messages),
                messages: messages.into_iter().map(dto::message_dto).collect(),
                why: found.answer.why,
                sent: Box::new(sent_to(&found.tried, Some(found.tool))),
                limits: found
                    .tool
                    .limits()
                    .iter()
                    .copied()
                    .map(dto::limit)
                    .collect(),
                tried: tried_dto(&found.tried, Some(found.tool)),
            }
        }
        Err(DiagnoseError::NoTool { tried }) => SessionDiagnosis::NoAgentTool {
            sent: tried
                .iter()
                .any(|tried| tried.given_prompt)
                .then(|| sent_to(&tried, None)),
            tried: tried_dto(&tried, None),
            text: dto::NO_TOOL.to_owned(),
        },
        Err(DiagnoseError::Refused {
            tool,
            model,
            error,
            tried,
        }) => SessionDiagnosis::AnswerRefused {
            tool: tool.into(),
            ran: dto::ran_text(tool, &model.asked, model.answered.as_deref(), false),
            model: model.answered,
            asked_model: model.asked,
            reason: error.to_string(),
            sent: sent_to(&tried, Some(tool)),
            tried: tried_dto(&tried, None),
            text: dto::REFUSED.to_owned(),
        },
        Err(DiagnoseError::Run {
            tool,
            model,
            failure,
            tried,
        }) => SessionDiagnosis::RunFailed {
            tool: tool.into(),
            ran: dto::ran_text(tool, &model.asked, model.answered.as_deref(), false),
            model: model.answered,
            asked_model: model.asked,
            reason: failure.to_string(),
            sent: sent_to(&tried, Some(tool)),
            tried: tried_dto(&tried, None),
            text: dto::run_failed_text(tool, failure),
        },
        Err(DiagnoseError::Cancelled) => SessionDiagnosis::Cancelled {
            text: dto::CANCELLED.to_owned(),
        },
        Err(DiagnoseError::Staging | DiagnoseError::UnsupportedPlatform) => {
            SessionDiagnosis::Failed {
                text: dto::PREPARE_FAILED.to_owned(),
            }
        }
    }
}

fn sent_dto(sent: &input::Sent, zone: &jiff::tz::TimeZone, tools: &[Tool]) -> DiagnosisSent {
    let clock = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .ok()
            .map(|at| at.to_zoned(zone.clone()).strftime("%H:%M").to_string())
    };
    let span = match (
        sent.first_at_ms.and_then(clock),
        sent.last_at_ms.and_then(clock),
    ) {
        (Some(from), Some(to)) => Some((from, to)),
        _ => None,
    };
    DiagnosisSent {
        messages: sent.messages as u32,
        flagged: sent.flagged as u32,
        cut: sent.cut as u32,
        first_at_ms: sent.first_at_ms,
        last_at_ms: sent.last_at_ms,
        tools: tools.iter().copied().map(Into::into).collect(),
        text: dto::sent_text(sent.messages, span, tools),
    }
}

/// The index's facts about one record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordFacts {
    /// M-02: whether it is a person's message.
    pub person: Option<bool>,
    pub agent_id: Option<String>,
}

/// The flagged record's conversation as messages, and the flagged one's
/// position. `None` when its record or explicit tool call/result block cannot be
/// selected exactly, or a flagged subagent record has no saved subagent ID.
pub fn conversation(
    records: &[ParsedRecord],
    facts: &HashMap<String, RecordFacts>,
    flag: &WarningPoint,
) -> Option<(Vec<Message>, usize)> {
    let mut anchors = records
        .iter()
        .filter(|record| record.canonical.uuid.as_deref() == Some(flag.record_uuid.as_str()));
    let anchor = anchors.next()?;
    if anchors.next().is_some() {
        return None;
    }
    let agent_of = |record: &ParsedRecord| {
        record
            .canonical
            .uuid
            .as_deref()
            .and_then(|uuid| facts.get(uuid))
            .and_then(|facts| facts.agent_id.clone())
    };
    let sidechain = anchor.canonical.is_sidechain;
    let agent = if sidechain {
        Some(agent_of(anchor).filter(|id| !id.trim().is_empty())?)
    } else {
        None
    };
    let mut messages: Vec<Message> = Vec::new();
    let mut flagged: Option<usize> = None;
    let mut calls: HashMap<String, String> = HashMap::new();
    let mut last_agent_message: Option<String> = None;
    for record in records.iter().filter(|record| {
        record.canonical.is_sidechain == sidechain && (!sidechain || agent_of(record) == agent)
    }) {
        let Ok(source) = SourceRecord::try_from(record) else {
            continue;
        };
        let is_flagged = source.id == flag.record_uuid && flagged.is_none();
        if is_flagged && let Some(index) = flag.block {
            let mut blocks = source.blocks.iter().filter(|block| block.index() == index);
            if !matches!(
                (source.role, blocks.next()),
                (
                    crate::transcript_dto::RecordRole::User,
                    Some(SourceBlock::ToolResult { .. })
                ) | (
                    crate::transcript_dto::RecordRole::Assistant,
                    Some(SourceBlock::ToolCall { .. })
                )
            ) || blocks.next().is_some()
            {
                return None;
            }
        }
        let at_ms = source
            .at
            .as_deref()
            .and_then(|at| xt_store::timestamp::parse(at).ok())
            .map(|(_, ms)| ms);
        let start = messages.len();
        let mut flagged_block: Option<usize> = None;
        match source.role {
            crate::transcript_dto::RecordRole::Assistant => {
                if is_flagged && let Some(index) = flag.block {
                    let mut parts = Vec::new();
                    let flush = |parts: &mut Vec<String>, messages: &mut Vec<Message>| {
                        if !parts.is_empty() {
                            messages.push(Message {
                                sender: Sender::Agent,
                                text: std::mem::take(parts).join("\n"),
                                at_ms,
                            });
                        }
                    };
                    for block in &source.blocks {
                        match block {
                            SourceBlock::Text { text, .. } => parts.push(text.clone()),
                            SourceBlock::ToolCall {
                                index: block_index,
                                name,
                                call_id,
                                input,
                            } => {
                                if let Some(id) = call_id {
                                    calls.insert(id.clone(), name.clone());
                                }
                                let line = call_line(name, input.as_ref());
                                if *block_index == index {
                                    flush(&mut parts, &mut messages);
                                    flagged = Some(messages.len());
                                    messages.push(Message {
                                        sender: Sender::Agent,
                                        text: line,
                                        at_ms,
                                    });
                                } else {
                                    parts.push(line);
                                }
                            }
                            _ => {}
                        }
                    }
                    flush(&mut parts, &mut messages);
                    last_agent_message = None;
                    continue;
                }
                let mut parts = Vec::new();
                for block in &source.blocks {
                    match block {
                        SourceBlock::Text { text, .. } => parts.push(text.clone()),
                        SourceBlock::ToolCall {
                            name,
                            call_id,
                            input,
                            ..
                        } => {
                            if let Some(id) = call_id {
                                calls.insert(id.clone(), name.clone());
                            }
                            parts.push(call_line(name, input.as_ref()));
                        }
                        SourceBlock::Thinking { .. }
                        | SourceBlock::ToolResult { .. }
                        | SourceBlock::Unsupported { .. } => {}
                    }
                }
                let message_id = record.canonical.message.id.clone();
                let text = parts.join("\n");
                let same_reply = message_id.is_some() && message_id == last_agent_message;
                match messages.last_mut() {
                    Some(previous)
                        if same_reply && previous.sender == Sender::Agent && !text.is_empty() =>
                    {
                        previous.text.push('\n');
                        previous.text.push_str(&text);
                    }
                    _ if !text.is_empty() || is_flagged => messages.push(Message {
                        sender: Sender::Agent,
                        text,
                        at_ms,
                    }),
                    _ => {}
                }
                last_agent_message = message_id;
                if is_flagged {
                    flagged = Some(messages.len() - 1);
                }
                continue;
            }
            crate::transcript_dto::RecordRole::User => {
                last_agent_message = None;
                let person = facts
                    .get(&source.id)
                    .and_then(|facts| facts.person)
                    .unwrap_or(false);
                let mut text = String::new();
                let flush = |text: &mut String, messages: &mut Vec<Message>| {
                    if text.trim().is_empty() {
                        text.clear();
                        return;
                    }
                    let sender = if person && !xt_metrics::tool_written(text) {
                        Sender::Person
                    } else {
                        Sender::Automatic
                    };
                    messages.push(Message {
                        sender,
                        text: std::mem::take(text),
                        at_ms,
                    });
                };
                for block in &source.blocks {
                    match block {
                        SourceBlock::Text { text: words, .. } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(words);
                        }
                        SourceBlock::ToolResult {
                            index,
                            call_id,
                            failed,
                            parts,
                        } => {
                            flush(&mut text, &mut messages);
                            let tool = call_id.as_ref().and_then(|id| calls.get(id)).cloned();
                            let mut body: Vec<String> = parts
                                .iter()
                                .map(|part| match part {
                                    SourceResultPart::Text { text } => text.clone(),
                                    SourceResultPart::Unshown { .. } => "(not shown)".to_owned(),
                                })
                                .collect();
                            if *failed {
                                body.insert(0, "(the tool reported a failure)".to_owned());
                            }
                            if is_flagged && flag.block == Some(*index) {
                                flagged_block = Some(messages.len());
                            }
                            messages.push(Message {
                                sender: Sender::ToolResult { tool },
                                text: body.join("\n"),
                                at_ms,
                            });
                        }
                        SourceBlock::Unsupported { .. } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str("(not shown)");
                        }
                        SourceBlock::Thinking { .. } | SourceBlock::ToolCall { .. } => {}
                    }
                }
                flush(&mut text, &mut messages);
                if is_flagged && messages.len() == start {
                    messages.push(Message {
                        sender: if person {
                            Sender::Person
                        } else {
                            Sender::Automatic
                        },
                        text: String::new(),
                        at_ms,
                    });
                }
            }
        }
        if is_flagged {
            flagged = Some(match flag.block {
                Some(_) => flagged_block?,
                None => start,
            });
        }
    }
    flagged.map(|index| (messages, index))
}

/// `(ran <tool>: <short input>)`.
fn call_line(name: &str, input: Option<&SourcePayload>) -> String {
    let name: String = name.chars().take(input::MAX_TOOL_LABEL_CHARS).collect();
    match input.map(short_input).filter(|text| !text.is_empty()) {
        Some(short) => format!("(ran {name}: {short})"),
        None => format!("(ran {name})"),
    }
}

/// A call's input in one short line: the command, file, path, pattern or
/// address it names when it names one, else its arguments as compact JSON;
/// at most [`MAX_CALL_INPUT_CHARS`] characters.
fn short_input(payload: &SourcePayload) -> String {
    use serde_json::Value;
    let text = match payload {
        SourcePayload::Text { text } => text.clone(),
        SourcePayload::Json { value } => {
            let named = value.as_object().and_then(|object| {
                [
                    "command",
                    "cmd",
                    "file_path",
                    "path",
                    "pattern",
                    "url",
                    "query",
                    "description",
                ]
                .iter()
                .find_map(|key| match object.get(*key) {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Array(parts)) if parts.iter().all(Value::is_string) => Some(
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    _ => None,
                })
            });
            named.unwrap_or_else(|| value.to_string())
        }
    };
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_CALL_INPUT_CHARS {
        line
    } else {
        let cut: String = line.chars().take(MAX_CALL_INPUT_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Save a suggested skill into the session's project, as the module says.
pub fn save_suggested_skill(
    state: &AppState,
    session_id: &str,
    name: &str,
    draft: &str,
) -> Result<SavedSkill, StateError> {
    let refused = |reason| SavedSkill::Refused {
        reason,
        text: dto::refusal_text(reason).to_owned(),
    };
    if let Err(reason) = check_skill(name, draft) {
        return Ok(refused(reason));
    }
    let Some(session) = state.with_store(|store| Ok(store.session(session_id)?))? else {
        return Ok(refused(SaveSkillRefusal::SessionNotFound));
    };
    let Some(cwd) = session.meta.cwd.filter(|cwd| !cwd.trim().is_empty()) else {
        return Ok(refused(SaveSkillRefusal::NoProjectFolder));
    };
    let home = state
        .native_home()
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
    Ok(
        match write_skill(Path::new(&cwd), home.as_deref(), name, draft) {
            Ok(path) => SavedSkill::Saved {
                path: path.to_string_lossy().into_owned(),
                text: dto::saved_text(name),
            },
            Err(reason) => refused(reason),
        },
    )
}

/// The name and draft checks of a save: a skill name, a draft the
/// diagnosis reader accepts as a skill, whose own `name:` is `name`, with no
/// personal detail or secret.
pub fn check_skill(name: &str, draft: &str) -> Result<String, SaveSkillRefusal> {
    if !xt_judge::rubric::is_slug(name) {
        return Err(SaveSkillRefusal::InvalidName);
    }
    let draft = answer::normalise_draft(draft);
    answer::check_skill_draft(&draft).map_err(|_| SaveSkillRefusal::InvalidDraft)?;
    if answer::skill_draft_name(&draft).ok().as_deref() != Some(name) {
        return Err(SaveSkillRefusal::NameMismatch);
    }
    if answer::has_secret(&draft) {
        return Err(SaveSkillRefusal::Secret);
    }
    let draft = answer::quote_description(&draft).map_err(|_| SaveSkillRefusal::InvalidDraft)?;
    answer::check_skill_draft(&draft).map_err(|_| SaveSkillRefusal::InvalidDraft)?;
    Ok(draft)
}

/// Write `<project>/.claude/skills/<name>/SKILL.md`, the checks first.
pub fn write_skill(
    project: &Path,
    home: Option<&Path>,
    name: &str,
    draft: &str,
) -> Result<PathBuf, SaveSkillRefusal> {
    write_skill_inner(
        project,
        home,
        name,
        draft,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
type SaveTestWrite = fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>;

#[cfg(test)]
#[derive(Default)]
struct SaveTestHooks {
    before_open: Option<fn(&Path)>,
    write: Option<SaveTestWrite>,
}

fn write_skill_inner(
    project: &Path,
    home: Option<&Path>,
    name: &str,
    draft: &str,
    #[cfg(test)] hooks: Option<&SaveTestHooks>,
) -> Result<PathBuf, SaveSkillRefusal> {
    let draft = check_skill(name, draft)?;
    if !project.is_absolute() {
        return Err(SaveSkillRefusal::ProjectUnavailable);
    }
    // The project must be a real folder, named as it resolves: no link, no
    // `..`.
    let resolved =
        std::fs::canonicalize(project).map_err(|_| SaveSkillRefusal::ProjectUnavailable)?;
    if resolved != project || !resolved.is_dir() {
        return Err(SaveSkillRefusal::ProjectUnavailable);
    }
    if let Some(home) = home.and_then(|home| std::fs::canonicalize(home).ok())
        && home.starts_with(&resolved)
    {
        return Err(SaveSkillRefusal::ProjectUnavailable);
    }
    // Folders this save created, removed again (deepest first) if it fails.
    let mut created: Vec<PathBuf> = Vec::new();
    let undo = |created: &[PathBuf]| {
        for folder in created.iter().rev() {
            let _ = std::fs::remove_dir(folder);
        }
    };
    let mut folder = resolved.clone();
    for part in [".claude", "skills"] {
        folder.push(part);
        match std::fs::symlink_metadata(&folder) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                undo(&created);
                return Err(SaveSkillRefusal::UnsafeFolder);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::create_dir(&folder).is_err() {
                    undo(&created);
                    return Err(SaveSkillRefusal::WriteFailed);
                }
                created.push(folder.clone());
            }
            Err(_) => {
                undo(&created);
                return Err(SaveSkillRefusal::WriteFailed);
            }
        }
    }
    // Checked again right before the skill's own folder is made, to narrow
    // the time in which `.claude` or `skills` could be swapped for a link.
    if !real_folders(&resolved, &folder) {
        undo(&created);
        return Err(SaveSkillRefusal::UnsafeFolder);
    }
    let skill = folder.join(name);
    match std::fs::create_dir(&skill) {
        Ok(()) => created.push(skill.clone()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            undo(&created);
            return Err(SaveSkillRefusal::AlreadyExists);
        }
        Err(_) => {
            undo(&created);
            return Err(SaveSkillRefusal::WriteFailed);
        }
    }
    // The new folder must be where it was meant to be: no folder above it
    // turned into a link meanwhile.
    if std::fs::canonicalize(&skill).ok().as_deref() != Some(skill.as_path()) {
        undo(&created);
        return Err(SaveSkillRefusal::UnsafeFolder);
    }
    let path = skill.join("SKILL.md");
    #[cfg(test)]
    if let Some(before_open) = hooks.and_then(|hooks| hooks.before_open) {
        before_open(&path);
    }
    // `create_new` never follows or replaces a link at the file's own name.
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(_) => {
            undo(&created);
            return Err(SaveSkillRefusal::WriteFailed);
        }
    };
    let written = (|| {
        use std::io::Write;
        #[cfg(test)]
        if let Some(write) = hooks.and_then(|hooks| hooks.write) {
            return write(&mut file, draft.as_bytes());
        }
        file.write_all(draft.as_bytes())?;
        file.sync_all()
    })();
    drop(file);
    if written.is_err() {
        let _ = std::fs::remove_file(&path);
        undo(&created);
        return Err(SaveSkillRefusal::WriteFailed);
    }
    Ok(path)
}

/// Whether `<project>/.claude` and `<project>/.claude/skills` are both real
/// folders (not links) and `folder` resolves to itself.
fn real_folders(project: &Path, folder: &Path) -> bool {
    let claude = project.join(".claude");
    [claude.as_path(), folder]
        .iter()
        .all(|path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir()))
        && std::fs::canonicalize(folder).ok().as_deref() == Some(folder)
}

#[cfg(test)]
#[path = "session_diagnosis_tests.rs"]
mod tests;
