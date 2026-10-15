//! Structural ingestion facts. Observation/discovery writes never create capture
//! receipts. Receipt digests and masks are supplied by the future ingest writer;
//! this module stores their exact values and seals each submitted coverage set.

use crate::{Error, Host, Result, SessionSource, Store, model::text_enum, timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSourceObservation {
    pub session_id: String,
    pub source: SessionSource,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordSourceObservation {
    pub uuid: String,
    pub source: SessionSource,
    pub field_presence: i64,
    pub conflict_flags: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReceipt {
    pub receipt_id: String,
    pub session_id: String,
    pub surface: Option<String>,
    /// Explicit UTC milliseconds supplied by the caller, not a fixture wall clock.
    pub received_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordCoverage {
    pub record_uuid: String,
    /// Nonnegative SQLite integer; named bits are owned by the digest schema.
    pub metric_field_mask: i64,
    /// Canonical measurement digest supplied by ingestion, never recomputed from
    /// a richer stored record. Exactly 64 lowercase SHA-256 hexadecimal digits.
    pub measurement_revision: String,
    pub digest_schema_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredSession {
    pub host: Host,
    pub native_session_id: String,
    /// May be known before any canonical session has been imported.
    pub conversation_id: Option<String>,
    pub surface: Option<String>,
    pub started_at_ms: Option<i64>,
    pub last_observed_at: i64,
    pub discovery_complete: bool,
}

text_enum!(ToolKind {
    Builtin => "builtin", Mcp => "mcp", Skill => "skill", Hook => "hook",
    Command => "command", Subagent => "subagent"
});

/// A classified structural event that needs no fabricated user/assistant row.
/// This shape deliberately has no command, transcript, input or output payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolEvent {
    pub session_id: String,
    pub source: SessionSource,
    pub source_event_id: String,
    /// Native RFC3339 timestamp, preserved verbatim. Absence stays unknown;
    /// session start and receipt time are not substitutes for an event time.
    pub timestamp: Option<String>,
    pub name: String,
    pub kind: ToolKind,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub skill: Option<String>,
}

/// One unchanged stored observation and its current canonical owner, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolEventObservation {
    pub id: i64,
    pub event: ToolEvent,
    pub canonical_owner: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolEventUncertainty {
    MissingIdentity,
    ConflictingOwnership,
    ConflictingFacts,
}

/// A logical event only when every saved observation agrees. The representative
/// is a stable handle, not an authoritative choice among conflicting facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolEventResolution {
    Resolved(ToolEventObservation),
    Unresolved {
        reason: ToolEventUncertainty,
        observations: Vec<ToolEventObservation>,
    },
}

/// Shared read requirements, also available for a consumer's connection-time
/// schema probe. Bind NULL for all rows or an exact raw event ID for one group.
/// No window/session filter may hide a disagreeing member before resolution.
pub const TOOL_EVENT_OBSERVATIONS_QUERY: &str =
    "SELECT t.id,t.session_id,t.source,t.source_event_id,t.name,t.kind,t.server,t.tool,t.skill,t.event_ts,r.session_id
     FROM tool_uses t LEFT JOIN records r ON r.uuid=t.source_event_id
     WHERE t.uuid IS NULL AND (?1 IS NULL OR t.source_event_id=?1) ORDER BY t.id";

/// Resolve structural events on the caller's existing connection/snapshot.
/// This reads only; it opens no Store, runs no migration and changes no row.
/// Unresolved entries preserve candidate owners/times for downstream completeness
/// reporting and must not be interpreted as measured calls.
pub fn resolved_tool_events(connection: &Connection) -> Result<Vec<ToolEventResolution>> {
    let mut groups = std::collections::BTreeMap::<String, Vec<ToolEventObservation>>::new();
    let mut result = Vec::new();
    for observation in tool_event_observations(connection, None)? {
        if observation.event.source_event_id.trim().is_empty() {
            // Equal blank spellings do not establish a shared native identity.
            result.push(ToolEventResolution::Unresolved {
                reason: ToolEventUncertainty::MissingIdentity,
                observations: vec![observation],
            });
        } else {
            groups
                .entry(observation.event.source_event_id.clone())
                .or_default()
                .push(observation);
        }
    }
    for observations in groups.into_values() {
        result.push(resolve_tool_event_group(observations)?);
    }
    Ok(result)
}

fn tool_event_observations(
    connection: &Connection,
    id: Option<&str>,
) -> Result<Vec<ToolEventObservation>> {
    Ok(connection
        .prepare(TOOL_EVENT_OBSERVATIONS_QUERY)?
        .query_map([id], |row| {
            Ok(ToolEventObservation {
                id: row.get(0)?,
                event: tool_event_from_row(row, 1)?,
                canonical_owner: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn resolve_tool_event_group(
    observations: Vec<ToolEventObservation>,
) -> Result<ToolEventResolution> {
    let first = observations
        .first()
        .expect("a stored event group is nonempty");
    if observations.iter().any(|row| {
        row.event.session_id != first.event.session_id
            || row
                .canonical_owner
                .as_ref()
                .is_some_and(|owner| owner != &row.event.session_id)
    }) {
        return Ok(ToolEventResolution::Unresolved {
            reason: ToolEventUncertainty::ConflictingOwnership,
            observations,
        });
    }
    for row in &observations {
        if !same_tool_event(&first.event, &row.event)? {
            return Ok(ToolEventResolution::Unresolved {
                reason: ToolEventUncertainty::ConflictingFacts,
                observations,
            });
        }
    }
    // The shared query orders by ID; select a handle only after all facts agree.
    Ok(ToolEventResolution::Resolved(first.clone()))
}

fn same_tool_event(saved: &ToolEvent, incoming: &ToolEvent) -> Result<bool> {
    let saved_instant = saved
        .timestamp
        .as_deref()
        .map(timestamp::parse)
        .transpose()?
        .map(|(key, _)| key);
    let incoming_instant = incoming
        .timestamp
        .as_deref()
        .map(timestamp::parse)
        .transpose()?
        .map(|(key, _)| key);
    Ok((
        &saved.session_id,
        &saved.source_event_id,
        &saved.name,
        saved.kind,
        &saved.server,
        &saved.tool,
        &saved.skill,
        saved_instant,
    ) == (
        &incoming.session_id,
        &incoming.source_event_id,
        &incoming.name,
        incoming.kind,
        &incoming.server,
        &incoming.tool,
        &incoming.skill,
        incoming_instant,
    ))
}

impl Store {
    /// Monotonic observation interval; this is not a delivery receipt.
    pub fn observe_session_source(&mut self, observation: &SessionSourceObservation) -> Result<()> {
        observe_session_source(&self.connection, observation)
    }

    pub fn session_sources(&self, session_id: &str) -> Result<Vec<SessionSourceObservation>> {
        Ok(self.connection.prepare(
            "SELECT session_id,source,first_seen_at,last_seen_at FROM session_sources WHERE session_id=?1 ORDER BY source"
        )?.query_map([session_id], |row| Ok(SessionSourceObservation {
            session_id: row.get(0)?, source: row.get(1)?, first_seen_at: row.get(2)?, last_seen_at: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Accumulate supplied presence/conflict bits without reading stored content
    /// or claiming that these observations were covered by a plugin receipt.
    pub fn observe_record_source(&mut self, observation: &RecordSourceObservation) -> Result<()> {
        observe_record_source(&self.connection, observation)
    }

    pub fn record_sources(&self, uuid: &str) -> Result<Vec<RecordSourceObservation>> {
        Ok(self.connection.prepare(
            "SELECT uuid,source,field_presence,conflict_flags FROM record_sources WHERE uuid=?1 ORDER BY source"
        )?.query_map([uuid], |row| Ok(RecordSourceObservation {
            uuid: row.get(0)?, source: row.get(1)?, field_presence: row.get(2)?, conflict_flags: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Insert and seal one nonempty submitted set atomically. Duplicate receipt
    /// IDs, duplicate child UUIDs and cross-session coverage fail without leaving
    /// partial facts. Retry arbitration and digest generation belong to ingestion.
    pub fn insert_capture_receipt(
        &mut self,
        receipt: &CaptureReceipt,
        coverage: &[RecordCoverage],
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_receipt(&transaction, receipt, coverage)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn capture_receipts(&self, session_id: &str) -> Result<Vec<CaptureReceipt>> {
        Ok(self
            .connection
            .prepare(
                "SELECT receipt_id,session_id,surface,received_at FROM capture_receipts
             WHERE session_id=?1 AND coverage_sealed=1 ORDER BY received_at,receipt_id",
            )?
            .query_map([session_id], |row| {
                Ok(CaptureReceipt {
                    receipt_id: row.get(0)?,
                    session_id: row.get(1)?,
                    surface: row.get(2)?,
                    received_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn capture_coverage(&self, receipt_id: &str) -> Result<Vec<RecordCoverage>> {
        coverage(&self.connection, receipt_id)
    }

    /// Discovery exists independently of canonical ingestion. Known identities
    /// fill once; conflicting non-null values fail. A newer probe can change
    /// completeness, while a stale result cannot replace the current status.
    pub fn observe_discovered_session(&mut self, discovery: &DiscoveredSession) -> Result<()> {
        observe_discovered_session(&self.connection, discovery)
    }

    pub fn discovered_sessions(&self, host: Host) -> Result<Vec<DiscoveredSession>> {
        Ok(self.connection.prepare(
            "SELECT host,native_session_id,conversation_id,surface,started_at_ms,last_observed_at,discovery_complete
             FROM discovered_sessions WHERE host=?1 ORDER BY native_session_id"
        )?.query_map([host], |row| Ok(DiscoveredSession {
            host: row.get(0)?, native_session_id: row.get(1)?, conversation_id: row.get(2)?, surface: row.get(3)?,
            started_at_ms: row.get(4)?, last_observed_at: row.get(5)?, discovery_complete: row.get(6)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Store only the classified event identity and names, with no record FK
    /// when the native event is not a canonical user/assistant record.
    /// Idempotent for the event's own stable identity, and bound to the session
    /// that owns that identity rather than the supplied one. All saved
    /// observations must agree; unresolved historical groups fail explicitly.
    pub fn insert_tool_event(&mut self, event: &ToolEvent) -> Result<i64> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = insert_tool_event(&transaction, event, true)?
            .expect("the public event API permits new event insertion");
        transaction.commit()?;
        Ok(id)
    }

    /// Raw historical observations, including duplicates. For logical events
    /// and explicit uncertainty, use the resolved reader instead of row counts.
    pub fn tool_events(&self, session_id: &str) -> Result<Vec<ToolEvent>> {
        Ok(self.connection.prepare(
            "SELECT session_id,source,source_event_id,name,kind,server,tool,skill,event_ts FROM tool_uses
             WHERE session_id=?1 AND uuid IS NULL ORDER BY source,source_event_id"
        )?
        .query_map([session_id], |row| tool_event_from_row(row, 0))?
        .collect::<rusqlite::Result<_>>()?)
    }

    pub fn resolved_tool_events(&self) -> Result<Vec<ToolEventResolution>> {
        resolved_tool_events(&self.connection)
    }
}

/// Match a complete stored group under the caller's immediate transaction.
/// Canonical ownership takes priority for new identities; saved groups must
/// already agree with it. Matching copies bind to the group's unanimous owner.
/// Acquisition permission never bypasses existing-group arbitration.
pub(crate) fn insert_tool_event(
    connection: &Connection,
    event: &ToolEvent,
    acquire: bool,
) -> Result<Option<i64>> {
    if event.name.trim().is_empty() {
        return Err(Error::InvalidInput("tool event name must be nonempty"));
    }
    // An arrival-ordered identity would make retries look like new events.
    if event.source_event_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "structural tool event requires a stable source event ID",
        ));
    }
    if let Some(value) = &event.timestamp {
        timestamp::parse(value)?;
    }
    let observations = tool_event_observations(connection, Some(&event.source_event_id))?;
    if !observations.is_empty() {
        let ToolEventResolution::Resolved(saved) = resolve_tool_event_group(observations)? else {
            return Err(Error::InvalidInput(
                "structural tool event identity has unresolved saved observations",
            ));
        };
        let owned = ToolEvent {
            session_id: saved.event.session_id.clone(),
            ..event.clone()
        };
        if !same_tool_event(&saved.event, &owned)? {
            return Err(Error::InvalidInput(
                "structural tool event identity replays with conflicting facts",
            ));
        }
        return Ok(Some(saved.id));
    }
    // A contradicted command occurrence can establish no new event. Existing
    // identities above still compare all facts and fail on a conflicting replay.
    if !acquire {
        return Ok(None);
    }
    let owner = connection
        .query_row(
            "SELECT session_id FROM records WHERE uuid=?1",
            [&event.source_event_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| event.session_id.clone());
    let event = &ToolEvent {
        session_id: owner,
        ..event.clone()
    };
    connection.execute(
        "INSERT INTO tool_uses(session_id,source,source_event_id,name,kind,server,tool,skill,event_ts)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event.session_id,
            event.source,
            event.source_event_id,
            event.name,
            event.kind,
            event.server,
            event.tool,
            event.skill,
            event.timestamp
        ],
    )?;
    Ok(Some(connection.last_insert_rowid()))
}

fn tool_event_from_row(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<ToolEvent> {
    Ok(ToolEvent {
        session_id: row.get(start)?,
        source: row.get(start + 1)?,
        source_event_id: row.get(start + 2)?,
        name: row.get(start + 3)?,
        kind: row.get(start + 4)?,
        server: row.get(start + 5)?,
        tool: row.get(start + 6)?,
        skill: row.get(start + 7)?,
        timestamp: row.get(start + 8)?,
    })
}

pub(crate) fn observe_session_source(
    connection: &Connection,
    observation: &SessionSourceObservation,
) -> Result<()> {
    connection.execute(
        "INSERT INTO session_sources(session_id,source,first_seen_at,last_seen_at) VALUES (?1,?2,?3,?4)
         ON CONFLICT(session_id,source) DO UPDATE SET
             first_seen_at=min(first_seen_at,excluded.first_seen_at),
             last_seen_at=max(last_seen_at,excluded.last_seen_at)",
        params![observation.session_id, observation.source, observation.first_seen_at, observation.last_seen_at],
    )?;
    Ok(())
}

pub(crate) fn observe_record_source(
    connection: &Connection,
    observation: &RecordSourceObservation,
) -> Result<()> {
    connection.execute(
        "INSERT INTO record_sources(uuid,source,field_presence,conflict_flags) VALUES (?1,?2,?3,?4)
         ON CONFLICT(uuid,source) DO UPDATE SET field_presence=field_presence | excluded.field_presence,
             conflict_flags=conflict_flags | excluded.conflict_flags",
        params![observation.uuid, observation.source, observation.field_presence, observation.conflict_flags],
    )?;
    Ok(())
}

pub(crate) fn insert_receipt(
    connection: &Connection,
    receipt: &CaptureReceipt,
    coverage: &[RecordCoverage],
) -> Result<()> {
    if coverage.is_empty() {
        return Err(Error::InvalidInput(
            "capture receipt requires submitted record coverage",
        ));
    }
    connection.execute(
        "INSERT INTO capture_receipts(receipt_id,session_id,surface,received_at) VALUES (?1,?2,?3,?4)",
        params![receipt.receipt_id, receipt.session_id, receipt.surface, receipt.received_at],
    )?;
    for item in coverage {
        connection.execute(
            "INSERT INTO capture_record_coverage(receipt_id,record_uuid,session_id,metric_field_mask,measurement_revision,digest_schema_version)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![receipt.receipt_id, item.record_uuid, receipt.session_id, item.metric_field_mask, item.measurement_revision, item.digest_schema_version],
        )?;
    }
    connection.execute(
        "UPDATE capture_receipts SET coverage_sealed=1 WHERE receipt_id=?1",
        [&receipt.receipt_id],
    )?;
    Ok(())
}

fn coverage(connection: &Connection, receipt_id: &str) -> Result<Vec<RecordCoverage>> {
    Ok(connection.prepare(
            "SELECT c.record_uuid,c.metric_field_mask,c.measurement_revision,c.digest_schema_version
             FROM capture_record_coverage c JOIN capture_receipts r ON r.receipt_id=c.receipt_id
             WHERE c.receipt_id=?1 AND r.coverage_sealed=1 ORDER BY c.record_uuid"
        )?.query_map([receipt_id], |row| Ok(RecordCoverage {
            record_uuid: row.get(0)?, metric_field_mask: row.get(1)?, measurement_revision: row.get(2)?, digest_schema_version: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
}

// Called only inside the owning immediate write transaction. An existing ID
// cannot race another writer between comparison and canonical/cursor commit.
pub(crate) fn insert_or_match_receipt(
    connection: &Connection,
    receipt: &CaptureReceipt,
    submitted: &[RecordCoverage],
    allow_exact: bool,
    ignore_new_empty: bool,
) -> Result<bool> {
    let saved = connection.query_row(
        "SELECT session_id,surface,received_at,coverage_sealed FROM capture_receipts WHERE receipt_id=?1",
        [&receipt.receipt_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,i64>(2)?,row.get::<_,bool>(3)?)),
    ).optional()?;
    if let Some((session, surface, received, sealed)) = saved {
        let mut expected = submitted.to_vec();
        expected.sort_by(|a, b| a.record_uuid.cmp(&b.record_uuid));
        if !allow_exact
            || !sealed
            || expected.is_empty()
            || session != receipt.session_id
            || surface != receipt.surface
            || received != receipt.received_at
            || coverage(connection, &receipt.receipt_id)? != expected
        {
            return Err(Error::InvalidInput(
                "receipt retry does not match immutable submitted facts",
            ));
        }
        return Ok(true);
    }
    if ignore_new_empty && submitted.is_empty() {
        return Ok(false);
    }
    insert_receipt(connection, receipt, submitted)?;
    Ok(true)
}

/// The discovered-session upsert shared by direct observation and the batch
/// transaction, so a batch can fill an identity atomically with its rows.
pub(crate) fn observe_discovered_session(
    connection: &Connection,
    discovery: &DiscoveredSession,
) -> Result<()> {
    let changed = connection.execute(
        "INSERT INTO discovered_sessions(host,native_session_id,conversation_id,surface,started_at_ms,last_observed_at,discovery_complete)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(host,native_session_id) DO UPDATE SET
             conversation_id=coalesce(conversation_id,excluded.conversation_id),
             surface=coalesce(surface,excluded.surface),started_at_ms=coalesce(started_at_ms,excluded.started_at_ms),
             discovery_complete=CASE WHEN excluded.last_observed_at >= last_observed_at THEN excluded.discovery_complete ELSE discovery_complete END,
             last_observed_at=max(last_observed_at,excluded.last_observed_at)
         WHERE (conversation_id IS NULL OR excluded.conversation_id IS NULL OR conversation_id=excluded.conversation_id)
             AND (surface IS NULL OR excluded.surface IS NULL OR surface=excluded.surface)
             AND (started_at_ms IS NULL OR excluded.started_at_ms IS NULL OR started_at_ms=excluded.started_at_ms)",
        params![discovery.host, discovery.native_session_id, discovery.conversation_id, discovery.surface, discovery.started_at_ms, discovery.last_observed_at, discovery.discovery_complete],
    )?;
    if changed == 0 {
        return Err(Error::InvalidInput(
            "conflicting discovered session identity",
        ));
    }
    Ok(())
}
