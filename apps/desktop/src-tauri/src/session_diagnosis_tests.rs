//! Pure diagnosis outcome translation with synthetic messages and answers.
use super::*;
use serde_json::{Value, json};
use xt_judge::diagnosis::{
    agents::{Diagnosis, Model, RunFailure, Skip},
    input::Sender,
};

fn sent_input() -> input::Built {
    let messages: Vec<input::Message> = (1..=20)
        .map(|number| input::Message {
            sender: if number == 13 {
                Sender::Person
            } else {
                Sender::Agent
            },
            text: match number {
                1 => "Synthetic long context. ".repeat(100),
                11 => "The tests failed to connect to the database.".into(),
                12 => "Try the package command again.".into(),
                13 => "Check the database setup and use pnpm for this project.".into(),
                14 => "The work continued with the corrected command.".into(),
                _ => "Synthetic conversation context.".into(),
            },
            at_ms: Some(1_790_000_000_000 + if number == 20 { 29 } else { number - 1 } * 60_000),
        })
        .collect();
    input::build(
        &input::Header {
            kind: input::WarningKind::Correction,
            reason: input::reason(input::WarningKind::Correction, 1),
            host: HostName::ClaudeCode,
            project: "shop".into(),
        },
        &messages,
        12,
    )
    .unwrap()
}

fn sent_messages() -> input::Sent {
    sent_input().sent
}

fn sent_snapshots() -> Vec<input::SentMessage> {
    sent_input().messages
}

