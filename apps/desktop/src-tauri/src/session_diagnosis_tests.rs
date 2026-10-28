//! Pure diagnosis conversation and outcome translation with synthetic data.
use super::*;
use serde_json::{Value, json};
use xt_ingest::canonical::{Parsed, parse_line};
use xt_judge::diagnosis::{
    agents::{Diagnosis, Model, RunFailure, Skip},
    input::Sender,
};
use xt_metrics::{WarningKind, WarningSender};

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

fn record(line: Value) -> ParsedRecord {
    match parse_line(&line.to_string()).unwrap() {
        Parsed::Record(record) => *record,
        other => panic!("{other:?}"),
    }
}

fn uuid(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn at(n: u32) -> String {
    format!("2026-10-01T10:{:02}:00.000Z", n % 60)
}

fn person(n: u32, text: &str) -> ParsedRecord {
    record(json!({"uuid": uuid(n), "type": "user", "timestamp": at(n),
        "message": {"role": "user", "content": [{"type": "text", "text": text}]}}))
}

fn agent(n: u32, message_id: &str, content: Value) -> ParsedRecord {
    record(
        json!({"uuid": uuid(n), "type": "assistant", "timestamp": at(n),
        "message": {"id": message_id, "role": "assistant", "content": content}}),
    )
}

fn result(n: u32, call: &str, text: &str, failed: bool) -> ParsedRecord {
    record(json!({"uuid": uuid(n), "type": "user", "timestamp": at(n),
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": call, "is_error": failed, "content": text}]}}))
}

fn facts(people: &[u32]) -> HashMap<String, RecordFacts> {
    people
        .iter()
        .map(|n| {
            (
                uuid(*n),
                RecordFacts {
                    person: Some(true),
                    agent_id: None,
                },
            )
        })
        .collect()
}

fn flag(n: u32) -> WarningPoint {
    WarningPoint {
        record_uuid: uuid(n),
        block: None,
    }
}

#[test]
fn records_become_labelled_messages_with_calls_named_and_replies_joined() {
    let records = vec![
        person(1, "run the tests"),
        agent(
            2,
            "msg_a",
            json!([{"type": "thinking", "thinking": "secret plan"},
            {"type": "text", "text": "I'll run them."}]),
        ),
        agent(
            3,
            "msg_a",
            json!([{"type": "tool_use", "id": "toolu_1", "name": "Bash",
            "input": {"command": "pytest   tests/integration", "description": "run"}}]),
        ),
        result(4, "toolu_1", "ERROR: could not connect", true),
        person(5, "<task-notification>done</task-notification>"),
        record(json!({"uuid": uuid(6), "type": "user", "timestamp": at(6),
            "message": {"role": "user", "content": [{"type": "text", "text": "[Request interrupted by user]"}]}})),
        person(7, "still not working"),
    ];
    let (messages, flagged) = conversation(
        &records,
        &facts(&[1, 5, 7]),
        &flag(7),
        WarningSender::Person,
    )
    .unwrap();
    let shown: Vec<(Sender, &str)> = messages
        .iter()
        .map(|m| (m.sender.clone(), m.text.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (Sender::Person, "run the tests"),
            (
                Sender::Agent,
                "I'll run them.\n(ran Bash: pytest tests/integration)"
            ),
            (
                Sender::ToolResult {
                    tool: Some("Bash".into())
                },
                "(the tool reported a failure)\nERROR: could not connect"
            ),
            // Tool-written, though the index calls it a person's.
            (
                Sender::Automatic,
                "<task-notification>done</task-notification>"
            ),
            // Not a person's by the index's classification.
            (Sender::Automatic, "[Request interrupted by user]"),
            (Sender::Person, "still not working"),
        ]
    );
    assert_eq!(flagged, 5);
    assert_eq!(
        messages[0].at_ms,
        Some(xt_store::timestamp::parse(&at(1)).unwrap().1)
    );
    // Thinking is never sent.
    assert!(!messages.iter().any(|m| m.text.contains("secret plan")));
    // A record the source does not hold flags nothing.
    assert!(conversation(&records, &facts(&[]), &flag(99), WarningSender::Person).is_none());
}

#[test]
fn a_flagged_tool_result_block_is_the_flagged_message() {
    let records = vec![
        agent(
            1,
            "m1",
            json!([
            {"type": "tool_use", "id": "a", "name": "Read", "input": {"file_path": "/p/x.rs"}},
            {"type": "tool_use", "id": "b", "name": "Edit", "input": {"file_path": "/p/y.rs"}}]),
        ),
        record(json!({"uuid": uuid(2), "type": "user", "timestamp": at(2),
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a", "content": "fine"},
                {"type": "tool_result", "tool_use_id": "b", "is_error": true,
                 "content": [{"type": "text", "text": "The user doesn't want to proceed"}]}]}})),
    ];
    let flag = WarningPoint {
        record_uuid: uuid(2),
        block: Some(1),
    };
    let (messages, flagged) =
        conversation(&records, &HashMap::new(), &flag, WarningSender::Person).unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].text, "(ran Read: /p/x.rs)\n(ran Edit: /p/y.rs)");
    assert_eq!(flagged, 2);
    assert_eq!(
        messages[2].sender,
        Sender::ToolResult {
            tool: Some("Edit".into())
        }
    );
    assert!(
        conversation(
            &records,
            &HashMap::new(),
            &WarningPoint {
                block: Some(2),
                ..flag.clone()
            },
            WarningSender::Person
        )
        .is_none()
    );
    let (_, whole_record) = conversation(
        &records,
        &HashMap::new(),
        &WarningPoint {
            block: None,
            ..flag
        },
        WarningSender::Person,
    )
    .unwrap();
    assert_eq!(whole_record, 1);
}

#[test]
fn an_explicit_block_refuses_text_and_tools_in_the_wrong_role() {
    for role in ["user", "assistant"] {
        for block in [
            json!({"type": "text", "text": "replacement text"}),
            json!({"type": "tool_use", "id": "replacement", "name": "Read", "input": {}}),
        ] {
            if role == "assistant" && block["type"] == "tool_use" {
                continue;
            }
            let records = vec![record(json!({
                "uuid": uuid(2), "type": role, "timestamp": at(2),
                "message": {"role": role, "content": [
                    {"type": "tool_result", "tool_use_id": "first", "content": "first result"},
                    block
                ]}
            }))];
            assert!(
                conversation(
                    &records,
                    &HashMap::new(),
                    &WarningPoint {
                        record_uuid: uuid(2),
                        block: Some(1),
                    },
                    WarningSender::Person
                )
                .is_none(),
                "{role} {records:?}"
            );
            assert_eq!(
                conversation(&records, &HashMap::new(), &flag(2), WarningSender::Person).is_some(),
                role == "user",
                "whole-record warnings require the user role"
            );
        }
    }
}

