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
    /// Marks where the target record says it is a correction, for the
    /// correction warning whose stored phrase is `phrase` (`None` for any
    /// other warning, or none found). M-21's one matcher
    /// (`xt_metrics::correction_spans`) runs over exactly the text the check
    /// read: the record's text blocks joined with nothing between them
    /// (`xt_store::record_text::joined`). Each matched word is split at block
    /// boundaries into one mark per block.
    ///
    /// Marked only when both hold, so a mark never disagrees with the
    /// warning: M-02 counts the whole record as the person's message
    /// (`whole_message`, unknown counts as no), and the matched words,
    /// lowercased and joined by single spaces, are the stored phrase (a cut
    /// message the check read only the start of can match differently).
    pub fn with_correction_marks(
        mut self,
        phrase: Option<&str>,
        persons: &std::collections::BTreeMap<String, xt_store::RecordPerson>,
    ) -> Self {
        let Some(phrase) = phrase else {
            return self;
        };
        let Self::Loaded { records, .. } = &mut self else {
            return self;
        };
        let Some(target) = records.iter_mut().find(|entry| entry.target) else {
            return self;
        };
        let whole = persons
            .get(&target.record.id)
            .and_then(|facts| facts.whole_message);
        if whole != Some(true) {
            return self;
        }
        // Each text block's index and its byte range in the joined text.
        let mut joined = String::new();
        let mut blocks: Vec<(u32, std::ops::Range<usize>)> = Vec::new();
        for block in &target.record.blocks {
            if let SourceBlock::Text { index, text } = block {
                blocks.push((*index, joined.len()..joined.len() + text.len()));
                joined.push_str(text);
            }
        }
        if xt_metrics::correction(&joined).as_deref() != Some(phrase) {
            return self;
        }
        let Some(spans) = xt_metrics::correction_spans(&joined) else {
            return self;
        };
        let utf16 = |text: &str| text.encode_utf16().count() as u32;
        let mut marks = Vec::new();
        for span in &spans {
            for (index, range) in &blocks {
                let start = span.start.max(range.start);
                let end = span.end.min(range.end);
                if start >= end {
                    continue;
                }
                let block = &joined[range.clone()];
                marks.push(TextMark {
                    block: *index,
                    start: utf16(&block[..start - range.start]),
                    end: utf16(&block[..end - range.start]),
                });
            }
        }
        target.correction_marks = marks;
        self
    }
    /// Sets each record's M-02 answer from the index's, by record ID.
    pub fn with_persons(
        mut self,
        persons: &std::collections::BTreeMap<String, xt_store::RecordPerson>,
    ) -> Self {
        if let Self::Loaded { records, .. } = &mut self {
            for entry in records {
                entry.person = persons.get(&entry.record.id).and_then(|facts| facts.person);
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
    use std::collections::BTreeMap;
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
    fn facts(whole: Option<bool>) -> BTreeMap<String, xt_store::RecordPerson> {
        BTreeMap::from([(
            "target".to_owned(),
            xt_store::RecordPerson {
                person: Some(true),
                whole_message: whole,
            },
        )])
    }
    fn marks_of(blocks: serde_json::Value, phrase: &str, whole: Option<bool>) -> Vec<TextMark> {
        let SessionExcerptStatus::Loaded { mut records, .. } =
            status(record(blocks), None).with_correction_marks(Some(phrase), &facts(whole))
        else {
            panic!("loaded");
        };
        records.remove(0).correction_marks
    }
    fn texts(blocks: &[&str]) -> serde_json::Value {
        blocks
            .iter()
            .map(|text| json!({"type":"text","text":text}))
            .collect()
    }
    #[test]
    fn correction_marks_only_the_target_in_utf16_and_only_for_a_correction() {
        let neighbour = record(json!([{"type":"text","text":"that was wrong"}]));
        let target = record(json!([
            {"type":"text","text":"🙂 ok "},
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
        // Not a correction warning (or no warning found): nothing is marked.
        let SessionExcerptStatus::Loaded { records, .. } = status
            .clone()
            .with_correction_marks(None, &facts(Some(true)))
        else {
            panic!("loaded");
        };
        assert!(
            records
                .iter()
                .all(|entry| entry.correction_marks.is_empty())
        );
        let SessionExcerptStatus::Loaded { records, .. } =
            status.with_correction_marks(Some("i said"), &facts(Some(true)))
        else {
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
        // A multi-byte character before the match counts as two UTF-16 units.
        assert_eq!(
            marks_of(texts(&["🙂 that is wrong"]), "wrong", Some(true)),
            [TextMark {
                block: 0,
                start: 11,
                end: 16
            }]
        );
        // The fixture's correction, as the UI fixtures state its marks.
        assert_eq!(
            marks_of(
                texts(&["Still not fixed. Please check the retry limit."]),
                "still not",
                Some(true)
            ),
            [
                TextMark {
                    block: 0,
                    start: 0,
                    end: 5
                },
                TextMark {
                    block: 0,
                    start: 6,
                    end: 9
                },
            ]
        );
    }
    #[test]
    fn no_marks_unless_the_whole_record_is_the_persons_and_the_phrase_agrees() {
        // Only part of the record is the person's (or M-02 cannot say): the
        // correction word may sit in the part that is not.
        assert!(marks_of(texts(&["fix it, wrong file"]), "wrong", Some(false)).is_empty());
        assert!(marks_of(texts(&["fix it, wrong file"]), "wrong", None).is_empty());
        // A cut message: the check read only its start, with the "no" rule
        // off, and found "wrong"; the whole text's first match is "no".
        assert!(marks_of(texts(&["No, that file is wrong"]), "wrong", Some(true)).is_empty());
        // No match at all.
        assert!(marks_of(texts(&["add an export button"]), "wrong", Some(true)).is_empty());
    }
    #[test]
    fn marks_follow_the_checks_own_joining_across_blocks() {
        // Joined with nothing between, "stop" + "watch" is "stopwatch": the
        // check found "wrong", never "stop".
        assert_eq!(
            marks_of(texts(&["stop", "watch is wrong"]), "wrong", Some(true)),
            [TextMark {
                block: 1,
                start: 9,
                end: 14
            }]
        );
        assert!(marks_of(texts(&["stop", "watch is wrong"]), "stop", Some(true)).is_empty());
        // A word split between blocks is one word, marked in each block.
        assert_eq!(
            marks_of(texts(&["I sai", "d the left panel"]), "i said", Some(true)),
            [
                TextMark {
                    block: 0,
                    start: 0,
                    end: 1
                },
                TextMark {
                    block: 0,
                    start: 2,
                    end: 5
                },
                TextMark {
                    block: 1,
                    start: 0,
                    end: 1
                },
            ]
        );
    }
    #[test]
    fn person_comes_only_from_the_index_answer() {
        let loaded = status(record(json!([{"type":"text","text":"words"}])), None);
        assert_eq!(loaded.record_ids(), vec!["target"]);
        let SessionExcerptStatus::Loaded { records, .. } = &loaded else {
            panic!("loaded");
        };
        // A user-role record is not a person's message until M-02 says so.
        assert_eq!(records[0].person, None);
        let persons = BTreeMap::from([(
            "target".to_owned(),
            xt_store::RecordPerson {
                person: Some(false),
                whole_message: Some(true),
            },
        )]);
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
