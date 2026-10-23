-- How each tool call ended, kept as metadata only, so the failed-call and
-- declined-call rules (M-21) work whatever the content retention setting says.
-- Both facts are derived at write time from the record's own blocks, before
-- retention decides about them (src/tool_result.rs):
--
-- * `tool_uses.call_id`: the host's own identifier of the call (a `tool_use`
--   block's `id`), the only link a tool result names. `call_id_conflict` is
--   set when two observations of the same call stated different identifiers;
--   such a call links to nothing.
-- * `tool_results`: one row per `tool_result` block — which call it answers
--   and how that call ended: `failed` (`is_error` true), `declined` (Claude
--   Code's "The user doesn't want to proceed with this tool use" result,
--   also an error), `ok`, or `unknown` when the block does not say. No result
--   text, no argument and no output is kept.
--
-- Rows fill once and are never rewritten by a replay; a later observation
-- that contradicts a row marks it `conflicted`, and readers treat a
-- conflicted row as unknown. Nothing is backfilled: records indexed before
-- this migration gain a row only when their source is read again.
ALTER TABLE tool_uses ADD COLUMN call_id TEXT CHECK(
    call_id IS NULL OR (typeof(call_id) = 'text' AND length(trim(call_id)) > 0
        AND length(call_id) <= 256)
);
ALTER TABLE tool_uses ADD COLUMN call_id_conflict INTEGER NOT NULL DEFAULT 0 CHECK(
    call_id_conflict IN (0, 1)
);
CREATE INDEX tool_uses_call_id ON tool_uses(session_id, call_id) WHERE call_id IS NOT NULL;
CREATE TABLE tool_results (
    record_uuid TEXT NOT NULL CHECK(length(trim(record_uuid)) > 0),
    session_id TEXT NOT NULL,
    block_index INTEGER NOT NULL CHECK(typeof(block_index) = 'integer' AND block_index >= 0),
    call_id TEXT CHECK(
        call_id IS NULL OR (typeof(call_id) = 'text' AND length(trim(call_id)) > 0
            AND length(call_id) <= 256)
    ),
    outcome TEXT NOT NULL CHECK(outcome IN ('ok', 'failed', 'declined', 'unknown')),
    rule_version INTEGER NOT NULL CHECK(typeof(rule_version) = 'integer' AND rule_version = 1),
    conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0, 1)),
    PRIMARY KEY(record_uuid, block_index),
    FOREIGN KEY(record_uuid, session_id) REFERENCES records(uuid, session_id)
);
CREATE INDEX tool_results_session ON tool_results(session_id);
