//! Complete-record pages, explicitly distinct from a whole transcript.
use crate::{
    session_excerpt_dto::{SessionExcerptReason, SessionExcerptStatus},
    session_source_dto::{SessionGeneration, SessionSourceRange},
    transcript_dto::SourceRecord,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_ingest::native::session_source::{
    LoadedPage, MAX_PAGE_OUTPUT_BYTES, PageSelection, PageTargetKind, PageUnavailable,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionPageSelection {
    First,
    Cursor {
        cursor: String,
    },
    Anchor {
        record_uuid: String,
        block: Option<u32>,
        expected: SessionPageTargetKind,
        generation: Option<String>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionPageTargetKind {
    Message,
    ToolCall,
    ToolBlock,
}
impl SessionPageSelection {
    pub fn native(&self) -> PageSelection {
        match self {
            Self::First => PageSelection::First,
            Self::Cursor { cursor } => PageSelection::Cursor(cursor.clone()),
            Self::Anchor {
                record_uuid,
                block,
                expected,
                generation,
            } => PageSelection::Anchor {
                record_uuid: record_uuid.clone(),
                block: *block,
                generation: generation.clone(),
                kind: match expected {
                    SessionPageTargetKind::Message => PageTargetKind::Message,
                    SessionPageTargetKind::ToolCall => PageTargetKind::ToolCall,
                    SessionPageTargetKind::ToolBlock => PageTargetKind::ToolBlock,
                },
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionPageStatus {
    Loaded {
        generation: SessionGeneration,
        digest: String,
        records: Vec<SourceRecord>,
        record_ranges: Vec<SessionSourceRange>,
        start: u32,
        end: u32,
        total_records: u32,
        dropped_records: u32,
        previous: Option<String>,
        next: Option<String>,
        /// The native scan proved this message unique across every source.
        target: Option<SessionPageTarget>,
    },
    Unavailable {
        reason: SessionPageReason,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionPageTarget {
    pub record: u32,
    pub block: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SessionPageReason {
    Read { detail: SessionExcerptReason },
    Changed,
    InvalidCursor,
    RecordTooLarge,
}
fn unavailable(reason: PageUnavailable) -> Result<SessionPageStatus, &'static str> {
    Ok(SessionPageStatus::Unavailable {
        reason: match reason {
            PageUnavailable::Changed => SessionPageReason::Changed,
            PageUnavailable::InvalidCursor => SessionPageReason::InvalidCursor,
            PageUnavailable::RecordTooLarge => SessionPageReason::RecordTooLarge,
            PageUnavailable::Read(reason) => {
                let SessionExcerptStatus::Unavailable { reason } =
                    SessionExcerptStatus::from_read(Err(reason), None)?
                else {
                    return Err("unexpected loaded excerpt");
                };
                SessionPageReason::Read { detail: reason }
            }
        },
    })
}
impl SessionPageStatus {
    pub fn from_read(
        result: Result<LoadedPage, PageUnavailable>,
        selection: &SessionPageSelection,
    ) -> Result<Self, &'static str> {
        let mut page = match result {
            Ok(page) => page,
            Err(reason) => return unavailable(reason),
        };
        let mut wire = page
            .records
            .iter()
            .map(|r| SourceRecord::try_from(&r.record))
            .collect::<Result<Vec<_>, _>>()?;
        loop {
            if page.expired() {
                return unavailable(PageUnavailable::Read(
                    xt_ingest::native::session_source::ExcerptUnavailable::Deadline,
                ));
            }
            let start = page.records.first().map_or(0, |r| r.ordinal);
            let end = page.records.last().map_or(start, |r| r.ordinal + 1);
            let mut ranges: Vec<SessionSourceRange> = Vec::new();
            let mut source = None;
            for (index, record) in page.records.iter().enumerate() {
                if source == Some(record.source) {
                    ranges.last_mut().unwrap().end = index as u32 + 1;
                } else {
                    ranges.push(SessionSourceRange {
                        start: index as u32,
                        end: index as u32 + 1,
                    });
                    source = Some(record.source);
                }
            }
            let target = page.target.map(|ordinal| SessionPageTarget {
                record: (ordinal - start) as u32,
                block: match selection {
                    SessionPageSelection::Anchor { block, .. } => *block,
                    _ => None,
                },
            });
            let status = Self::Loaded {
                generation: page.generation.into(),
                digest: page.digest.clone(),
                records: wire,
                record_ranges: ranges,
                start: start as u32,
                end: end as u32,
                total_records: page.total_records as u32,
                dropped_records: u32::try_from(page.dropped_records)
                    .map_err(|_| "dropped record count overflow")?,
                previous: (start > 0).then(|| page.cursor(true, start)),
                next: (end < page.total_records).then(|| page.cursor(false, end)),
                target,
            };
            if serde_json::to_writer(Counter(0), &status).is_ok() {
                return Ok(status);
            }
            let Self::Loaded { records, .. } = status else {
                unreachable!()
            };
            wire = records;
            if wire.len() <= 1 {
                return unavailable(PageUnavailable::RecordTooLarge);
            }
            // Remove only from an edge, preserving a contiguous interval and the anchor.
            let front = if let Some(target) = page.target {
                let left = target - start;
                let right = end - 1 - target;
                left > right
            } else {
                page.backwards
            };
            if front {
                page.records.remove(0);
                wire.remove(0);
            } else {
                page.records.pop();
                wire.pop();
            }
        }
    }
}
struct Counter(usize);
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MAX_PAGE_OUTPUT_BYTES {
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
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
    };
    use xt_ingest::native::{
        readers_cli::CancelToken,
        session_source::{
            ExcerptLimits, SessionSourceOutcome, SessionSourceRequest, SourceCeiling, SourceLimits,
            SourceUnavailable, load_session_page_with, load_session_source,
        },
    };
    const SESSION: &str = "synthetic-paged-conversation";
    struct Fixture {
        home: tempfile::TempDir,
        path: PathBuf,
    }
    impl Fixture {
        fn new(body: &str) -> Self {
            let home = tempfile::tempdir().unwrap();
            let path = home
                .path()
                .join(format!(".claude/projects/project/{SESSION}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
            Self { home, path }
        }
        fn request(&self) -> SessionSourceRequest<'_> {
            SessionSourceRequest {
                home: self.home.path(),
                host: xt_store::Host::Claude,
                native_session_id: SESSION,
                indexed: &[],
                cancel: None,
                limits: SourceLimits::default(),
            }
        }
        fn page(&self, selection: &SessionPageSelection) -> SessionPageStatus {
            SessionPageStatus::from_read(
                load_session_page_with(
                    &self.request(),
                    &selection.native(),
                    ExcerptLimits::default(),
                ),
                selection,
            )
            .unwrap()
        }
        fn sidechain(&self, body: &str) -> PathBuf {
            let path = self
                .path
                .parent()
                .unwrap()
                .join(SESSION)
                .join("subagents/agent-a.jsonl");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
            path
        }
    }
    fn line(id: &str, text: &str) -> String {
        format!(
            "{}\n",
            json!({"type":"user","uuid":id,"sessionId":SESSION,"timestamp":"2026-10-25T12:00:00Z","message":{"role":"user","content":[{"type":"text","text":text}]}})
        )
    }
    fn anchor(id: &str) -> SessionPageSelection {
        SessionPageSelection::Anchor {
            record_uuid: id.into(),
            block: None,
            expected: SessionPageTargetKind::Message,
            generation: None,
        }
    }
    fn loaded(
        status: &SessionPageStatus,
    ) -> (&[SourceRecord], u32, u32, Option<&str>, Option<&str>) {
        match status {
            SessionPageStatus::Loaded {
                records,
                start,
                end,
                previous,
                next,
                ..
            } => (records, *start, *end, previous.as_deref(), next.as_deref()),
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn escaped_output_adapts_boundaries_and_every_record_remains_reachable_both_ways() {
        let body = (0..11)
            .map(|i| line(&format!("record-{i}"), &"\0".repeat(100_000)))
            .collect::<String>();
        let f = Fixture::new(&body);
        let mut selection = SessionPageSelection::First;
        let mut seen = Vec::new();
        let last;
        loop {
            let status = f.page(&selection);
            assert!(serde_json::to_vec(&status).unwrap().len() <= MAX_PAGE_OUTPUT_BYTES);
            let (records, start, end, _, next) = loaded(&status);
            assert_eq!(start as usize, seen.len());
            assert!(end > start);
            for record in records {
                seen.push(record.id.clone());
                assert_eq!(record.blocks.len(), 1);
            }
            if let Some(next) = next {
                selection = SessionPageSelection::Cursor {
                    cursor: next.into(),
                };
            } else {
                last = status;
                break;
            }
        }
        assert_eq!(
            seen,
            (0..11).map(|i| format!("record-{i}")).collect::<Vec<_>>()
        );
        let mut status = last;
        let mut boundary = 11;
        loop {
            let (records, start, end, previous, _) = loaded(&status);
            assert_eq!(end, boundary);
            assert_eq!(records.len(), (end - start) as usize);
            boundary = start;
            let Some(previous) = previous else { break };
            status = f.page(&SessionPageSelection::Cursor {
                cursor: previous.into(),
            });
        }
        assert_eq!(boundary, 0);
        let status = f.page(&anchor("record-5"));
        let (records, start, _, _, _) = loaded(&status);
        assert!(records.iter().any(|r| r.id == "record-5"));
        let SessionPageStatus::Loaded {
            target: Some(target),
            ..
        } = status
        else {
            panic!()
        };
        assert_eq!(start + target.record, 5);
    }
    #[test]
    fn display_and_diagnosis_context_keep_their_own_predecessor_counts() {
        let f = Fixture::new(
            &(0..40)
                .map(|i| line(&format!("r{i}"), "synthetic"))
                .collect::<String>(),
        );
        let selection = anchor("r20");
        let display = f.page(&selection);
        let (_, start, _, _, _) = loaded(&display);
        assert_eq!(start, 19);
        let context = xt_ingest::native::session_source::load_session_context_with_reader(
            &f.request(),
            &selection.native(),
            None,
            ExcerptLimits::default(),
        )
        .unwrap();
        assert_eq!(context.records.first().unwrap().ordinal, 8);
        assert_eq!(context.target, Some(20));
        assert_eq!(
            context.records[12].record.canonical.uuid.as_deref(),
            Some("r20")
        );
    }
    #[test]
    fn oversized_required_message_refuses_without_a_skipping_cursor() {
        for text in ["x".repeat(4 * 1024 * 1024), "\0".repeat(200_000)] {
            let f = Fixture::new(&line("big", &text));
            for selection in [SessionPageSelection::First, anchor("big")] {
                assert!(matches!(
                    f.page(&selection),
                    SessionPageStatus::Unavailable {
                        reason: SessionPageReason::RecordTooLarge
                    }
                ));
            }
        }
    }
    #[test]
    fn source_generation_changes_refuse_old_cursor_and_bound_anchor() {
        for change in ["append", "replace", "rewrite", "truncate", "new", "missing"] {
            let f = Fixture::new(
                &(0..220)
                    .map(|i| line(&format!("r{i}"), "synthetic"))
                    .collect::<String>(),
            );
            let side = f.sidechain(&line("agent", "synthetic"));
            let first = f.page(&SessionPageSelection::First);
            let (_, _, _, _, next) = loaded(&first);
            let next = next.unwrap().to_owned();
            let SessionPageStatus::Loaded { digest, .. } = first else {
                panic!()
            };
            match change {
                "append" => {
                    let mut file = fs::OpenOptions::new().append(true).open(&f.path).unwrap();
                    file.write_all(line("new", "synthetic").as_bytes()).unwrap();
                }
                "replace" => {
                    let tmp = f.path.with_extension("replacement");
                    fs::write(&tmp, fs::read(&f.path).unwrap()).unwrap();
                    fs::rename(tmp, &f.path).unwrap();
                }
                "rewrite" => fs::write(
                    &f.path,
                    fs::read_to_string(&f.path)
                        .unwrap()
                        .replace("synthetic\"", "changedxx\""),
                )
                .unwrap(),
                "truncate" => {
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&f.path)
                        .unwrap()
                        .set_len(10)
                        .unwrap();
                }
                "new" => {
                    fs::write(
                        side.with_file_name("agent-b.jsonl"),
                        line("agentb", "synthetic"),
                    )
                    .unwrap();
                }
                "missing" => fs::remove_file(side).unwrap(),
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    f.page(&SessionPageSelection::Cursor { cursor: next }),
                    SessionPageStatus::Unavailable {
                        reason: SessionPageReason::Changed
                    }
                ),
                "{change}"
            );
            let mut bound = anchor("r1");
            if let SessionPageSelection::Anchor { generation, .. } = &mut bound {
                *generation = Some(digest);
            }
            assert!(
                matches!(
                    f.page(&bound),
                    SessionPageStatus::Unavailable {
                        reason: SessionPageReason::Changed
                    }
                ),
                "{change}"
            );
        }
    }
    #[test]
    fn exact_anchor_requires_global_unique_id_and_expected_block_kind() {
        let f = Fixture::new(&line("target", "synthetic"));
        f.sidechain(&line("target", "duplicate"));
        assert!(matches!(
            f.page(&anchor("target")),
            SessionPageStatus::Unavailable {
                reason: SessionPageReason::Read {
                    detail: SessionExcerptReason::TargetAmbiguous
                }
            }
        ));
        let f = Fixture::new(&line("target", "synthetic"));
        let selection = SessionPageSelection::Anchor {
            record_uuid: "target".into(),
            block: Some(0),
            expected: SessionPageTargetKind::ToolCall,
            generation: None,
        };
        assert!(matches!(
            f.page(&selection),
            SessionPageStatus::Unavailable {
                reason: SessionPageReason::Read {
                    detail: SessionExcerptReason::BlockMissing
                }
            }
        ));
        assert!(matches!(
            f.page(&anchor("missing")),
            SessionPageStatus::Unavailable {
                reason: SessionPageReason::Read {
                    detail: SessionExcerptReason::TargetMissing
                }
            }
        ));
        f.sidechain("not json\n");
        assert!(matches!(
            f.page(&anchor("target")),
            SessionPageStatus::Unavailable {
                reason: SessionPageReason::Read {
                    detail: SessionExcerptReason::Incomplete
                }
            }
        ));
    }
    #[test]
    fn cursor_input_and_scan_cancellation_limits_refuse_without_text() {
        let f = Fixture::new(&line("target", "synthetic"));
        for cursor in [
            "x".repeat(129),
            "v2:after:1:bad".into(),
            format!("v1:after:999999:{}", "0".repeat(64)),
        ] {
            assert!(matches!(
                f.page(&SessionPageSelection::Cursor { cursor }),
                SessionPageStatus::Unavailable {
                    reason: SessionPageReason::InvalidCursor
                }
            ));
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        let mut request = f.request();
        request.cancel = Some(&cancel);
        assert!(matches!(
            load_session_page_with(&request, &PageSelection::First, ExcerptLimits::default()),
            Err(PageUnavailable::Read(
                xt_ingest::native::session_source::ExcerptUnavailable::Source(
                    SourceUnavailable::Cancelled
                )
            ))
        ));
        let result = load_session_page_with(
            &f.request(),
            &PageSelection::First,
            ExcerptLimits {
                scan_bytes: 1,
                ..ExcerptLimits::default()
            },
        );
        assert!(result.is_err());
        let result = load_session_page_with(
            &f.request(),
            &PageSelection::First,
            ExcerptLimits {
                deadline: std::time::Duration::ZERO,
                ..ExcerptLimits::default()
            },
        );
        assert!(result.is_err());
    }
    fn portable(status: &SessionPageStatus) -> serde_json::Value {
        let mut value = serde_json::to_value(status).unwrap();
        let zero = "0".repeat(64);
        value["digest"] = json!(zero);
        for key in ["previous", "next"] {
            if let Some(cursor) = value[key].as_str() {
                let prefix = cursor.rsplit_once(':').unwrap().0.to_owned();
                value[key] = json!(format!("{prefix}:{zero}"));
            }
        }
        value
    }
    #[test]
    fn large_native_pages_cover_every_record_and_match_the_browser_fixture() {
        let f = Fixture::new("");
        let side = f.sidechain("");
        let padding = "A".repeat(112_000);
        let mut primary = fs::File::create(&f.path).unwrap();
        let mut agent = fs::File::create(&side).unwrap();
        for i in 0..630 {
            let id = if i == 315 {
                "w-h3".into()
            } else {
                format!("synthetic-{i}")
            };
            let record = json!({"type":"user","uuid":id,"sessionId":SESSION,"timestamp":"2026-10-25T12:00:00Z","message":{"role":"user","content":[{"type":"text","text":format!("Synthetic conversation message {i}. Every message remains complete.")},{"type":"image","source":{"type":"base64","data":padding}}]}});
            writeln!(if i < 300 { &mut primary } else { &mut agent }, "{record}").unwrap();
        }
        drop(primary);
        drop(agent);
        assert!(
            fs::metadata(&f.path).unwrap().len() + fs::metadata(&side).unwrap().len()
                > 66 * 1024 * 1024
        );
        assert!(matches!(
            load_session_source(&f.request()),
            SessionSourceOutcome::Unavailable(SourceUnavailable::TooLarge {
                limit: SourceCeiling::Bytes,
                ..
            })
        ));
        let mut selection = SessionPageSelection::First;
        let mut seen = 0;
        let mut pages = Vec::new();
        let mut crossed = false;
        loop {
            let status = f.page(&selection);
            let (records, start, end, _, next) = loaded(&status);
            assert_eq!(start as usize, seen);
            assert!(end > start);
            assert_eq!(end - start, records.len() as u32);
            for (n, record) in records.iter().enumerate() {
                let i = seen + n;
                assert_eq!(
                    record.id,
                    if i == 315 {
                        "w-h3".into()
                    } else {
                        format!("synthetic-{i}")
                    }
                );
                assert_eq!(record.blocks.len(), 2);
            }
            if let SessionPageStatus::Loaded { record_ranges, .. } = &status {
                crossed |= record_ranges.len() > 1;
            }
            pages.push(portable(&status));
            seen = end as usize;
            let Some(next) = next else { break };
            selection = SessionPageSelection::Cursor {
                cursor: next.into(),
            };
        }
        assert_eq!(seen, 630);
        assert!(crossed);
        let mut anchors = Vec::new();
        let mut cursor_pages = Vec::new();
        for (id, ordinal) in [("synthetic-0", 0), ("w-h3", 315), ("synthetic-629", 629)] {
            let status = f.page(&anchor(id));
            let SessionPageStatus::Loaded {
                start,
                target: Some(ref target),
                ..
            } = status
            else {
                panic!()
            };
            assert_eq!(start + target.record, ordinal);
            if ordinal == 315 {
                let (_, _, _, previous, next) = loaded(&status);
                for cursor in [previous, next].into_iter().flatten() {
                    let page = f.page(&SessionPageSelection::Cursor {
                        cursor: cursor.into(),
                    });
                    let (_, _, _, prev, next) = loaded(&page);
                    let return_cursor = if cursor.contains(":before:") {
                        next
                    } else {
                        prev
                    }
                    .unwrap();
                    let returned = f.page(&SessionPageSelection::Cursor {
                        cursor: return_cursor.into(),
                    });
                    let normalize = |cursor: &str| {
                        format!("{}:{}", cursor.rsplit_once(':').unwrap().0, "0".repeat(64))
                    };
                    cursor_pages.push(
                        json!({"cursor":normalize(return_cursor),"status":portable(&returned)}),
                    );
                    cursor_pages.push(json!({"cursor":normalize(cursor),"status":portable(&page)}));
                }
            }
            anchors.push(portable(&status));
        }
        let proof = json!({"pages":pages,"anchors":anchors,"cursor_pages":cursor_pages,"source_bytes_over":66*1024*1024});
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ui/e2e/warning-full-session.synthetic.json");
        if std::env::var_os("XTRACE_WRITE_PAGE_FIXTURE").is_some() {
            fs::write(&path, serde_json::to_string_pretty(&proof).unwrap() + "\n").unwrap();
        }
        let expected: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            proof, expected,
            "browser pages must match native output; only the source identity digest is replaced with zeros"
        );
    }
    #[test]
    fn native_high_index_tool_result_anchor_survives_adaptive_encoded_output() {
        let escaped = "\0".repeat(100_000);
        let mut content = (0..128)
            .map(|index| json!({"type":"text","text":format!("synthetic block {index}")}))
            .collect::<Vec<_>>();
        content.push(json!({"type":"tool_result","tool_use_id":"synthetic-call","content":escaped,"is_error":true}));
        let target = format!(
            "{}\n",
            json!({"type":"user","uuid":"target","sessionId":SESSION,"message":{"role":"user","content":content}})
        );
        let f = Fixture::new(
            &[
                line("before-first", &escaped),
                line("before-near", &escaped),
                target,
                line("after", &escaped),
            ]
            .concat(),
        );
        let selection = SessionPageSelection::Anchor {
            record_uuid: "target".into(),
            block: Some(128),
            expected: SessionPageTargetKind::ToolBlock,
            generation: None,
        };
        let status = f.page(&selection);
        assert!(serde_json::to_vec(&status).unwrap().len() <= MAX_PAGE_OUTPUT_BYTES);
        let SessionPageStatus::Loaded {
            records,
            start,
            end,
            target: Some(proof),
            previous: Some(_),
            next: Some(_),
            ..
        } = status
        else {
            panic!("{status:?}");
        };
        assert_eq!(
            (start, end),
            (2, 3),
            "oversized neighboring messages must be trimmed, never the anchor"
        );
        assert_eq!(proof.record, 0);
        assert_eq!(proof.block, Some(128));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].blocks.len(), 129);
        assert_eq!(records[0].id, "target");
        let crate::transcript_dto::SourceBlock::ToolResult {
            index,
            call_id,
            failed,
            parts,
        } = &records[0].blocks[128]
        else {
            panic!("original result block must remain a result");
        };
        assert_eq!(*index, 128);
        assert_eq!(call_id.as_deref(), Some("synthetic-call"));
        assert!(*failed);
        assert_eq!(
            parts,
            &vec![crate::transcript_dto::SourceResultPart::Text { text: escaped }]
        );
    }
}
