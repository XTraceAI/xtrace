//! Conditional publication of neutral M-21 answers computed in one snapshot.
use crate::{Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

/// One answer and the exact source revision used to decide it.
#[derive(Clone, Debug)]
pub struct SavedSessionWarning {
    pub session_id: String,
    pub revision: i64,
    pub rule_version: i64,
    pub payload_json: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarningPublication {
    pub published: usize,
    pub rejected: usize,
}

impl Store {
    /// Stable sessions publish even if another session changed while computing.
    /// Dismissals are independent and never replaced here.
    pub fn publish_session_warnings(
        &mut self,
        rows: &[SavedSessionWarning],
    ) -> Result<WarningPublication> {
        self.publish_session_warnings_cancellable(rows, &|| false)
    }

    pub fn publish_session_warnings_cancellable(
        &mut self,
        rows: &[SavedSessionWarning],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<WarningPublication> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cancelled() {
            return Ok(WarningPublication::default());
        }
        let mut outcome = WarningPublication::default();
        for row in rows {
            if cancelled() {
                return Ok(WarningPublication::default());
            }
            let current:Option<i64>=transaction.query_row(
                "SELECT i.revision FROM session_warning_inputs i JOIN sessions s USING(session_id) WHERE session_id=?1 AND s.kind='user'",
                [&row.session_id],|value|value.get(0),
            ).optional()?;
            if current != Some(row.revision) {
                outcome.rejected += 1;
                continue;
            }
            transaction.execute(
                "INSERT INTO saved_session_warnings(session_id,revision,rule_version,payload_json) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision,rule_version=excluded.rule_version,payload_json=excluded.payload_json",
                params![row.session_id,row.revision,row.rule_version,row.payload_json],
            )?;
            outcome.published += 1;
        }
        transaction.execute("DELETE FROM saved_session_warnings WHERE session_id NOT IN (SELECT session_id FROM sessions WHERE kind='user')",[])?;
        if cancelled() {
            return Ok(WarningPublication::default());
        }
        transaction.commit()?;
        Ok(outcome)
    }
}
