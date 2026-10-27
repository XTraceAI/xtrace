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
    /// Set on the target record of a correction warning only: the words
    /// M-21's own matcher (`xt_metrics::correction_spans`) finds a
    /// correction in, over this record's text blocks as shown. Empty
    /// everywhere else, and where the shown text has no match.
    pub correction_marks: Vec<TextMark>,
}

/// A stretch of one text block's text. `start` and `end` count UTF-16 code
/// units, as a JavaScript string indexes it (`text.slice(start, end)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct TextMark {
    /// The text block's `index` in its record.
    pub block: u32,
    pub start: u32,
    pub end: u32,
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
    /// Marks where the target record's text says it is a correction, by
    /// M-21's own matcher over its text blocks as shown (joined by line
    /// breaks, so no word spans two blocks). For a correction warning only.
    pub fn with_correction_marks(mut self) -> Self {
        let Self::Loaded { records, .. } = &mut self else {
            return self;
        };
        let Some(target) = records.iter_mut().find(|entry| entry.target) else {
            return self;
        };
        let blocks: Vec<(u32, &str)> = target
            .record
            .blocks
            .iter()
            .filter_map(|block| match block {
                SourceBlock::Text { index, text } => Some((*index, text.as_str())),
                _ => None,
            })
            .collect();
        let joined = blocks
            .iter()
            .map(|(_, text)| *text)
            .collect::<Vec<_>>()
            .join("\n");
        let Some(spans) = xt_metrics::correction_spans(&joined) else {
            return self;
        };
        let mut marks = Vec::new();
        let mut offset = 0;
        for (index, text) in &blocks {
            let range = offset..offset + text.len();
            for span in spans.iter().filter(|span| range.contains(&span.start)) {
                let utf16 = |byte: usize| text[..byte - offset].encode_utf16().count() as u32;
                marks.push(TextMark {
                    block: *index,
                    start: utf16(span.start),
                    end: utf16(span.end),
                });
            }
            offset = range.end + 1;
        }
        target.correction_marks = marks;
        self
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
                    correction_marks: Vec::new(),
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
    fn correction_marks_only_the_target_in_utf16_and_only_when_asked() {
        let neighbour = record(json!([{"type":"text","text":"that was wrong"}]));
        let target = record(json!([
            {"type":"text","text":"🙂 ok"},
            {"type":"text","text":"Then I said the left panel"}
        ]));
        let status = SessionExcerptStatus::from_read(
            Ok(LoadedExcerpt {
                generation: GenerationBasis::Unrecorded,
                records: vec![neighbour, target],
                target: 1,
            }),
            None,
        )
        .unwrap();
        // Not a correction warning: nothing is marked.
        let SessionExcerptStatus::Loaded { records, .. } = &status else {
            panic!("loaded");
        };
        assert!(
            records
                .iter()
                .all(|entry| entry.correction_marks.is_empty())
        );
        let SessionExcerptStatus::Loaded { records, .. } = status.with_correction_marks() else {
            panic!("loaded");
        };
        assert!(records[0].correction_marks.is_empty());
        let SourceBlock::Text { index, text } = &records[1].record.blocks[1] else {
            panic!("text");
        };
        assert_eq!(
            records[1].correction_marks,
            [
                TextMark {
                    block: *index,
                    start: 5,
                    end: 6
                },
                TextMark {
                    block: *index,
                    start: 7,
                    end: 11
                },
            ]
        );
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[5..6]).unwrap(), "I");
        assert_eq!(String::from_utf16(&units[7..11]).unwrap(), "said");
        // A target with no match stays unmarked.
        let plain = status_of(record(
            json!([{"type":"text","text":"add an export button"}]),
        ));
        let SessionExcerptStatus::Loaded { records, .. } = plain.with_correction_marks() else {
            panic!("loaded");
        };
        assert!(records[0].correction_marks.is_empty());
        // A multi-byte character before the match counts as two UTF-16 units.
        let emoji = status_of(record(json!([{"type":"text","text":"🙂 that is wrong"}])));
        let SessionExcerptStatus::Loaded { records, .. } = emoji.with_correction_marks() else {
            panic!("loaded");
        };
        assert_eq!(records[0].correction_marks[0].start, 11);
        assert_eq!(records[0].correction_marks[0].end, 16);
    }
    fn status_of(record: xt_ingest::canonical::ParsedRecord) -> SessionExcerptStatus {
        status(record, None)
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
