//! Translate a diagnosis outcome into the app's result and message details.

use crate::session_diagnosis_dto::{self as dto, DiagnosisSent, SessionDiagnosis};
use std::path::{Path, PathBuf};
use xt_judge::diagnosis::{
    agents::{self, DiagnoseError, Tool, Tried},
    answer,
    input::{self, HostName},
};

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

#[cfg(test)]
#[path = "session_diagnosis_tests.rs"]
mod tests;
