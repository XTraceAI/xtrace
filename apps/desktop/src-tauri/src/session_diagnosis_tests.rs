//! Pure diagnosis conversation and outcome translation with synthetic data.
use super::*;
use serde_json::{Value, json};
use xt_ingest::canonical::{Parsed, parse_line};
use xt_judge::diagnosis::{
    agents::{Diagnosis, Model, RunFailure, Skip},
    input::Sender,
};
use xt_metrics::WarningKind;

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
    let (messages, flagged) = conversation(&records, &facts(&[1, 5, 7]), &flag(7)).unwrap();
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
    assert!(conversation(&records, &facts(&[]), &flag(99)).is_none());
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
    let (messages, flagged) = conversation(&records, &HashMap::new(), &flag).unwrap();
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
            }
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
                    }
                )
                .is_none(),
                "{role} {records:?}"
            );
            assert!(conversation(&records, &HashMap::new(), &flag(2)).is_some());
        }
    }
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
                }
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
    let (main, flagged) = conversation(&records, &facts, &flag(4)).unwrap();
    assert_eq!(
        main.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["main one", "main two"]
    );
    assert_eq!(flagged, 1);
    let (sub, _) = conversation(&records, &facts, &flag(3)).unwrap();
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
            compact_reason: "After an agent reply".into(),
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
    let (messages, flagged) =
        conversation(&records, &HashMap::new(), &repeated.points_at()).unwrap();
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
    )
    .unwrap();
    let built = input::build(
        &input::Header {
            kind: input::WarningKind::RepeatedCalls,
            reason: "repeated calls".into(),
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
                conversation(&records, &saved, &flag(n)).is_none(),
                "unknown saved subagent ID {missing:?} must stop before choosing messages"
            );
        }
        let (main, _) = conversation(&records, &saved, &flag(1)).unwrap();
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
fn save_regression_failed_create_new_preserves_a_competing_file() {
    let (_dir, project) = project();
    let hooks = SaveTestHooks {
        before_open: Some(|path| std::fs::write(path, "competing writer's skill").unwrap()),
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
        let hooks = SaveTestHooks {
            before_open: (setup == "unrelated file in new folder").then_some(|path: &Path| {
                std::fs::write(
                    path.parent().unwrap().join("unrelated.txt"),
                    "keep unrelated",
                )
                .unwrap();
            }),
            write: Some(|file, _| {
                use std::io::Write;
                file.write_all(b"owned partial skill")?;
                Err(std::io::Error::other("injected write failure"))
            }),
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
            _ => assert_eq!(
                std::fs::read_to_string(skill.join("unrelated.txt")).unwrap(),
                "keep unrelated"
            ),
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
