-- Neutral M-21 answers and conservative source revisions. No prompt,
-- argument, comparison key or rendered clock time is retained here.
INSERT INTO meta(key,value) VALUES('session_warnings_revision_allocator','1');
CREATE TABLE session_warning_inputs (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=1),
    host TEXT NOT NULL,
    surface TEXT,
    first_ms INTEGER,
    last_ms INTEGER,
    CHECK((first_ms IS NULL)=(last_ms IS NULL)),
    CHECK(first_ms IS NULL OR first_ms<=last_ms)
);
CREATE INDEX session_warning_inputs_surface_bounds ON session_warning_inputs(host,surface,first_ms,last_ms);
-- Backfill once, before any saved answer exists, including checkpoint-skipped
-- history. Bounds include all records and only widen, so excluded records and
-- later deletions can make an overlap broader but never omit an affected window.
INSERT INTO session_warning_inputs(session_id,revision,host,surface,first_ms,last_ms)
SELECT s.session_id,1,s.host,s.surface,min(r.ts_ms),max(r.ts_ms)
FROM sessions s LEFT JOIN records r ON r.session_id=s.session_id GROUP BY s.session_id;
CREATE TABLE saved_session_warnings (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=1),
    rule_version INTEGER NOT NULL CHECK(typeof(rule_version)='integer' AND rule_version>=1),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json))
);

