//! A complete-record portion, never a whole-transcript status.
use crate::{
    session_source_dto::{SessionSourceReason, SessionSourceStatus},
    transcript_dto::SourceRecord,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_ingest::native::{
    readers_cli::ContextRefusal,
    session_source::{
        CODEX_PAGE_OUTPUT_BYTES, CodexPage, CodexPageError, CodexPageSelection,
        SessionSourceOutcome,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodexPageRequest {
    First,
    Cursor {
        cursor: String,
    },
    ToolCall {
        record_uuid: String,
        block: u32,
        generation: Option<String>,
    },
}
impl CodexPageRequest {
    pub fn native(&self) -> CodexPageSelection {
        match self {
            Self::First => CodexPageSelection::First,
            Self::Cursor { cursor } => CodexPageSelection::Cursor(cursor.clone()),
            Self::ToolCall {
                record_uuid,
                block,
                generation,
            } => CodexPageSelection::ToolCall {
                record_uuid: record_uuid.clone(),
                block: *block,
                generation: generation.clone(),
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CodexPageReason {
    Source { detail: SessionSourceReason },
    Changed,
    InvalidCursor,
    TargetMissing,
    TargetAmbiguous,
    BlockMissing,
    RecordTooLarge,
    UnsupportedHistory,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CodexPageStatus {
    Loaded {
        digest: String,
        records: Vec<SourceRecord>,
        start: u32,
        end: u32,
        total: u32,
        previous: Option<String>,
        next: Option<String>,
        target: Option<u32>,
    },
    Unavailable {
        reason: CodexPageReason,
    },
}
impl CodexPageStatus {
    pub fn unavailable(detail: SessionSourceReason) -> Self {
        Self::Unavailable {
            reason: CodexPageReason::Source { detail },
        }
    }
    pub fn from_read(result: Result<CodexPage, CodexPageError>) -> Result<Self, &'static str> {
        let mut page = match result {
            Ok(page) => page,
            Err(CodexPageError::Source(reason)) => {
                let SessionSourceStatus::Unavailable { reason } =
                    SessionSourceStatus::try_from(&SessionSourceOutcome::Unavailable(reason))?
                else {
                    unreachable!()
                };
                return Ok(Self::unavailable(reason));
            }
            Err(CodexPageError::UnsupportedHistory) => {
                return Ok(Self::Unavailable {
                    reason: CodexPageReason::UnsupportedHistory,
                });
            }
            Err(CodexPageError::Context(reason)) => {
                return Ok(Self::Unavailable {
                    reason: match reason {
                        ContextRefusal::Changed => CodexPageReason::Changed,
                        ContextRefusal::InvalidCursor => CodexPageReason::InvalidCursor,
                        ContextRefusal::TargetMissing => CodexPageReason::TargetMissing,
                        ContextRefusal::TargetAmbiguous => CodexPageReason::TargetAmbiguous,
                        ContextRefusal::BlockMissing => CodexPageReason::BlockMissing,
                        ContextRefusal::RecordTooLarge => CodexPageReason::RecordTooLarge,
                    },
                });
            }
        };
        let mut wire = page
            .session
            .records
            .iter()
            .map(SourceRecord::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        loop {
            if std::time::Instant::now() >= page.deadline {
                return Ok(Self::unavailable(SessionSourceReason::ReaderDeadline));
            }
            let start = page.start;
            let end = start + wire.len();
            let status = Self::Loaded {
                digest: page.digest.clone(),
                records: wire,
                start: start as u32,
                end: end as u32,
                total: page.total as u32,
                previous: (start > 0).then(|| page.cursor(true, start)),
                next: (end < page.total).then(|| page.cursor(false, end)),
                target: page.target.map(|t| (t - start) as u32),
            };
            if serde_json::to_writer(Counter(0), &status).is_ok() {
                return Ok(status);
            }
            let Self::Loaded { records, .. } = status else {
                unreachable!()
            };
            wire = records;
            if wire.len() <= 1 {
                return Ok(Self::Unavailable {
                    reason: CodexPageReason::RecordTooLarge,
                });
            }
            let front = page
                .target
                .map_or(page.backwards, |t| t - start > end - 1 - t);
            if front {
                wire.remove(0);
                page.start += 1;
            } else {
                wire.pop();
            }
        }
    }
}
struct Counter(usize);
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > CODEX_PAGE_OUTPUT_BYTES {
            return Err(std::io::Error::other("page output limit"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};
    use xt_ingest::{
        canonical::{Parsed, parse_line},
        native::session_source::{GenerationBasis, LoadedSession},
    };
    fn page(count: usize, text: &str, backwards: bool, target: Option<usize>) -> CodexPage {
        let records = (10..10+count).map(|i| {
            let Parsed::Record(record) = parse_line(&json!({"uuid":format!("r{i}"),"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string()).unwrap() else { panic!("record") };
            *record
        }).collect();
        CodexPage {
            session: LoadedSession {
                native_session_id: "synthetic".into(),
                host: xt_store::Host::Codex,
                generation: GenerationBasis::Unrecorded,
                sources: vec![],
                records,
                other_lines: 0,
                dropped_records: 0,
                gaps: vec![],
            },
            digest: "a".repeat(64),
            start: 10,
            total: 20,
            target,
            backwards,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    #[test]
    fn encoded_bound_preserves_contiguous_edges_or_exact_target_and_advancing_cursors() {
        for (backwards, target, expected_start, expected_end) in [
            (false, None, 10, 12),
            (true, None, 11, 13),
            (false, Some(11), 10, 12),
        ] {
            let status =
                CodexPageStatus::from_read(Ok(page(3, &"x".repeat(400_000), backwards, target)))
                    .unwrap();
            let CodexPageStatus::Loaded {
                records,
                start,
                end,
                previous,
                next,
                target: wire_target,
                ..
            } = &status
            else {
                panic!("{status:?}")
            };
            assert_eq!((*start, *end), (expected_start, expected_end));
            assert_eq!(records[0].id, format!("r{start}"));
            assert_eq!(records.last().unwrap().id, format!("r{}", end - 1));
            assert!(
                previous
                    .as_ref()
                    .unwrap()
                    .starts_with(&format!("v1:before:{start}:"))
            );
            assert!(
                next.as_ref()
                    .unwrap()
                    .starts_with(&format!("v1:after:{end}:"))
            );
            if let Some(target) = target {
                assert_eq!(
                    records[wire_target.unwrap() as usize].id,
                    format!("r{target}")
                );
            }
            assert!(serde_json::to_vec(&status).unwrap().len() <= CODEX_PAGE_OUTPUT_BYTES);
        }
    }
    #[test]
    fn oversized_required_record_and_expired_encoding_never_return_empty_success() {
        assert!(matches!(
            CodexPageStatus::from_read(Ok(page(1, &"\0".repeat(200_000), false, None))).unwrap(),
            CodexPageStatus::Unavailable {
                reason: CodexPageReason::RecordTooLarge
            }
        ));
        let mut expired = page(1, "synthetic", false, None);
        expired.deadline = Instant::now() - Duration::from_secs(1);
        assert!(matches!(
            CodexPageStatus::from_read(Ok(expired)).unwrap(),
            CodexPageStatus::Unavailable {
                reason: CodexPageReason::Source {
                    detail: SessionSourceReason::ReaderDeadline
                }
            }
        ));
    }
}
