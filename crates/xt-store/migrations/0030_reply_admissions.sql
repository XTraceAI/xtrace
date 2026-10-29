-- Which fixed admission phrases each agent reply opens with ("you're right",
-- "my mistake", "sorry", …), for the admitted-mistake session warning (M-21).
-- Read at write time from the reply's own text blocks, before retention
-- decides about them (src/reply_admission.rs), so the rule works whatever the
-- content retention setting says.
--
-- Privacy: `phrases` holds only fixed phrase NAMES from this build's table
-- (`you_are_right`, `sorry`, …), space-separated in order of first
-- appearance, or the empty string when the reply has none. Never the reply's
-- text, never any other word of it. Like `tool_results`, the table holds no
-- conversation content, so "Delete stored content" leaves it.
--
-- One row per agent record outside sidechains whose text blocks are not
-- empty (a stored `text_len > 0`). Which names make a warning is decided by
-- the reader, in Rust, so narrowing that list needs no rewrite here.
-- `fact_version` names the write-time derivation: a later observation with a
-- higher version replaces the row; one with the same version that states
-- other phrases marks it `conflicted`, which readers treat as unknown; a
-- lower version changes nothing.
CREATE TABLE reply_admissions (
    record_uuid TEXT PRIMARY KEY NOT NULL CHECK(
        typeof(record_uuid) = 'text' AND length(trim(record_uuid)) > 0
        AND length(record_uuid) <= 256
    ),
    session_id TEXT NOT NULL,
    phrases TEXT NOT NULL CHECK(
        typeof(phrases) = 'text' AND length(phrases) <= 600
        AND phrases NOT GLOB '*[^a-z_ ]*'
    ),
    fact_version INTEGER NOT NULL CHECK(typeof(fact_version) = 'integer' AND fact_version >= 1),
    conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0, 1)),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
CREATE INDEX reply_admissions_session ON reply_admissions(session_id);

-- A changed fact changes the session's saved warnings (migration 29).
CREATE TRIGGER session_warnings_reply_admissions_insert AFTER INSERT ON reply_admissions
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_reply_admissions_update AFTER UPDATE ON reply_admissions
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.phrases IS NOT NEW.phrases OR OLD.fact_version IS NOT NEW.fact_version OR OLD.conflicted IS NOT NEW.conflicted
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_reply_admissions_delete AFTER DELETE ON reply_admissions
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;

-- Dismissals may now name an admitted-mistake warning, one event like a
-- correction (no size). The old `repeated_calls` identities stay valid: such
-- rows exist and are kept, though they hide nothing. Every existing row is
-- copied unchanged.
CREATE TABLE dismissed_warnings_rebuilt (
    session_id TEXT NOT NULL CHECK(
        typeof(session_id) = 'text' AND length(session_id) BETWEEN 1 AND 256
        AND trim(session_id) = session_id
    ),
    warning_id TEXT NOT NULL CHECK(
        typeof(warning_id) = 'text' AND length(warning_id) BETWEEN 3 AND 600
        AND (warning_id GLOB 'correction:?*' OR warning_id GLOB 'interruption:?*'
            OR warning_id GLOB 'repeated_calls:?*' OR warning_id GLOB 'repeated_prompt:?*'
            OR warning_id GLOB 'repeated_failure:?*' OR warning_id GLOB 'admitted_mistake:?*')
    ),
    size INTEGER CHECK(
        (size IS NULL) = (warning_id GLOB 'correction:?*' OR warning_id GLOB 'interruption:?*'
            OR warning_id GLOB 'admitted_mistake:?*')
        AND (size IS NULL OR (typeof(size) = 'integer' AND size >= 1))
    ),
    dismissed_at INTEGER NOT NULL CHECK(
        typeof(dismissed_at) = 'integer' AND dismissed_at >= 0
    ),
    PRIMARY KEY(session_id, warning_id)
) WITHOUT ROWID;
INSERT INTO dismissed_warnings_rebuilt(session_id, warning_id, size, dismissed_at)
SELECT session_id, warning_id, size, dismissed_at FROM dismissed_warnings;
DROP TABLE dismissed_warnings;
ALTER TABLE dismissed_warnings_rebuilt RENAME TO dismissed_warnings;

-- Fill the phrases of history indexed before this migration: the next native
-- scan replays every Claude transcript once (as migration 17 did), and each
-- reply it reads again gets its row. Codex and Cursor are read whole by the
-- pinned reader on every scan, so they fill without a reset. Re-read tool
-- results also gain their saved outcomes where they had none.
DELETE FROM native_checkpoints WHERE source='transcript';