#[test]
fn the_answer_dto_says_which_tool_ran_and_which_messages_were_sent() {
    let parsed = answer::parse(
        r#"{"what_happened": "x", "real_problem": "yes", "suggestion": "make_skill",
            "draft": "---\nname: run-e2e\ndescription: Use when running e2e.\n---\n\n# Run e2e\n\n1. Start it.",
            "why": "[3] shows it."}"#,
    )
    .unwrap();
    let tried = vec![Tried {
        tool: Tool::ClaudeCode,
        skip: Skip::NotSignedIn,
        given_prompt: false,
    }];
    let utc = jiff::tz::TimeZone::UTC;
    let codex_session = Session {
        host: HostName::Codex,
        project: Some("/Users/ana/shop".into()),
    };
    let found = answer_dto(
        Ok(Diagnosis {
            tool: Tool::Codex,
            model: Model {
                asked: "small-fast".into(),
                answered: None,
            },
            answer: parsed,
            tries: 1,
            tried: tried.clone(),
        }),
        &sent_messages(),
        sent_snapshots(),
        &utc,
        &codex_session,
    );
    let serialized = serde_json::to_value(&found).unwrap();
    let snapshots = sent_snapshots();
    assert_eq!(
        serialized["messages"].as_array().unwrap().len(),
        snapshots.len()
    );
    for (actual, expected) in serialized["messages"]
        .as_array()
        .unwrap()
        .iter()
        .zip(snapshots)
    {
        assert_eq!(actual["number"], expected.number);
        assert_eq!(actual["text"], expected.text);
        assert_eq!(actual["cut"], expected.cut);
        assert_eq!(actual["flagged"], expected.flagged);
        assert_eq!(actual["at_ms"], expected.at_ms.unwrap());
    }
    assert_eq!(
        serialized["why_parts"][0],
        json!({"kind": "message", "text": "[3]", "number": 3})
    );
    let SessionDiagnosis::Diagnosed {
        tool,
        model,
        asked_model,
        draft_kind,
        skill_name,
        skill_path,
        skill_note,
        instruction_file,
        sent,
        limits,
        tried,
        ..
    } = found
    else {
        panic!("{found:?}")
    };
    assert_eq!(tool, dto::DiagnosisTool::Codex);
    assert_eq!((model, asked_model.as_str()), (None, "small-fast"));
    assert_eq!(draft_kind, dto::DiagnosisDraftKind::Skill);
    assert_eq!(skill_name.as_deref(), Some("run-e2e"));
    assert_eq!(
        skill_path.as_deref(),
        Some("/Users/ana/shop/.claude/skills/run-e2e/SKILL.md")
    );
    assert_eq!(skill_note.as_deref(), Some(dto::SKILL_FOR_CLAUDE_CODE));
    assert_eq!(instruction_file, None);
    // Only the tool that answered was given the messages.
    assert_eq!(sent.tools, [dto::DiagnosisTool::Codex]);
    assert_eq!(
        sent.text,
        "20 messages from this session (14:13 to 14:42) were sent to Codex."
    );
    assert_eq!(
        limits[0].kind,
        dto::DiagnosisLimitKind::CodexLoadsUserInstructions
    );
    assert_eq!(
        tried[0].text,
        "Claude Code is not signed in. Run `claude` and log in. Codex was used instead."
    );
    assert!(!tried[0].given_messages);
    // An instruction line names its file for the session's tool.
    let line = |host| {
        let parsed = answer::parse(
            r#"{"what_happened": "x", "real_problem": "yes", "suggestion": "add_to_claude_md",
                "draft": "Use pnpm, never npm.", "why": "[3] shows it."}"#,
        )
        .unwrap();
        match answer_dto(
            Ok(Diagnosis {
                tool: Tool::ClaudeCode,
                model: Model {
                    asked: "haiku".into(),
                    answered: Some("claude-haiku-test".into()),
                },
                answer: parsed,
                tries: 1,
                tried: Vec::new(),
            }),
            &sent_messages(),
            sent_snapshots(),
            &utc,
            &Session {
                host,
                project: None,
            },
        ) {
            SessionDiagnosis::Diagnosed {
                instruction_file,
                skill_path,
                skill_note,
                model,
                ..
            } => {
                assert_eq!((skill_path, skill_note), (None, None));
                assert_eq!(model.as_deref(), Some("claude-haiku-test"));
                instruction_file
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(line(HostName::ClaudeCode).as_deref(), Some("CLAUDE.md"));
    assert_eq!(line(HostName::Codex).as_deref(), Some("AGENTS.md"));
    assert_eq!(line(HostName::Other), None);
    // Nothing given to any tool: no "sent" at all.
    let none = answer_dto(
        Err(DiagnoseError::NoTool {
            tried: vec![Tried {
                tool: Tool::ClaudeCode,
                skip: Skip::NotInstalled,
                given_prompt: false,
            }],
        }),
        &sent_messages(),
        sent_snapshots(),
        &utc,
        &codex_session,
    );
    assert!(matches!(
        none,
        SessionDiagnosis::NoAgentTool { sent: None, .. }
    ));
    // Both signed out: nothing sent, and each says how to sign in.
    let signed_out = |tool| Tried {
        tool,
        skip: Skip::NotSignedIn,
        given_prompt: false,
    };
    match answer_dto(
        Err(DiagnoseError::NoTool {
            tried: vec![signed_out(Tool::ClaudeCode), signed_out(Tool::Codex)],
        }),
        &sent_messages(),
        sent_snapshots(),
        &utc,
        &codex_session,
    ) {
        SessionDiagnosis::NoAgentTool { tried, sent, text } => {
            assert_eq!(sent, None);
            assert_eq!(text, dto::NO_TOOL);
            assert_eq!(
                tried.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
                [
                    "Claude Code is not signed in. Run `claude` and log in.",
                    "Codex is not signed in. Run `codex login`."
                ]
            );
            let json = serde_json::to_value(&tried).unwrap();
            assert_eq!(json[1]["skip"]["reason"], "not_signed_in");
        }
        other => panic!("{other:?}"),
    }
    let failed = |failure| {
        answer_dto(
            Err(DiagnoseError::Run {
                tool: Tool::ClaudeCode,
                model: Model {
                    asked: "haiku".into(),
                    answered: None,
                },
                failure,
                tried: Vec::new(),
            }),
            &sent_messages(),
            sent_snapshots(),
            &utc,
            &codex_session,
        )
    };
    let json = serde_json::to_value(failed(RunFailure::Timeout)).unwrap();
    assert_eq!(json["state"], "run_failed");
    assert_eq!(json["sent"]["tools"], json!(["claude_code"]));
    assert_eq!(json["model"], Value::Null);
    assert_eq!(json["asked_model"], "haiku");
    assert_eq!(json["text"], dto::RUN_FAILED);
    // Not signed in after the messages were given: said plainly, and that
    // no other tool got them.
    let json = serde_json::to_value(failed(RunFailure::NotSignedIn)).unwrap();
    assert_eq!(
        json["text"],
        "Claude Code is not signed in. Run `claude` and log in. The messages were not given to \
         another tool."
    );
}