#[test]
fn an_admitted_mistake_flags_the_agents_reply_after_the_persons_message() {
    let records = vec![
        person(1, "the parser drops the last line"),
        agent(
            2,
            "m1",
            json!([{"type": "text", "text": "You're right, I misread the loop."}]),
        ),
        person(3, "thanks, try again"),
    ];
    let people = facts(&[1, 3]);
    let sender = WarningKind::AdmittedMistake.points_at_sender();
    assert_eq!(sender, WarningSender::Agent);
    let (messages, flagged) = conversation(&records, &people, &flag(2), sender).unwrap();
    assert_eq!(flagged, 1);
    assert_eq!(
        messages
            .iter()
            .map(|m| (m.sender.clone(), m.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (Sender::Person, "the parser drops the last line"),
            (Sender::Agent, "You're right, I misread the loop."),
            (Sender::Person, "thanks, try again"),
        ]
    );
    let built = input::build(
        &input::Header {
            kind: input::WarningKind::AdmittedMistake,
            reason: input::reason(input::WarningKind::AdmittedMistake, 2),
            host: input::HostName::ClaudeCode,
            project: "synthetic".into(),
        },
        &messages,
        flagged,
    )
    .unwrap();
    assert!(built.text.contains("flagged_message: 2\n"));
    assert!(built.text.contains(
        "[1] person:\n  the parser drops the last line\n\
         [2] agent (flagged):\n  You're right, I misread the loop.\n\
         [3] person:\n  thanks, try again\n"
    ));
    assert_eq!(built.sent.flagged, 2);
    // Every other kind names a person's record, so the same assistant record
    // is still refused for them, and an admitted mistake never flags the
    // person's message.
    for kind in WarningKind::ALL {
        if kind != WarningKind::AdmittedMistake {
            assert!(
                conversation(&records, &people, &flag(2), kind.points_at_sender()).is_none(),
                "{kind:?}"
            );
        }
    }
    assert!(conversation(&records, &people, &flag(1), sender).is_none());
}

#[test]
fn duplicate_flagged_record_ids_are_refused() {
    let records = vec![
        result(2, "a", "first copy", true),
        result(2, "a", "second copy", true),
    ];
    for block in [None, Some(0)] {
        assert!(
            conversation(
                &records,
                &HashMap::new(),
                &WarningPoint {
                    record_uuid: uuid(2),
                    block,
                },
                WarningSender::Person
            )
            .is_none()
        );
    }
}

#[cfg(unix)]
#[test]
fn only_the_flagged_records_own_conversation_is_used() {
    let side = |n: u32, agent_id: &str, text: &str| {
        record(
            json!({"uuid": uuid(n), "type": "user", "timestamp": at(n), "isSidechain": true,
            "agentId": agent_id,
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}}),
        )
    };
    let records = vec![
        person(1, "main one"),
        side(2, "agent-a", "sub a"),
        side(3, "agent-b", "sub b"),
        person(4, "main two"),
    ];
    let mut facts = facts(&[1, 4]);
    for (n, id) in [(2, "agent-a"), (3, "agent-b")] {
        facts.insert(
            uuid(n),
            RecordFacts {
                person: Some(false),
                agent_id: Some(id.into()),
            },
        );
    }
    let (main, flagged) = conversation(&records, &facts, &flag(4), WarningSender::Person).unwrap();
    assert_eq!(
        main.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["main one", "main two"]
    );
    assert_eq!(flagged, 1);
    let (sub, _) = conversation(&records, &facts, &flag(3), WarningSender::Person).unwrap();
    assert_eq!(
        sub.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["sub b"]
    );
}

#[test]
fn the_flagged_record_is_the_one_the_warning_points_at() {
    let warning = |kind, anchor: &str, records: &[u32], repeat: Option<WarningPoint>| {
        xt_metrics::SessionWarning {
            kind,
            session_id: "s".into(),
            anchor: anchor.into(),
            at_ms: 0,
            record_uuids: records.iter().map(|n| uuid(*n)).collect(),
            reason: "you wrote \"no\" after a reply (10:00)".into(),
            compact_reason: Some("After an agent reply".into()),
            correction_phrase: None,
            repeat,
            size: None,
        }
    };
    assert_eq!(
        warning(WarningKind::Correction, &uuid(3), &[3], None).points_at(),
        flag(3)
    );
    // Two failures in one record: the second failure's own result is the
    // flagged message.
    let both = WarningPoint {
        record_uuid: uuid(2),
        block: Some(1),
    };
    let records = vec![
        agent(
            1,
            "m1",
            json!([
            {"type": "tool_use", "id": "a", "name": "Bash", "input": {"command": "make"}},
            {"type": "tool_use", "id": "b", "name": "Bash", "input": {"command": "make"}}]),
        ),
        record(json!({"uuid": uuid(2), "type": "user", "timestamp": at(2),
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a", "is_error": true, "content": "first"},
                {"type": "tool_result", "tool_use_id": "b", "is_error": true, "content": "second"}]}})),
    ];
    let repeated = warning(
        WarningKind::RepeatedFailure,
        &format!("{}#0", uuid(2)),
        &[2, 2],
        Some(both.clone()),
    );
    assert_eq!(repeated.points_at(), both);
    let (messages, flagged) = conversation(
        &records,
        &HashMap::new(),
        &repeated.points_at(),
        WarningSender::Person,
    )
    .unwrap();
    assert!(messages[flagged].text.ends_with("second"));
    // The header's reason is the fixed sentence, never the stored one that
    // quotes the person.
    let header_reason = input::reason(input::WarningKind::Correction, 1);
    assert!(!header_reason.contains("10:00"));
}

#[test]
fn a_call_input_is_one_short_line() {
    let json = |value: Value| SourcePayload::Json { value };
    assert_eq!(
        short_input(&json(json!({"command": ["bash", "-lc", "cargo  test"]}))),
        "bash -lc cargo test"
    );
    assert_eq!(short_input(&json(json!({"x": 1}))), "{\"x\":1}");
    let long = short_input(&json(json!({"command": "a ".repeat(300)})));
    assert_eq!(long.chars().count(), MAX_CALL_INPUT_CHARS);
    assert!(long.ends_with('…'));
    assert_eq!(call_line("Bash", None), "(ran Bash)");
}

