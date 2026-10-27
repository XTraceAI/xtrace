-- Session warnings (M-21) the user dismissed: one row per dismissed warning,
-- keyed by its session and the warning's own identity, `kind:anchor`
-- (`xt_metrics::SessionWarning::id`), which stays the same while the session
-- grows. `size` is how big the warning was when dismissed (how many times a
-- repeated prompt was sent, a repeated call failed, or how many repeated
-- calls a stretch had), and NULL for a correction or an interruption, which
-- are one event each: a warning that has grown past it shows again.
-- `dismissed_at` is when it was dismissed, UTC milliseconds. Metadata only:
-- saved identifiers, a count and a time, never what the session said. Undoing a
-- dismissal deletes its row. A row whose warning is no longer found (its
-- session was re-read under a changed rule, say) hides nothing and is kept.
-- Additive only: no existing table changes.
CREATE TABLE dismissed_warnings (
    session_id TEXT NOT NULL CHECK(
        typeof(session_id) = 'text' AND length(session_id) BETWEEN 1 AND 256
        AND trim(session_id) = session_id
    ),
    warning_id TEXT NOT NULL CHECK(
        typeof(warning_id) = 'text' AND length(warning_id) BETWEEN 3 AND 600
        AND (warning_id GLOB 'correction:?*' OR warning_id GLOB 'interruption:?*'
            OR warning_id GLOB 'repeated_calls:?*' OR warning_id GLOB 'repeated_prompt:?*'
            OR warning_id GLOB 'repeated_failure:?*')
    ),
    size INTEGER CHECK(
        (size IS NULL) = (warning_id GLOB 'correction:?*' OR warning_id GLOB 'interruption:?*')
        AND (size IS NULL OR (typeof(size) = 'integer' AND size >= 1))
    ),
    dismissed_at INTEGER NOT NULL CHECK(
        typeof(dismissed_at) = 'integer' AND dismissed_at >= 0
    ),
    PRIMARY KEY(session_id, warning_id)
) WITHOUT ROWID;
