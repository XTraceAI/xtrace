//! Only the original messages around one verified warning target cross here.
//! Paths and raw parser/reader diagnostics never enter this contract.
use crate::session_source_dto::{SessionGeneration, SessionSourceReason};
use crate::transcript_dto::{SourceBlock, SourceRecord};
use serde::Serialize;
use ts_rs::TS;
use xt_ingest::native::session_source::{
    ExcerptLimit, ExcerptUnavailable, LoadedExcerpt, MAX_EXCERPT_OUTPUT_BYTES,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionExcerptStatus {
    Loaded {
        generation: SessionGeneration,
        records: Vec<SessionExcerptRecord>,
    },
    Unavailable {
        reason: SessionExcerptReason,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionExcerptRecord {
    pub record: SourceRecord,
    pub target: bool,
    pub omitted_blocks: bool,
    /// Computed before filtering, so a selected result never turns a mixed
    /// human message into a tool-only message in the view.
    pub only_tool_results: bool,
    /// M-02: whether the index counts this record as a person's message
    /// (`Store::record_persons`, the answer the diagnosis evidence uses).
    /// `null` where the index has no answer for it. Only `true` is shown as
    /// "You"; the saved role alone never decides it.
    pub person: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SessionExcerptReason {
    Source { source: SessionSourceReason },
    Limit { limit: SessionExcerptLimit },
    Deadline,
    Incomplete,
    TargetMissing,
    TargetAmbiguous,
    BlockMissing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionExcerptLimit {
    ScanBytes,
    LineBytes,
    Records,
    RawLines,
    OutputBytes,
    DiscoveryEntries,
    DiscoveryDepth,
    SourceFiles,
}

impl From<ExcerptLimit> for SessionExcerptLimit {
    fn from(limit: ExcerptLimit) -> Self {
        match limit {
            ExcerptLimit::ScanBytes => Self::ScanBytes,
            ExcerptLimit::LineBytes => Self::LineBytes,
            ExcerptLimit::Records => Self::Records,
            ExcerptLimit::RawLines => Self::RawLines,
            ExcerptLimit::OutputBytes => Self::OutputBytes,
            ExcerptLimit::DiscoveryEntries => Self::DiscoveryEntries,
            ExcerptLimit::DiscoveryDepth => Self::DiscoveryDepth,
            ExcerptLimit::SourceFiles => Self::SourceFiles,
        }
    }
}
impl SessionExcerptStatus {
    pub fn unavailable(reason: SessionExcerptReason) -> Self {
        Self::Unavailable { reason }
    }
    /// The saved record IDs a loaded excerpt shows, for M-02's lookup.
    pub fn record_ids(&self) -> Vec<&str> {
        match self {
            Self::Loaded { records, .. } => records
                .iter()
                .map(|entry| entry.record.id.as_str())
                .collect(),
            Self::Unavailable { .. } => Vec::new(),
        }
    }
    /// Sets each record's M-02 answer from the index's, by record ID.
    pub fn with_persons(mut self, persons: &std::collections::BTreeMap<String, bool>) -> Self {
        if let Self::Loaded { records, .. } = &mut self {
            for entry in records {
                entry.person = persons.get(&entry.record.id).copied();
            }
        }
        self
    }
    pub fn from_read(
        result: Result<LoadedExcerpt, ExcerptUnavailable>,
        block: Option<u32>,
    ) -> Result<Self, &'static str> {
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(reason) => {
                return Ok(Self::unavailable(match reason {
                    ExcerptUnavailable::Source(reason) => SessionExcerptReason::Source {
                        source: crate::session_source_dto::source_reason(&reason)?,
                    },
                    ExcerptUnavailable::Limit(limit) => SessionExcerptReason::Limit {
                        limit: limit.into(),
                    },
                    ExcerptUnavailable::Deadline => SessionExcerptReason::Deadline,
                    ExcerptUnavailable::Incomplete => SessionExcerptReason::Incomplete,
                    ExcerptUnavailable::TargetMissing => SessionExcerptReason::TargetMissing,
                    ExcerptUnavailable::TargetAmbiguous => SessionExcerptReason::TargetAmbiguous,
                    ExcerptUnavailable::BlockMissing => SessionExcerptReason::BlockMissing,
                }));
            }
        };
        let records = loaded
            .records
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let mut record = SourceRecord::try_from(source)?;
                let only_tool_results = !record.blocks.is_empty()
                    && record
                        .blocks
                        .iter()
                        .all(|block| matches!(block, SourceBlock::ToolResult { .. }));
                let target = index == loaded.target;
                let original = record.blocks.len();
                if target && let Some(index) = block {
                    record.blocks.retain(|block| block.index() == index);
                } else {
                    record.blocks.truncate(3);
                }
                Ok(SessionExcerptRecord {
                    omitted_blocks: record.blocks.len() < original,
                    record,
                    target,
                    only_tool_results,
                    person: None,
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        let status = Self::Loaded {
            generation: loaded.generation.into(),
            records,
        };
        // Count encoded bytes without constructing an unbounded JSON string.
        let mut counter = OutputCounter { bytes: 0 };
        if serde_json::to_writer(&mut counter, &status).is_err() {
            return Ok(Self::unavailable(SessionExcerptReason::Limit {
                limit: SessionExcerptLimit::OutputBytes,
            }));
        }
        Ok(status)
    }
}
struct OutputCounter {
    bytes: usize,
}
impl std::io::Write for OutputCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > MAX_EXCERPT_OUTPUT_BYTES {
            return Err(std::io::Error::other("excerpt output limit"));
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
    use xt_ingest::{
        canonical::{Parsed, SourceContext, parse_with_context},
        native::session_source::GenerationBasis,
    };
    fn record(content: serde_json::Value) -> xt_ingest::canonical::ParsedRecord {
        let text=json!({"uuid":"target","type":"user","sessionId":"synthetic", "timestamp":"2026-10-25T12:00:00Z","message":{"role":"user","content":content}}).to_string();
        let Parsed::Record(record) = parse_with_context(&text, &SourceContext::default()).unwrap()
        else {
            panic!("record");
        };
        *record
    }
    fn status(
        record: xt_ingest::canonical::ParsedRecord,
        block: Option<u32>,
    ) -> SessionExcerptStatus {
        SessionExcerptStatus::from_read(
            Ok(LoadedExcerpt {
                generation: GenerationBasis::Unrecorded,
                records: vec![record],
                target: 0,
            }),
            block,
        )
        .unwrap()
    }
    #[test]
    fn selected_result_keeps_original_index_and_mixed_message_caption() {
        let record = record(
            json!([{"type":"text","text":"human words"},{"type":"tool_result","tool_use_id":"call","is_error":true,"content":"synthetic failure"}]),
        );
        let SessionExcerptStatus::Loaded { records, .. } = status(record, Some(1)) else {
            panic!("loaded");
        };
        assert!(!records[0].only_tool_results);
        assert!(records[0].omitted_blocks);
        assert_eq!(records[0].record.blocks.len(), 1);
        assert_eq!(records[0].record.blocks[0].index(), 1);
    }
    #[test]
    fn person_comes_only_from_the_index_answer() {
        let record = record(json!([{"type":"text","text":"words"}]));
        let loaded = status(record, None);
        assert_eq!(loaded.record_ids(), vec!["target"]);
        let SessionExcerptStatus::Loaded { records, .. } = &loaded else {
            panic!("loaded");
        };
        // A user-role record is not a person's message until M-02 says so.
        assert_eq!(records[0].person, None);
        let mut persons = std::collections::BTreeMap::new();
        persons.insert("target".to_owned(), false);
        let SessionExcerptStatus::Loaded { records, .. } = loaded.with_persons(&persons) else {
            panic!("loaded");
        };
        assert_eq!(records[0].person, Some(false));
    }
    #[test]
    fn oversized_serialized_payload_is_refused_without_truncating_text() {
        let record = record(json!([{"type":"text","text":"x".repeat(MAX_EXCERPT_OUTPUT_BYTES)}]));
        assert_eq!(
            status(record, None),
            SessionExcerptStatus::Unavailable {
                reason: SessionExcerptReason::Limit {
                    limit: SessionExcerptLimit::OutputBytes
                }
            }
        );
    }
    #[test]
    fn image_bodies_do_not_cross_the_excerpt_contract() {
        let record = record(json!([{"type":"image","source":{"data":"PRIVATE_IMAGE_BYTES"}}]));
        let encoded = serde_json::to_string(&status(record, None)).unwrap();
        assert!(!encoded.contains("PRIVATE_IMAGE_BYTES"));
        assert!(encoded.contains("unsupported"));
    }
}