#[test]
fn a_late_flagged_tool_call_survives_long_assistant_prose_and_input_bounds() {
    let mut blocks = vec![json!({"type":"text", "text":"long prose ".repeat(2000)})];
    for n in 0..25 {
        blocks.push(json!({"type":"tool_use", "id":format!("call-{n}"),
        "name":"setup", "input":{}}));
    }
    blocks.push(
        json!({"type":"tool_use", "id":"repeat-call", "name":"write_stdin",
        "input":{"session_id":123,"chars":""}}),
    );
    let records = vec![
        person(1, "start the updater tests"),
        agent(2, "m1", json!(blocks)),
        result(3, "repeat-call", "test process still running", false),
    ];
    let (messages, flagged) = conversation(
        &records,
        &facts(&[1]),
        &WarningPoint {
            record_uuid: uuid(2),
            block: Some(26),
        },
        WarningSender::Person,
    )
    .unwrap();
    let built = input::build(
        &input::Header {
            kind: input::WarningKind::RepeatedFailure,
            reason: "the same tool call failed 2 times".into(),
            host: input::HostName::Codex,
            project: "synthetic".into(),
        },
        &messages,
        flagged,
    )
    .unwrap();
    assert!(built.text.contains("write_stdin"));
    assert!(built.text.contains("test process still running"));
    assert!(messages[flagged].text.starts_with("(ran write_stdin:"));
}

#[test]
fn unknown_subagent_identity_refuses_context_instead_of_mixing_two_subagents() {
    let subagent = |n, id, text| {
        record(json!({"uuid": uuid(n), "type": "user", "timestamp": at(n),
            "isSidechain": true, "agentId": id,
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}}))
    };
    let records = vec![
        person(1, "main conversation"),
        subagent(2, "agent-a", "first subagent's private context"),
        subagent(3, "agent-b", "second subagent's private context"),
    ];
    for missing in [None, Some(None), Some(Some("")), Some(Some(" \t"))] {
        let mut saved = facts(&[1]);
        if let Some(agent_id) = missing {
            for n in [2, 3] {
                saved.insert(
                    uuid(n),
                    RecordFacts {
                        person: None,
                        agent_id: agent_id.map(str::to_owned),
                    },
                );
            }
        }
        for n in [2, 3] {
            assert!(
                conversation(&records, &saved, &flag(n), WarningSender::Person).is_none(),
                "unknown saved subagent ID {missing:?} must stop before choosing messages"
            );
        }
        let (main, _) = conversation(&records, &saved, &flag(1), WarningSender::Person).unwrap();
        assert_eq!(main.len(), 1);
        assert_eq!(main[0].text, "main conversation");
        assert_eq!(main[0].sender, Sender::Person);
    }
}

const SKILL: &str = "---\nname: shop-api-tests\ndescription: Use when running the shop-api \
                     tests.\n---\n\n# Run shop-api tests\n\n1. Start the database.\n2. Run the \
                     tests.\n";

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap().join("shop-api");
    std::fs::create_dir(&path).unwrap();
    (dir, path)
}

/// `SKILL` as saved: its description written as a quoted string.
fn saved(skill: &str) -> String {
    answer::quote_description(skill).unwrap()
}

#[test]
fn a_suggested_skill_is_saved_in_the_project_once() {
    let (_dir, project) = project();
    let path = write_skill(&project, None, "shop-api-tests", SKILL).unwrap();
    assert_eq!(path, project.join(".claude/skills/shop-api-tests/SKILL.md"));
    assert_eq!(
        path,
        skill_path(project.to_str().unwrap(), "shop-api-tests"),
        "the path the answer names"
    );
    let written = saved(SKILL);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [std::ffi::OsString::from("SKILL.md")]);
    // Never over an existing one, and the existing one is untouched.
    let other = SKILL.replace("Start the database", "Something else");
    assert_eq!(
        write_skill(&project, None, "shop-api-tests", &other),
        Err(SaveSkillRefusal::AlreadyExists)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
}

#[test]
fn a_saved_skill_header_reads_as_yaml_whatever_its_description_holds() {
    let (_dir, project) = project();
    let draft = SKILL.replace(
        "description: Use when running the shop-api tests.",
        "description: Use when: the shop-api tests fail #2 [ci]",
    );
    let path = write_skill(&project, None, "shop-api-tests", &draft).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let line = written.lines().nth(2).unwrap();
    let value = line.strip_prefix("description: ").unwrap();
    // A JSON string is a YAML double-quoted scalar.
    assert!(value.starts_with('"'));
    let text: String = serde_json::from_str(value).unwrap();
    assert_eq!(text, "Use when: the shop-api tests fail #2 [ci]");
    assert!(written.starts_with("---\nname: shop-api-tests\n"));
}

#[test]
fn a_failed_save_leaves_no_folder_it_made_and_keeps_the_ones_it_did_not() {
    use std::os::unix::fs::PermissionsExt;
    // The project's own `.claude` exists and cannot be written into: the
    // save fails, `.claude` stays, nothing new is there.
    let (_dir, project) = project();
    let claude = project.join(".claude");
    std::fs::create_dir(&claude).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = write_skill(&project, None, "shop-api-tests", SKILL);
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(outcome, Err(SaveSkillRefusal::WriteFailed));
    assert!(claude.is_dir());
    assert_eq!(std::fs::read_dir(&claude).unwrap().count(), 0);
    // The project folder itself cannot be written into: nothing is made.
    let (_dir2, project) = self::project();
    std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = write_skill(&project, None, "shop-api-tests", SKILL);
    std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(outcome, Err(SaveSkillRefusal::WriteFailed));
    assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
}

