//! Embedded migrations are registered in lexical filename order, one transaction
//! per version. Extend this chain; never modify an applied migration or reuse a
//! version. Unknown or non-prefix histories fail.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_canonical.sql")),
    (2, include_str!("../migrations/0002_ingest.sql")),
    (3, include_str!("../migrations/0003_native_checkpoints.sql")),
    (
        4,
        include_str!("../migrations/0004_native_record_copies.sql"),
    ),
    (
        5,
        include_str!("../migrations/0005_human_classification_replay.sql"),
    ),
    (
        6,
        include_str!("../migrations/0006_structural_tool_kinds.sql"),
    ),
];

impl Store {
    /// Highest applied migration after opening this store.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.connection.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
            )",
        )?;
        for (index, &(version, sql)) in MIGRATIONS.iter().enumerate() {
            // Serialize the history read and migration with competing openers.
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let applied = transaction
                .prepare("SELECT version FROM schema_version ORDER BY version")?
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if applied.len() > MIGRATIONS.len()
                || !applied.iter().zip(MIGRATIONS).all(|(a, (b, _))| a == b)
            {
                return Err(Error::IncompatibleSchema);
            }
            if applied.len() <= index {
                transaction.execute_batch(sql)?;
                transaction.execute(
                    "INSERT INTO schema_version(version, applied_at)
                     VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    [version],
                )?;
            }
            transaction.commit()?;
        }
        // Maintain only the named views and this rebuildable, nonunique lookup
        // index after immutable history. Logical schema/data changes still belong
        // to numbered migrations; this access path preserves every stored row.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let views = [
            ("v_records", include_str!("../views/records.sql")),
            (
                "v_usage_records",
                include_str!("../views/usage_records.sql"),
            ),
            (
                "v_response_usage",
                include_str!("../views/response_usage.sql"),
            ),
            (
                "v_session_events",
                include_str!("../views/session_events.sql"),
            ),
        ];
        let mut changed = false;
        for (name, sql) in views {
            let current: Option<String> = transaction
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='view' AND name=?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()?;
            changed |= current.as_deref() != Some(sql.trim().trim_end_matches(';'));
        }
        if changed {
            transaction.execute_batch(
                "DROP VIEW IF EXISTS v_response_usage; DROP VIEW IF EXISTS v_usage_records; DROP VIEW IF EXISTS v_session_events; DROP VIEW IF EXISTS v_records;",
            )?;
            for (_, sql) in views {
                transaction.execute_batch(sql)?;
            }
        }
        const INDEX_NAME: &str = "tool_uses_structural_event_id";
        const INDEX_SQL: &str = "CREATE INDEX tool_uses_structural_event_id ON tool_uses(source_event_id) WHERE uuid IS NULL";
        let index: Option<(String, String, Option<String>)> = transaction
            .query_row(
                "SELECT type,tbl_name,sql FROM sqlite_schema WHERE name=?1",
                [INDEX_NAME],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        match index {
            None => transaction.execute_batch(INDEX_SQL)?,
            Some((kind, table, definition)) if kind == "index" && table == "tool_uses" => {
                if definition.as_deref() != Some(INDEX_SQL) {
                    transaction.execute_batch("DROP INDEX tool_uses_structural_event_id")?;
                    transaction.execute_batch(INDEX_SQL)?;
                }
            }
            Some(_) => return Err(Error::IncompatibleSchema),
        }
        transaction.commit()?;
        Ok(())
    }
}
