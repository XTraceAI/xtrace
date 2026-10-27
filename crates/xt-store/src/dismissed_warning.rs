//! Session warnings the user dismissed (`dismissed_warnings`, migration 28):
//! one row per dismissed warning, keyed by its session and the warning's own
//! identity (`kind:anchor`, `xt_metrics::SessionWarning::id`), with when it
//! was dismissed and how big it was then (`size`).
//!
//! Saved identifiers, a count and a time only, never conversation content,
//! so the content purge leaves them. Dismissing a warning already dismissed
//! at the same size keeps its first time ([`Store::dismiss_warning`]);
//! undoing deletes the row ([`Store::restore_warning`]). Which warnings a screen shows is decided by
//! the reader (the app's warnings answer, which reads this table in its own
//! snapshot through `xt_metrics::MetricsDb::dismissed_warnings`), not here.

use crate::{Result, Store};
use rusqlite::params;

/// Longest warning identity a dismissal names.
pub const MAX_WARNING_ID_CHARS: usize = 600;

impl Store {
    /// Dismiss one warning at `size`, how big it is now (`None` for a
    /// correction or an interruption, which are one event each). True when
    /// something was stored: a new dismissal, or a dismissal of a warning
    /// that had grown past the size it was dismissed at, which now keeps the
    /// new size and time. Dismissing again at the same size changes nothing
    /// and keeps the first time. A value the table refuses (an empty or
    /// overlong identifier, an unknown kind, a size on a single-event kind
    /// or none on one that grows) is an error and writes nothing.
    pub fn dismiss_warning(
        &mut self,
        session_id: &str,
        warning_id: &str,
        size: Option<u64>,
        dismissed_at_ms: i64,
    ) -> Result<bool> {
        let size = size
            .map(i64::try_from)
            .transpose()
            .map_err(|_| crate::Error::InvalidInput("warning size out of range"))?;
        let changed = self.connection.execute(
            "INSERT INTO dismissed_warnings(session_id,warning_id,size,dismissed_at)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(session_id,warning_id) DO UPDATE
             SET size=excluded.size, dismissed_at=excluded.dismissed_at
             WHERE excluded.size > dismissed_warnings.size",
            params![session_id, warning_id, size, dismissed_at_ms],
        )?;
        Ok(changed == 1)
    }

    /// Undo a dismissal. True when one was stored.
    pub fn restore_warning(&mut self, session_id: &str, warning_id: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM dismissed_warnings WHERE session_id=?1 AND warning_id=?2",
            params![session_id, warning_id],
        )? == 1)
    }
}