#[test]
fn a_suggested_skill_is_refused_unless_name_draft_and_folder_are_safe() {
    let (dir, project) = project();
    let write = |name: &str, draft: &str| write_skill(&project, None, name, draft);
    assert_eq!(write("../x", SKILL), Err(SaveSkillRefusal::InvalidName));
    assert_eq!(write("Shop", SKILL), Err(SaveSkillRefusal::InvalidName));
    assert_eq!(
        write("other-name", SKILL),
        Err(SaveSkillRefusal::NameMismatch)
    );
    assert_eq!(
        write("shop-api-tests", "just text"),
        Err(SaveSkillRefusal::InvalidDraft)
    );
    assert_eq!(
        write(
            "shop-api-tests",
            &SKILL.replace("Start the database.", "Ask ana@example.com.")
        ),
        Err(SaveSkillRefusal::Secret)
    );
    // A relative, missing or linked project folder.
    assert_eq!(
        write_skill(Path::new("shop-api"), None, "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    assert_eq!(
        write_skill(&project.join("gone"), None, "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&project, &link).unwrap();
    assert_eq!(
        write_skill(&link, None, "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    // The home folder, or a folder above it, is never a project here.
    assert_eq!(
        write_skill(&project, Some(&project), "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    assert_eq!(
        write_skill(&project.join(".."), None, "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    let nested_home = project.join("home");
    std::fs::create_dir(&nested_home).unwrap();
    assert_eq!(
        write_skill(&project, Some(&nested_home), "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::ProjectUnavailable)
    );
    // A `.claude` that is a link is not followed.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, project.join(".claude")).unwrap();
    assert_eq!(
        write("shop-api-tests", SKILL),
        Err(SaveSkillRefusal::UnsafeFolder)
    );
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    assert!(!project.join(".claude/skills").exists());
    let (other_dir, other_project) = self::project();
    let other_target = other_dir.path().join("elsewhere");
    std::fs::create_dir(&other_target).unwrap();
    std::fs::create_dir(other_project.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&other_target, other_project.join(".claude/skills")).unwrap();
    assert_eq!(
        write_skill(&other_project, None, "shop-api-tests", SKILL),
        Err(SaveSkillRefusal::UnsafeFolder)
    );
    assert_eq!(std::fs::read_dir(other_target).unwrap().count(), 0);
}

#[test]
fn save_regression_rechecks_the_quoted_draft_at_the_byte_boundary() {
    let (_dir, project) = project();
    let prefix = format!(
        "---\nname: boundary\ndescription: {}\n---\n# Boundary\n\n1. ",
        "\\".repeat(answer::MAX_DESCRIPTION_CHARS)
    );
    let draft = |bytes| format!("{prefix}{}\n", "x".repeat(bytes - prefix.len() - 1));
    let expanding = draft(answer::MAX_SKILL_BYTES);
    assert_eq!(answer::check_skill_draft(&expanding), Ok(()));
    assert!(answer::quote_description(&expanding).unwrap().len() > answer::MAX_SKILL_BYTES);
    assert_eq!(
        write_skill(&project, None, "boundary", &expanding),
        Err(SaveSkillRefusal::InvalidDraft)
    );
    assert!(
        !project.join(".claude").exists(),
        "refused before creating folders"
    );

    let fitting = draft(answer::MAX_SKILL_BYTES - answer::MAX_DESCRIPTION_CHARS - 2);
    let path = write_skill(&project, None, "boundary", &fitting).unwrap();
    let written = std::fs::read_to_string(path).unwrap();
    assert_eq!(written.len(), answer::MAX_SKILL_BYTES);
    assert_eq!(answer::check_skill_draft(&written), Ok(()));
    assert_eq!(written, answer::quote_description(&fitting).unwrap());
}

#[test]
fn save_regression_failed_publish_preserves_a_competing_file() {
    let (_dir, project) = project();
    let hooks = SaveTestHooks {
        before_publish: Some(|path| std::fs::write(path, "competing writer's skill").unwrap()),
        ..Default::default()
    };
    assert_eq!(
        write_skill_inner(&project, None, "shop-api-tests", SKILL, Some(&hooks)),
        Err(SaveSkillRefusal::WriteFailed)
    );
    let path = skill_path(project.to_str().unwrap(), "shop-api-tests");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "competing writer's skill"
    );
    assert_eq!(
        std::fs::read_dir(project.join(".claude/skills/shop-api-tests"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn an_owned_partial_write_removes_only_owned_files_and_new_empty_folders() {
    for setup in [
        "new folders",
        "existing parents",
        "unrelated file in new folder",
    ] {
        let (_dir, project) = project();
        let parent = project.join(".claude/skills");
        if setup == "existing parents" {
            std::fs::create_dir_all(&parent).unwrap();
            std::fs::write(parent.join("existing.txt"), "keep existing").unwrap();
        }
        fn fail_write(_: &Path, file: &mut std::fs::File, _: &[u8]) -> std::io::Result<()> {
            use std::io::Write;
            file.write_all(b"owned partial skill")?;
            Err(std::io::Error::other("injected write failure"))
        }
        let hooks = SaveTestHooks {
            write: Some(if setup == "unrelated file in new folder" {
                |path, file, draft| {
                    std::fs::write(
                        path.parent().unwrap().join("unrelated.txt"),
                        "keep unrelated",
                    )?;
                    fail_write(path, file, draft)
                }
            } else {
                fail_write
            }),
            ..Default::default()
        };
        assert_eq!(
            write_skill_inner(&project, None, "shop-api-tests", SKILL, Some(&hooks)),
            Err(SaveSkillRefusal::WriteFailed),
            "{setup}"
        );
        let skill = parent.join("shop-api-tests");
        assert!(!skill.join("SKILL.md").exists(), "{setup}");
        match setup {
            "new folders" => assert!(!project.join(".claude").exists()),
            "existing parents" => {
                assert!(!skill.exists());
                assert_eq!(
                    std::fs::read_to_string(parent.join("existing.txt")).unwrap(),
                    "keep existing"
                );
            }
            _ => {
                assert_eq!(
                    std::fs::read_to_string(skill.join("unrelated.txt")).unwrap(),
                    "keep unrelated"
                );
                assert_eq!(std::fs::read_dir(skill).unwrap().count(), 1);
            }
        }
    }
}

#[test]
fn saving_uses_the_indexed_sessions_project_and_returns_typed_refusals() {
    let (dir, project) = project();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let state = AppState::build(
        crate::state::StartupOptions {
            data_dir: Some(dir.path().join("data")),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("synthetic data directory is explicit"),
        || panic!("synthetic home is explicit"),
    )
    .unwrap();
    let session = uuid(91);
    let no_project = uuid(92);
    state
        .with_store(|store| {
            let mut metadata =
                xt_store::SessionMeta::new(&session, "codex", xt_store::SessionSource::Transcript);
            metadata.cwd = Some(project.to_string_lossy().into_owned());
            store.upsert_session(&metadata, false)?;
            store.upsert_session(
                &xt_store::SessionMeta::new(
                    &no_project,
                    "claude",
                    xt_store::SessionSource::Transcript,
                ),
                false,
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        !project.join(".claude").exists(),
        "loading metadata starts no save"
    );
    for (session_id, reason) in [
        (&uuid(93), SaveSkillRefusal::SessionNotFound),
        (&no_project, SaveSkillRefusal::NoProjectFolder),
    ] {
        assert_eq!(
            save_suggested_skill(&state, session_id, "shop-api-tests", SKILL).unwrap(),
            SavedSkill::Refused {
                reason,
                text: dto::refusal_text(reason).into()
            }
        );
    }
    let saved = save_suggested_skill(&state, &session, "shop-api-tests", SKILL).unwrap();
    let path = skill_path(project.to_str().unwrap(), "shop-api-tests");
    assert_eq!(
        saved,
        SavedSkill::Saved {
            path: path.to_string_lossy().into_owned(),
            text: "Saved the skill shop-api-tests in this project.".into(),
        }
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), self::saved(SKILL));
    assert!(!home.join(".claude").exists());
    state.shutdown();
}

#[test]
fn a_failed_staging_write_preserves_a_competing_final_file() {
    let (_dir, project) = project();
    let hooks = SaveTestHooks {
        write: Some(|path, file, _| {
            use std::io::Write;
            use std::os::unix::fs::MetadataExt;
            let owned_inode = file.metadata()?.ino();
            assert!(!path.exists(), "the checked bytes are still staged");
            file.write_all(b"owned partial skill")?;
            std::fs::write(path, "replacement skill from another writer")?;
            assert_ne!(std::fs::metadata(path)?.ino(), owned_inode);
            Err(std::io::Error::other("injected staging write failure"))
        }),
        ..Default::default()
    };
    let outcome = write_skill_inner(&project, None, "shop-api-tests", SKILL, Some(&hooks));
    assert_eq!(outcome, Err(SaveSkillRefusal::WriteFailed));
    let path = skill_path(project.to_str().unwrap(), "shop-api-tests");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "replacement skill from another writer"
    );
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
}

#[test]
fn a_competing_final_file_created_during_staging_prevents_publication() {
    let (_dir, project) = project();
    let hooks = SaveTestHooks {
        write: Some(|path, file, draft| {
            use std::io::Write;
            assert!(!path.exists());
            std::fs::write(path, "competing complete skill")?;
            file.write_all(draft)?;
            file.sync_all()
        }),
        ..Default::default()
    };
    assert_eq!(
        write_skill_inner(&project, None, "shop-api-tests", SKILL, Some(&hooks)),
        Err(SaveSkillRefusal::WriteFailed)
    );
    let path = skill_path(project.to_str().unwrap(), "shop-api-tests");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "competing complete skill"
    );
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
}

#[test]
fn changed_source_result_blocks_stop_before_any_agent_is_called() {
    use std::os::unix::fs::PermissionsExt;
    use xt_ingest::native::session_source::GenerationBasis;

    for replacement in [
        None,
        Some(json!({"type": "text", "text": "changed result"})),
    ] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join(".claude/projects/-diagnosis-fixture");
        std::fs::create_dir_all(&project).unwrap();
        let sid = uuid(90);
        let calls = json!({"uuid": uuid(1), "type": "assistant", "sessionId": sid,
        "timestamp": at(1), "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "a", "name": "Bash", "input": {"command": "make"}},
            {"type": "tool_use", "id": "b", "name": "Bash", "input": {"command": "make"}}
        ]}});
        let replies = json!({"uuid": uuid(2), "type": "user", "sessionId": sid,
        "timestamp": at(2), "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "a", "is_error": true, "content": "first failure"},
            {"type": "tool_result", "tool_use_id": "b", "is_error": true, "content": "second failure"}
        ]}});
        let data = root.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let mut store = xt_store::Store::open(data.join("xtrace.db")).unwrap();
        let mut meta =
            xt_store::SessionMeta::new(&sid, "claude", xt_store::SessionSource::Transcript);
        meta.native_session_id = Some(sid.clone());
        meta.surface = Some("cli".into());
        store.upsert_session(&meta, false).unwrap();
        store
            .upsert_records(
                &sid,
                &[
                    record(calls.clone()).canonical,
                    record(replies.clone()).canonical,
                ],
                false,
            )
            .unwrap();
        drop(store);

        let mut changed = replies;
        let blocks = changed["message"]["content"].as_array_mut().unwrap();
        blocks.pop();
        if let Some(replacement) = replacement {
            blocks.push(replacement);
        }
        std::fs::write(
            project.join(format!("{sid}.jsonl")),
            format!("{calls}\n{changed}\n"),
        )
        .unwrap();
        let state = AppState::build(
            crate::state::StartupOptions {
                data_dir: Some(data),
                native_home: Some(home.clone()),
                ..Default::default()
            },
            || panic!("explicit test data"),
            || panic!("explicit test home"),
        )
        .unwrap();
        let warning_id = format!("repeated_failure:{}#0", uuid(2));
        let (warning, _) = state
            .session_warning(&sid, &warning_id, "UTC")
            .unwrap()
            .unwrap();
        assert_eq!(warning.points_at().block, Some(1));
        let readers = crate::native_index::DetailReaders::disabled();
        let loaded = state
            .loaded_source(&sid, &CancelToken::new(), &readers)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.generation, GenerationBasis::Unrecorded);
        drop(loaded);

        let fake = root.path().join("fake-coding-tool");
        std::fs::write(&fake, "#!/bin/sh\n: > \"$HOME/agent-called\"\nexit 9\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment = RunEnvironment {
            inherited_env: vec![
                ("HOME".into(), home.as_os_str().to_owned()),
                ("PATH".into(), root.path().as_os_str().to_owned()),
                (
                    xt_probes::executable::CLAUDE_ENV.into(),
                    fake.as_os_str().to_owned(),
                ),
                (agents::CODEX_ENV.into(), fake.as_os_str().to_owned()),
                (agents::CURSOR_ENV.into(), fake.as_os_str().to_owned()),
            ],
            temp_dir: root.path().join("agent-runs"),
            limits: AgentLimits::default(),
        };
        let request = Request {
            session_id: &sid,
            warning_id: &warning_id,
            timezone: "UTC",
            read_id: "missing-result",
        };
        let runs = Arc::new(DiagnosisRuns::default());
        let answer = diagnose(
            &state,
            reserve(&runs, &request).unwrap().unwrap(),
            &readers,
            &environment,
            &request,
        )
        .unwrap();
        assert!(
            matches!(answer, SessionDiagnosis::FlaggedMessageNotFound { .. }),
            "{answer:?}"
        );
        assert!(!home.join("agent-called").exists());
        assert!(!environment.temp_dir.exists());
    }
}

#[test]
fn only_one_diagnosis_runs_at_a_time_and_cancel_names_it() {
    let runs = Arc::new(DiagnosisRuns::default());
    let slot = runs.begin("one").unwrap();
    assert!(runs.begin("two").is_none());
    assert!(!runs.cancel("two"));
    assert!(runs.cancel("one"));
    assert!(slot.source.is_cancelled() && slot.agent.is_cancelled());
    drop(slot);
    assert!(runs.begin("two").is_some());
    runs.shutdown();
    assert!(
        runs.begin("three").is_none(),
        "nothing starts after quitting"
    );
}

#[test]
fn a_reserved_diagnosis_can_be_stopped_before_its_worker_starts() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("home")).unwrap();
    let state = AppState::build(
        crate::state::StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(root.path().join("home")),
            ..Default::default()
        },
        || panic!("explicit test data"),
        || panic!("explicit test home"),
    )
    .unwrap();
    let runs = Arc::new(DiagnosisRuns::default());
    let request = Request {
        session_id: "no-session",
        warning_id: "correction:no-message",
        timezone: "UTC",
        read_id: "queued",
    };
    let slot = reserve(&runs, &request).unwrap().unwrap();
    assert!(runs.cancel("queued"));
    assert!(reserve(&runs, &request).unwrap().is_none());
    // The nonexistent session would answer WarningNotFound if preparation ran.
    let answer = diagnose(
        &state,
        slot,
        &crate::native_index::DetailReaders::disabled(),
        &RunEnvironment {
            inherited_env: Vec::new(),
            temp_dir: root.path().join("unused"),
            limits: AgentLimits::default(),
        },
        &request,
    )
    .unwrap();
    assert!(matches!(answer, SessionDiagnosis::Cancelled { .. }));
    assert!(!root.path().join("unused").exists());
    let later = reserve(&runs, &request).unwrap().unwrap();
    assert!(!runs.cancel("old"));
    assert!(!later.source.is_cancelled());
    drop(later);
    // A queued closure discarded on dispatch failure drops its slot as well.
    let slot = reserve(&runs, &request).unwrap().unwrap();
    let worker = move || drop(slot);
    drop(worker);
    assert!(reserve(&runs, &request).unwrap().is_some());
}

fn native_diagnosis_scene(lines: &[Value]) -> (tempfile::TempDir, AppState, String, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap().join("home");
    let directory = home.join(".claude/projects/-diagnose-run-fixture");
    std::fs::create_dir_all(&directory).unwrap();
    let sid = uuid(94);
    let mut lines = lines.to_vec();
    for line in &mut lines {
        line["sessionId"] = json!(sid);
    }
    std::fs::write(
        directory.join(format!("{sid}.jsonl")),
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let data = root.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let mut store = xt_store::Store::open(data.join("xtrace.db")).unwrap();
    let mut meta = xt_store::SessionMeta::new(&sid, "claude", xt_store::SessionSource::Transcript);
    meta.native_session_id = Some(sid.clone());
    meta.surface = Some("cli".into());
    store.upsert_session(&meta, false).unwrap();
    store
        .upsert_records(
            &sid,
            &lines
                .into_iter()
                .map(|line| record(line).canonical)
                .collect::<Vec<_>>(),
            false,
        )
        .unwrap();
    drop(store);
    let state = AppState::build(
        crate::state::StartupOptions {
            data_dir: Some(data),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("explicit synthetic data"),
        || panic!("explicit synthetic home"),
    )
    .unwrap();
    (root, state, sid, home)
}

fn repeated_failure_lines() -> Vec<Value> {
    vec![
        json!({"uuid":uuid(1),"type":"assistant","timestamp":at(1),"message":{"role":"assistant","content":[
            {"type":"tool_use","id":"first","name":"Bash","input":{"command":"make"}},
            {"type":"tool_use","id":"second","name":"Bash","input":{"command":"make"}}]}}),
        json!({"uuid":uuid(2),"type":"user","timestamp":at(2),"message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"first","is_error":true,"content":"first failure"},
            {"type":"tool_result","tool_use_id":"second","is_error":true,"content":"second failure"}]}}),
    ]
}

fn synthetic_agent(root: &Path, home: &Path, wait: bool) -> RunEnvironment {
    use std::os::unix::fs::PermissionsExt;
    let binary = root.join("fake-coding-tool");
    let answer = json!({"what_happened":"Two synthetic calls failed.","real_problem":"no","suggestion":"nothing","draft":"","why":"In [1] the tool was called."});
    let result = json!({"is_error":false,"result":answer.to_string(),"modelUsage":{"claude-haiku-synthetic":{}}});
    std::fs::write(&binary, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.304 (Claude Code)'; exit 0; fi\nif [ \"$1\" = auth ]; then echo '{{\"loggedIn\":true}}'; exit 0; fi\n/bin/cat > \"$HOME/sent-prompt.txt\"\necho started > \"$HOME/model-started\"\n{}\nprintf '%s\\n' '{result}'\n",
        if wait { "/bin/sleep 30" } else { ":" },
    )).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    RunEnvironment {
        inherited_env: vec![
            ("HOME".into(), home.into()),
            ("PATH".into(), root.into()),
            (
                xt_probes::executable::CLAUDE_ENV.into(),
                binary.clone().into(),
            ),
            (agents::CODEX_ENV.into(), binary.clone().into()),
            (agents::CURSOR_ENV.into(), binary.into()),
        ],
        temp_dir: root.into(),
        limits: AgentLimits {
            deadline: Duration::from_secs(20),
            check_deadline: Duration::from_secs(5),
            exit_grace: Duration::from_millis(100),
            term_grace: Duration::from_millis(100),
            ..AgentLimits::default()
        },
    }
}

#[test]
fn native_diagnosis_runs_a_fake_tool_and_returns_the_exact_sent_messages() {
    let (root, state, sid, home) = native_diagnosis_scene(&repeated_failure_lines());
    let environment = synthetic_agent(root.path(), &home, false);
    let runs = Arc::new(DiagnosisRuns::default());
    let warning_id = format!("repeated_failure:{}#0", uuid(2));
    let request = Request {
        session_id: &sid,
        warning_id: &warning_id,
        timezone: "UTC",
        read_id: "native-run",
    };
    let before = state.with_store(|store| Ok(store.counts()?)).unwrap();
    let found = diagnose(
        &state,
        reserve(&runs, &request).unwrap().unwrap(),
        &crate::native_index::DetailReaders::disabled(),
        &environment,
        &request,
    )
    .unwrap();
    let SessionDiagnosis::Diagnosed {
        tool,
        model,
        messages,
        sent,
        ..
    } = found
    else {
        panic!("{found:?}")
    };
    assert_eq!(tool, dto::DiagnosisTool::ClaudeCode);
    assert_eq!(model.as_deref(), Some("claude-haiku-synthetic"));
    assert_eq!(sent.tools, [dto::DiagnosisTool::ClaudeCode]);
    assert_eq!(messages.len(), 3);
    assert!(messages[2].flagged);
    assert!(messages[2].text.ends_with("second failure"));
    let prompt = std::fs::read_to_string(home.join("sent-prompt.txt")).unwrap();
    assert!(prompt.contains("first failure") && prompt.contains("second failure"));
    assert_eq!(
        std::fs::read_dir(root.path().join("xtrace-grading/diagnose"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        state.with_store(|store| Ok(store.counts()?)).unwrap(),
        before
    );
    assert!(
        reserve(&runs, &request).unwrap().is_some(),
        "the completed run releases admission"
    );
    state.shutdown();
}

#[test]
fn native_shutdown_cancels_a_running_fake_tool_and_closes_admission() {
    let (root, state, sid, home) = native_diagnosis_scene(&repeated_failure_lines());
    let environment = synthetic_agent(root.path(), &home, true);
    let runs = Arc::new(DiagnosisRuns::default());
    let warning_id = format!("repeated_failure:{}#0", uuid(2));
    let request = Request {
        session_id: &sid,
        warning_id: &warning_id,
        timezone: "UTC",
        read_id: "native-run",
    };
    let slot = reserve(&runs, &request).unwrap().unwrap();
    let shutdown_runs = Arc::clone(&runs);
    let ready = home.join("model-started");
    let shutdown = std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        let running = ready.exists();
        shutdown_runs.shutdown();
        running
    });
    let started = Instant::now();
    let found = diagnose(
        &state,
        slot,
        &crate::native_index::DetailReaders::disabled(),
        &environment,
        &request,
    )
    .unwrap();
    assert!(
        shutdown.join().unwrap(),
        "shutdown waited until the fake attempt started"
    );
    assert!(
        matches!(found, SessionDiagnosis::Cancelled { .. }),
        "{found:?}"
    );
    assert!(started.elapsed() < SHUTDOWN_BOUND);
    assert!(reserve(&runs, &request).unwrap().is_none());
    assert_eq!(
        std::fs::read_dir(root.path().join("xtrace-grading/diagnose"))
            .unwrap()
            .count(),
        0
    );
    state.shutdown();
}

#[test]
fn shutdown_cancels_both_tokens_and_waits_for_the_slot_guard() {
    let runs = Arc::new(DiagnosisRuns::default());
    let slot = runs.begin("held").unwrap();
    let closer_runs = Arc::clone(&runs);
    let started = Instant::now();
    let closer = std::thread::spawn(move || closer_runs.shutdown());
    let end = Instant::now() + Duration::from_secs(2);
    while !slot.agent.is_cancelled() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(slot.source.is_cancelled() && slot.agent.is_cancelled());
    assert!(
        !closer.is_finished(),
        "shutdown waits for the reserved slot to finish"
    );
    assert!(runs.begin("later").is_none());
    drop(slot);
    closer.join().unwrap();
    assert!(started.elapsed() < SHUTDOWN_BOUND);
}

#[test]
fn bounded_diagnosis_context_keeps_a_late_flag_and_only_its_subagent_messages() {
    use std::io::Write;
    let lines:Vec<_>=(1..=30).map(|n|json!({"uuid":uuid(n),"type":"user","timestamp":at(n),"message":{"role":"user","content":"main-only context"}})).collect();
    let (root, state, sid, home) = native_diagnosis_scene(&lines);
    let project = home.join(".claude/projects/-diagnose-run-fixture");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{sid}.jsonl")))
        .unwrap();
    let ignored = format!(
        "{}\n",
        json!({"type":"progress","data":"x".repeat(1024*1024)})
    );
    for _ in 0..65 {
        file.write_all(ignored.as_bytes()).unwrap();
    }
    drop(file);
    let agents = project.join(&sid).join("subagents");
    std::fs::create_dir_all(&agents).unwrap();
    let other = json!({"uuid":uuid(40),"type":"user","sessionId":sid,"isSidechain":true,"agentId":"agent-a","timestamp":at(40),"message":{"role":"user","content":"other-agent-only context"}});
    std::fs::write(agents.join("agent-a.jsonl"), format!("{other}\n")).unwrap();
    let mut blocks = vec![json!({"type":"text","text":"long prose ".repeat(2000)})];
    for n in 0..25 {
        blocks.push(json!({"type":"tool_use","id":format!("setup-{n}"),"name":"setup","input":{}}));
    }
    blocks.push(json!({"type":"tool_use","id":"repeat-call","name":"write_stdin","input":{"session_id":123,"chars":""}}));
    let own = vec![
        json!({"uuid":uuid(50),"type":"user","sessionId":sid,"isSidechain":true,"agentId":"agent-b","timestamp":at(50),"message":{"role":"user","content":"own-agent context"}}),
        json!({"uuid":uuid(51),"type":"assistant","sessionId":sid,"isSidechain":true,"agentId":"agent-b","timestamp":at(51),"message":{"role":"assistant","content":blocks}}),
        json!({"uuid":uuid(52),"type":"user","sessionId":sid,"isSidechain":true,"agentId":"agent-b","timestamp":at(52),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"repeat-call","content":"test process still running"}]}}),
    ];
    std::fs::write(
        agents.join("agent-b.jsonl"),
        own.iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    state
        .with_store(|store| {
            let parsed: Vec<_> = std::iter::once(other).chain(own).map(record).collect();
            let records: Vec<_> = parsed
                .iter()
                .map(|record| record.canonical.clone())
                .collect();
            let identities: Vec<_> = parsed
                .iter()
                .map(|record| xt_store::model::RecordIdentity {
                    agent_id: record.native.agent_id.clone(),
                    ..Default::default()
                })
                .collect();
            let meta = store.session(&sid)?.unwrap().meta;
            let mut batch = xt_store::batch::IngestBatch::new(&meta, &records, false);
            batch.identities = &identities;
            store.apply_ingest_batch(&batch)?;
            Ok(())
        })
        .unwrap();
    let readers = crate::native_index::DetailReaders::disabled();
    assert!(matches!(
        state
            .loaded_source(&sid, &CancelToken::new(), &readers)
            .unwrap(),
        Err(crate::dto::SessionSourceReason::TooLarge { .. })
    ));
    let point = WarningPoint {
        record_uuid: uuid(51),
        block: Some(26),
    };
    let records = state
        .diagnosis_context(&sid, &point, &CancelToken::new(), &readers)
        .unwrap()
        .unwrap();
    assert_eq!(records.iter().position(|record|record.canonical.uuid.as_deref()==Some(point.record_uuid.as_str())),Some(12));
    assert!(
        records
            .iter()
            .any(|record| record.canonical.uuid.as_deref() == Some(uuid(40).as_str()))
    );
    let facts = state
        .with_store(|store| session_facts(store, &sid))
        .unwrap()
        .unwrap();
    assert_eq!(
        facts.records[&uuid(51)].agent_id.as_deref(),
        Some("agent-b")
    );
    let (messages, flagged) =
        conversation(&records, &facts.records, &point, WarningSender::Person).unwrap();
    assert!(messages[flagged].text.starts_with("(ran write_stdin:"));
    assert!(
        !messages
            .iter()
            .any(|message| message.text.contains("main-only")
                || message.text.contains("other-agent-only"))
    );
    let built = input::build(
        &input::Header {
            kind: input::WarningKind::RepeatedFailure,
            reason: "the same tool call failed 2 times".into(),
            host: HostName::ClaudeCode,
            project: "synthetic".into(),
        },
        &messages,
        flagged,
    )
    .unwrap();
    assert!(built.text.contains("own-agent context"));
    assert!(
        built.text.contains("write_stdin") && built.text.contains("test process still running")
    );
    state.shutdown();
    drop(root);
}

#[test]
fn native_diagnosis_of_an_admitted_mistake_sends_the_flagged_reply() {
    let lines = vec![
        json!({"uuid":uuid(1),"type":"user","timestamp":at(1),"message":{"role":"user","content":[{"type":"text","text":"the parser drops the last line"}]}}),
        json!({"uuid":uuid(2),"type":"assistant","timestamp":at(2),"message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"You're right, I misread the loop."}]}}),
    ];
    let (root, state, sid, home) = native_diagnosis_scene(&lines);
    let warning_id = format!("admitted_mistake:{}", uuid(2));
    let (warning, _) = state
        .session_warning(&sid, &warning_id, "UTC")
        .unwrap()
        .expect("the indexed admission is a warning");
    assert_eq!(warning.points_at(), flag(2));
    let environment = synthetic_agent(root.path(), &home, false);
    let runs = Arc::new(DiagnosisRuns::default());
    let request = Request {
        session_id: &sid,
        warning_id: &warning_id,
        timezone: "UTC",
        read_id: "admitted-run",
    };
    let found = diagnose(
        &state,
        reserve(&runs, &request).unwrap().unwrap(),
        &crate::native_index::DetailReaders::disabled(),
        &environment,
        &request,
    )
    .unwrap();
    let SessionDiagnosis::Diagnosed { messages, .. } = found else {
        panic!("{found:?}")
    };
    assert_eq!(messages.len(), 2);
    assert!(!messages[0].flagged);
    assert!(messages[0].text.contains("the parser drops the last line"));
    assert!(messages[1].flagged);
    assert!(
        messages[1]
            .text
            .contains("You're right, I misread the loop.")
    );
    let prompt = std::fs::read_to_string(home.join("sent-prompt.txt")).unwrap();
    assert!(prompt.contains("warning_kind: admitted_mistake"));
    assert!(prompt.contains("[2] agent (flagged):"));
    state.shutdown();
}

#[test]
fn a_whole_record_warning_whose_source_changed_to_assistant_stops_before_the_tool() {
    let lines = vec![
        json!({"uuid":uuid(1),"type":"assistant","timestamp":at(1),"message":{"role":"assistant","content":[{"type":"text","text":"I updated the first step."}]}}),
        json!({"uuid":uuid(2),"type":"user","timestamp":at(2),"message":{"role":"user","content":[{"type":"text","text":"no, the other step"}]}}),
    ];
    let (root, state, sid, home) = native_diagnosis_scene(&lines);
    state
        .with_store(|store| {
            store.set_retention_mode(xt_store::retention::RetentionMode::FullContent)?;
            store.upsert_records(
                &sid,
                &lines
                    .iter()
                    .cloned()
                    .map(|line| record(line).canonical)
                    .collect::<Vec<_>>(),
                true,
            )?;
            Ok(())
        })
        .unwrap();
    let warning_id = format!("correction:{}", uuid(2));
    let original = state
        .session_warning(&sid, &warning_id, "UTC")
        .unwrap()
        .expect("the indexed user correction is a warning")
        .0;
    assert_eq!(original.points_at(), flag(2));
    let mut changed = lines;
    for line in &mut changed {
        line["sessionId"] = json!(sid);
    }
    changed[1]["type"] = json!("assistant");
    changed[1]["message"]["role"] = json!("assistant");
    std::fs::write(
        home.join(".claude/projects/-diagnose-run-fixture")
            .join(format!("{sid}.jsonl")),
        changed
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    assert!(
        state
            .session_warning(&sid, &warning_id, "UTC")
            .unwrap()
            .is_some(),
        "the indexed warning remains unchanged"
    );
    let readers = crate::native_index::DetailReaders::disabled();
    let loaded = state
        .loaded_source(&sid, &CancelToken::new(), &readers)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.generation,
        xt_ingest::native::session_source::GenerationBasis::Unrecorded
    );
    assert_eq!(
        SourceRecord::try_from(&loaded.records[1]).unwrap().role,
        crate::transcript_dto::RecordRole::Assistant
    );
    drop(loaded);
    let environment = synthetic_agent(root.path(), &home, false);
    let runs = Arc::new(DiagnosisRuns::default());
    let request = Request {
        session_id: &sid,
        warning_id: &warning_id,
        timezone: "UTC",
        read_id: "changed-role",
    };
    let found = diagnose(
        &state,
        reserve(&runs, &request).unwrap().unwrap(),
        &readers,
        &environment,
        &request,
    )
    .unwrap();
    let called = home.join("model-started").exists();
    let staged = root.path().join("xtrace-grading").exists();
    assert!(
        matches!(found, SessionDiagnosis::FlaggedMessageNotFound { .. }) && !called && !staged,
        "outcome={found:?}; fake_tool_called={called}; staging_started={staged}"
    );
    state.shutdown();
}

#[test]
fn role_checks_keep_automatic_user_interruptions_and_explicit_assistant_calls() {
    let records = vec![person(1, "[Request interrupted by user]")];
    let (messages, flagged) =
        conversation(&records, &HashMap::new(), &flag(1), WarningSender::Person).unwrap();
    assert_eq!(flagged, 0);
    assert_eq!(messages[0].sender, Sender::Automatic);
    let records = vec![agent(
        2,
        "call",
        json!([{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"/synthetic/a"}}]),
    )];
    let (messages, flagged) = conversation(
        &records,
        &HashMap::new(),
        &WarningPoint {
            record_uuid: uuid(2),
            block: Some(0),
        },
        WarningSender::Person,
    )
    .unwrap();
    assert_eq!(flagged, 0);
    assert_eq!(messages[0].sender, Sender::Agent);
    assert!(messages[0].text.starts_with("(ran Read:"));
    let records = vec![result(3, "read", "synthetic result", false)];
    let (messages, flagged) =
        conversation(&records, &HashMap::new(), &flag(3), WarningSender::Person).unwrap();
    assert_eq!(flagged, 0);
    assert_eq!(messages[0].sender, Sender::ToolResult { tool: None });
}