-- One persistent allocator prevents a deleted/recreated identifier from
-- reusing a committed revision. It is never global cache freshness: only
-- affected input rows receive each allocated value. Rollback restores both.
-- The ordinary whole-session window extends projections by -1s/+2s and an
-- exact leap-second instant can differ by up to 1s. Symmetric 3s overlap
-- padding conservatively includes either spelling. Unknown timing fans out
-- across the matching surface. These triggers select affected windows; only
-- M-09's existing HealthIndex decides timestamp health.
CREATE TRIGGER session_warnings_sessions_insert AFTER INSERT ON sessions
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    INSERT INTO session_warning_inputs(session_id,revision,host,surface) VALUES(NEW.session_id,(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator'),NEW.host,NEW.surface);
END;
CREATE TRIGGER session_warnings_sessions_update AFTER UPDATE ON sessions
WHEN OLD.session_id IS NOT NEW.session_id OR OLD.host IS NOT NEW.host OR OLD.surface IS NOT NEW.surface OR OLD.native_session_id IS NOT NEW.native_session_id OR OLD.kind IS NOT NEW.kind
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
    UPDATE session_warning_inputs SET host=NEW.host,surface=NEW.surface WHERE session_id=NEW.session_id;
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE (OLD.session_id IS NOT NEW.session_id OR OLD.host IS NOT NEW.host OR OLD.surface IS NOT NEW.surface OR OLD.kind IS NOT NEW.kind) AND
      ((host=OLD.host AND surface IS OLD.surface AND EXISTS(
    SELECT 1 FROM session_warning_inputs source WHERE source.session_id=NEW.session_id
    AND (source.first_ms IS NULL OR source.last_ms IS NULL OR session_warning_inputs.first_ms IS NULL OR session_warning_inputs.last_ms IS NULL
      OR (session_warning_inputs.first_ms<=source.last_ms+3000 AND session_warning_inputs.last_ms>=source.first_ms-3000)))) OR (host=NEW.host AND surface IS NEW.surface AND EXISTS(
    SELECT 1 FROM session_warning_inputs source WHERE source.session_id=NEW.session_id
    AND (source.first_ms IS NULL OR source.last_ms IS NULL OR session_warning_inputs.first_ms IS NULL OR session_warning_inputs.last_ms IS NULL
      OR (session_warning_inputs.first_ms<=source.last_ms+3000 AND session_warning_inputs.last_ms>=source.first_ms-3000)))));
END;
CREATE TRIGGER session_warnings_sessions_delete BEFORE DELETE ON sessions
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE (host=OLD.host AND surface IS OLD.surface AND EXISTS(
    SELECT 1 FROM session_warning_inputs source WHERE source.session_id=OLD.session_id
    AND (source.first_ms IS NULL OR source.last_ms IS NULL OR session_warning_inputs.first_ms IS NULL OR session_warning_inputs.last_ms IS NULL
      OR (session_warning_inputs.first_ms<=source.last_ms+3000 AND session_warning_inputs.last_ms>=source.first_ms-3000))));
END;
CREATE TRIGGER session_warnings_records_insert AFTER INSERT ON records
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
UPDATE session_warning_inputs SET
    first_ms=CASE WHEN first_ms IS NULL THEN NEW.ts_ms WHEN NEW.ts_ms IS NULL THEN first_ms ELSE min(first_ms,NEW.ts_ms) END,
    last_ms=CASE WHEN last_ms IS NULL THEN NEW.ts_ms WHEN NEW.ts_ms IS NULL THEN last_ms ELSE max(last_ms,NEW.ts_ms) END
    WHERE session_id=NEW.session_id;
UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE ((EXISTS(SELECT 1 FROM sessions source WHERE source.session_id=NEW.session_id
    AND source.host=session_warning_inputs.host AND source.surface IS session_warning_inputs.surface)
    AND (NEW.ts_ms IS NULL OR first_ms IS NULL OR last_ms IS NULL
      OR (first_ms<=NEW.ts_ms+3000 AND last_ms>=NEW.ts_ms-3000))));
END;
CREATE TRIGGER session_warnings_records_update AFTER UPDATE ON records
WHEN OLD.uuid IS NOT NEW.uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.type IS NOT NEW.type OR OLD.ts IS NOT NEW.ts OR OLD.ts_ms IS NOT NEW.ts_ms OR OLD.is_meta IS NOT NEW.is_meta OR OLD.is_sidechain IS NOT NEW.is_sidechain OR OLD.role IS NOT NEW.role OR OLD.model IS NOT NEW.model OR OLD.is_tool_result_carrier IS NOT NEW.is_tool_result_carrier OR OLD.text_len IS NOT NEW.text_len OR OLD.tool_use_count IS NOT NEW.tool_use_count OR OLD.content_json IS NOT NEW.content_json OR OLD.has_conflict IS NOT NEW.has_conflict OR OLD.parent_uuid IS NOT NEW.parent_uuid OR OLD.agent_id IS NOT NEW.agent_id OR OLD.subtype IS NOT NEW.subtype OR OLD.is_human IS NOT NEW.is_human OR OLD.is_command IS NOT NEW.is_command OR OLD.is_interrupted IS NOT NEW.is_interrupted OR OLD.is_system_reminder IS NOT NEW.is_system_reminder
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
UPDATE session_warning_inputs SET
    first_ms=CASE WHEN first_ms IS NULL THEN NEW.ts_ms WHEN NEW.ts_ms IS NULL THEN first_ms ELSE min(first_ms,NEW.ts_ms) END,
    last_ms=CASE WHEN last_ms IS NULL THEN NEW.ts_ms WHEN NEW.ts_ms IS NULL THEN last_ms ELSE max(last_ms,NEW.ts_ms) END
    WHERE session_id=NEW.session_id;
UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE (OLD.uuid IS NOT NEW.uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.type IS NOT NEW.type OR OLD.ts IS NOT NEW.ts OR OLD.ts_ms IS NOT NEW.ts_ms OR OLD.is_meta IS NOT NEW.is_meta OR OLD.model IS NOT NEW.model) AND ((EXISTS(SELECT 1 FROM sessions source WHERE source.session_id=OLD.session_id
    AND source.host=session_warning_inputs.host AND source.surface IS session_warning_inputs.surface)
    AND (OLD.ts_ms IS NULL OR first_ms IS NULL OR last_ms IS NULL
      OR (first_ms<=OLD.ts_ms+3000 AND last_ms>=OLD.ts_ms-3000))) OR (EXISTS(SELECT 1 FROM sessions source WHERE source.session_id=NEW.session_id
    AND source.host=session_warning_inputs.host AND source.surface IS session_warning_inputs.surface)
    AND (NEW.ts_ms IS NULL OR first_ms IS NULL OR last_ms IS NULL
      OR (first_ms<=NEW.ts_ms+3000 AND last_ms>=NEW.ts_ms-3000))));
END;
CREATE TRIGGER session_warnings_records_delete AFTER DELETE ON records
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE ((EXISTS(SELECT 1 FROM sessions source WHERE source.session_id=OLD.session_id
    AND source.host=session_warning_inputs.host AND source.surface IS session_warning_inputs.surface)
    AND (OLD.ts_ms IS NULL OR first_ms IS NULL OR last_ms IS NULL
      OR (first_ms<=OLD.ts_ms+3000 AND last_ms>=OLD.ts_ms-3000))));
END;
CREATE TRIGGER session_warnings_tool_uses_insert AFTER INSERT ON tool_uses
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_uses_update AFTER UPDATE ON tool_uses
WHEN OLD.uuid IS NOT NEW.uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.block_index IS NOT NEW.block_index OR OLD.name IS NOT NEW.name OR OLD.group_version IS NOT NEW.group_version OR OLD.group_key IS NOT NEW.group_key OR OLD.group_conflict IS NOT NEW.group_conflict OR OLD.call_id IS NOT NEW.call_id OR OLD.call_id_conflict IS NOT NEW.call_id_conflict
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_uses_delete AFTER DELETE ON tool_uses
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_tool_results_insert AFTER INSERT ON tool_results
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_results_update AFTER UPDATE ON tool_results
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.block_index IS NOT NEW.block_index OR OLD.call_id IS NOT NEW.call_id OR OLD.is_error IS NOT NEW.is_error OR OLD.flag_stated IS NOT NEW.flag_stated OR OLD.wording IS NOT NEW.wording OR OLD.fact_version IS NOT NEW.fact_version OR OLD.conflicted IS NOT NEW.conflicted
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_results_delete AFTER DELETE ON tool_results
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_record_previews_insert AFTER INSERT ON record_previews
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_record_previews_update AFTER UPDATE ON record_previews
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.kind IS NOT NEW.kind OR OLD.text IS NOT NEW.text OR OLD.truncated IS NOT NEW.truncated OR OLD.rule_version IS NOT NEW.rule_version
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_record_previews_delete AFTER DELETE ON record_previews
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_confirmed_automated_inputs_insert AFTER INSERT ON confirmed_automated_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_confirmed_automated_inputs_update AFTER UPDATE ON confirmed_automated_inputs
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_confirmed_automated_inputs_delete AFTER DELETE ON confirmed_automated_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_guardian_turn_inputs_insert AFTER INSERT ON guardian_turn_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_guardian_turn_inputs_update AFTER UPDATE ON guardian_turn_inputs
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_guardian_turn_inputs_delete AFTER DELETE ON guardian_turn_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_injected_context_inputs_insert AFTER INSERT ON injected_context_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_injected_context_inputs_update AFTER UPDATE ON injected_context_inputs
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_injected_context_inputs_delete AFTER DELETE ON injected_context_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_task_notification_inputs_insert AFTER INSERT ON task_notification_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_task_notification_inputs_update AFTER UPDATE ON task_notification_inputs
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_task_notification_inputs_delete AFTER DELETE ON task_notification_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_tool_sent_inputs_insert AFTER INSERT ON tool_sent_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_sent_inputs_update AFTER UPDATE ON tool_sent_inputs
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.evidence_kind IS NOT NEW.evidence_kind
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_tool_sent_inputs_delete AFTER DELETE ON tool_sent_inputs
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_session_creation_relations_insert AFTER INSERT ON session_creation_relations
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.child_session_id);
END;
CREATE TRIGGER session_warnings_session_creation_relations_update AFTER UPDATE ON session_creation_relations
WHEN OLD.child_session_id IS NOT NEW.child_session_id OR OLD.child_host IS NOT NEW.child_host OR OLD.child_native_session_id IS NOT NEW.child_native_session_id OR OLD.state IS NOT NEW.state
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.child_session_id,NEW.child_session_id);
END;
CREATE TRIGGER session_warnings_session_creation_relations_delete AFTER DELETE ON session_creation_relations
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.child_session_id);
END;
CREATE TRIGGER session_warnings_human_session_origins_insert AFTER INSERT ON human_session_origins
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_human_session_origins_update AFTER UPDATE ON human_session_origins
WHEN OLD.session_id IS NOT NEW.session_id OR OLD.host IS NOT NEW.host OR OLD.native_session_id IS NOT NEW.native_session_id OR OLD.conflicted IS NOT NEW.conflicted
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_human_session_origins_delete AFTER DELETE ON human_session_origins
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
CREATE TRIGGER session_warnings_human_input_adjustments_insert AFTER INSERT ON human_input_adjustments
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (NEW.session_id);
END;
CREATE TRIGGER session_warnings_human_input_adjustments_update AFTER UPDATE ON human_input_adjustments
WHEN OLD.record_uuid IS NOT NEW.record_uuid OR OLD.session_id IS NOT NEW.session_id OR OLD.original_ts IS NOT NEW.original_ts OR OLD.original_length IS NOT NEW.original_length OR OLD.retained_length IS NOT NEW.retained_length OR OLD.conflicted IS NOT NEW.conflicted
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id,NEW.session_id);
END;
CREATE TRIGGER session_warnings_human_input_adjustments_delete AFTER DELETE ON human_input_adjustments
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM meta
      WHERE key='session_warnings_revision_allocator' AND typeof(value)='text'
        AND length(value) BETWEEN 1 AND 19 AND value NOT GLOB '*[^0-9]*'
        AND (length(value)<19 OR value<'9223372036854775807'))
      THEN RAISE(ABORT,'warning revision allocator is invalid') END;
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='session_warnings_revision_allocator';
    UPDATE session_warning_inputs SET revision=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='session_warnings_revision_allocator') WHERE session_id IN (OLD.session_id);
END;
