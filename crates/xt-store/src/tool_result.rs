//! How each tool call ended, as metadata: the single owner of the rule that
//! reads a `tool_result` block's outcome, and of the call identifier that
//! links a result to the call it answers.
//!
//! Both are derived from the record's own blocks at write time, as the
//! interruption flag and the structural tool columns are, so a metadata-only
//! index keeps them once the blocks are discarded. Nothing here keeps text: a
//! result's words are read only to recognise Claude Code's fixed "declined"
//! wording, in memory, and dropped.
//!
//! Rows fill once. A replay that observes the same outcome changes nothing; one
//! that observes another marks the row conflicted, and a call observed with
//! two different identifiers links to nothing. Readers treat both as unknown.

use crate::{
    Host, Result,
    batch::RecordOutcome,
    model::{CanonicalRecord, RecordType},
};
use rusqlite::{Connection, params};
use serde_json::Value;

/// The start of the tool result Claude Code writes when the person declines a
/// tool call at its permission prompt. It always comes with `is_error: true`.
pub const DECLINED_PREFIX: &str = "The user doesn't want to proceed with this tool use";
const RULE_VERSION: i64 = 1;
/// The longest call identifier kept (the tables' CHECK).
const MAX_CALL_ID_CHARS: usize = 256;

/// How one tool call ended, as its result block states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
    Declined,
    /// The block does not say.
    Unknown,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Declined => "declined",
            Self::Unknown => "unknown",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "ok" => Self::Ok,
            "failed" => Self::Failed,
            "declined" => Self::Declined,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

/// The text of a result's `content`, which the host writes as a string, as a
/// list of parts, or not at all.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// The outcome one `tool_result` block states.
///
/// `is_error: true` is a failure, or a decline when the result is Claude
/// Code's declined wording; `is_error: false` is a success. Claude Code omits
/// `is_error` on many successful results and the Messages API defines an
/// omitted `is_error` as false, so for a Claude session an omitted flag is a
/// success. Codex and Cursor results reach the index through the pinned
/// readers, which never state the flag, so there an omitted flag is unknown.
/// Anything else stated in the flag is unknown.
pub fn outcome(block: &Value, host: Host) -> Outcome {
    match block.get("is_error") {
        Some(Value::Bool(true)) if result_text(block).starts_with(DECLINED_PREFIX) => {
            Outcome::Declined
        }
        Some(Value::Bool(true)) => Outcome::Failed,
        Some(Value::Bool(false)) => Outcome::Ok,
        None if host == Host::Claude => Outcome::Ok,
        _ => Outcome::Unknown,
    }
}

/// A block's call identifier when it is one the tables can hold.
fn call_id<'a>(block: &'a Value, field: &str) -> Option<&'a str> {
    block
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && id.chars().count() <= MAX_CALL_ID_CHARS)
}

/// Keep the call identifiers and result outcomes the accepted inputs of one
/// write state. Only a record this session owns, unconflicted, is written to
/// (each statement checks that itself, so no row is read for it); an input
/// whose content was not observed states nothing. Returns how many result rows
/// were newly kept.
pub(crate) fn apply(
    connection: &Connection,
    session_id: &str,
    host: Host,
    records: &[CanonicalRecord],
    outcomes: &[RecordOutcome],
) -> Result<usize> {
    let mut kept = 0;
    for outcome in outcomes {
        let Some(uuid) = outcome.uuid.as_deref() else {
            continue;
        };
        if !outcome.disposition.is_accepted() {
            continue;
        }
        let input = &records[outcome.input_index];
        let Some(blocks) = &input.message.content else {
            continue;
        };
        for (index, block) in blocks.iter().enumerate() {
            let index = index as i64;
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") if input.record_type == RecordType::Assistant => {
                    let Some(id) = call_id(block, "id") else {
                        continue;
                    };
                    connection.execute(
                        "UPDATE tool_uses SET call_id=?3 WHERE uuid=?1 AND block_index=?2
                           AND session_id=?4 AND call_id IS NULL AND EXISTS(SELECT 1 FROM records
                           WHERE uuid=?1 AND session_id=?4 AND has_conflict=0)",
                        params![uuid, index, id, session_id],
                    )?;
                    connection.execute(
                        "UPDATE tool_uses SET call_id_conflict=1 WHERE uuid=?1 AND block_index=?2
                           AND session_id=?4 AND call_id IS NOT ?3",
                        params![uuid, index, id, session_id],
                    )?;
                }
                Some("tool_result") if input.record_type == RecordType::User => {
                    let id = call_id(block, "tool_use_id");
                    let ended = self::outcome(block, host);
                    kept += connection.execute(
                        "INSERT INTO tool_results(record_uuid,session_id,block_index,call_id,outcome,rule_version)
                         SELECT ?1,?2,?3,?4,?5,?6 WHERE EXISTS(SELECT 1 FROM records
                           WHERE uuid=?1 AND session_id=?2 AND has_conflict=0)
                         ON CONFLICT(record_uuid,block_index) DO NOTHING",
                        params![uuid, session_id, index, id, ended.as_str(), RULE_VERSION],
                    )?;
                    connection.execute(
                        "UPDATE tool_results SET conflicted=1 WHERE record_uuid=?1 AND block_index=?2
                           AND session_id=?5 AND (call_id IS NOT ?3 OR outcome IS NOT ?4)",
                        params![uuid, index, id, ended.as_str(), session_id],
                    )?;
                }
                _ => {}
            }
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_outcome_is_the_flag_and_the_declined_wording() {
        let result = |flag: Value, content: Value| {
            let mut block = json!({"type":"tool_result","tool_use_id":"t","content":content});
            if !flag.is_null() {
                block["is_error"] = flag;
            }
            block
        };
        let declined = json!(
            "The user doesn't want to proceed with this tool use. The tool use was rejected."
        );
        for (block, claude, other) in [
            (
                result(json!(true), json!("exit 1")),
                Outcome::Failed,
                Outcome::Failed,
            ),
            (
                result(json!(true), declined.clone()),
                Outcome::Declined,
                Outcome::Declined,
            ),
            (
                result(json!(true), json!([{"type":"text","text":declined}])),
                Outcome::Declined,
                Outcome::Declined,
            ),
            // The wording without the flag is an ordinary result.
            (result(json!(false), declined), Outcome::Ok, Outcome::Ok),
            (
                result(Value::Null, json!("done")),
                Outcome::Ok,
                Outcome::Unknown,
            ),
            (
                result(json!("yes"), json!("done")),
                Outcome::Unknown,
                Outcome::Unknown,
            ),
        ] {
            assert_eq!(outcome(&block, Host::Claude), claude, "{block}");
            assert_eq!(outcome(&block, Host::Codex), other, "{block}");
        }
    }
}
