-- A fifth closed kind of sub-session evidence, `claude_prompt_launch`: a
-- Claude agent's own recorded `Bash` call (in its session's saved transcript
-- or one of that session's helper-agent transcripts) ran a create-mode
-- `claude -p` command with a literal prompt, and exactly one indexed Claude
-- user session, first saved at or after that call, has exactly that prompt
-- as its first input, while no other launch could have made that session and
-- that launch could have made no other. It rests on the witness
-- `claude_bash_literal_prompt` and keeps the exact parent, the child's first
-- input, the call and the launch's position in it, in the columns the Codex
-- launch kinds already use. As for every kind, the Human input view reads an
-- accepted relation of it the same way; nothing else about the child changes.
-- The same launch is kept as a child fact of the same kind.
--
-- A CHECK cannot be widened in place, so both tables are rebuilt with their
-- rows, indexes and triggers unchanged and only their closed lists and this
-- kind's own checks widened. Dropping the relation table drops the saved
-- warning triggers migration 0029 put on it; they are recreated unchanged.
-- No path, command, prompt, output, time or digest is kept. The managed
-- views are dropped before every migration and recreated when the store
-- opens.
CREATE TABLE session_creation_relations_0032 (
    child_session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex', 'cursor', 'other')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(trim(child_native_session_id)) > 0 AND length(child_native_session_id) <= 256
    ),
    parent_host TEXT NOT NULL CHECK(parent_host IN ('claude', 'codex', 'cursor', 'other')),
    parent_native_session_id TEXT NOT NULL CHECK(
        length(trim(parent_native_session_id)) > 0 AND length(parent_native_session_id) <= 256
    ),
    evidence_kind TEXT NOT NULL CHECK(
        evidence_kind IN ('codex_thread_spawn', 'cli_artifact_create', 'codex_claude_launch',
                          'codex_cli_launch', 'claude_prompt_launch')
    ),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    witness TEXT NOT NULL CHECK(
        witness IN ('rollout_opening_session_meta', 'claude_cli_redirected_json_result',
                    'codex_exec_session_id_launch', 'codex_exec_json_thread_started',
                    'claude_bash_literal_prompt')
    ),
    state TEXT NOT NULL CHECK(state IN ('accepted', 'conflicted')),
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer'),
    parent_session_id TEXT,
    first_record_uuid TEXT,
    launch_call_id TEXT,
    launch_operation_index INTEGER,
    process_session_id TEXT,
    completion_call_id TEXT,
    output_read_call_id TEXT,
    provider_result_uuid TEXT,
    -- `codex_claude_launch` and `codex_cli_launch` only.
    completion_operation_index INTEGER,
    segment_rollout_id TEXT,
    launch_ordinal INTEGER,
    completion_ordinal INTEGER,
    UNIQUE(child_host, child_native_session_id),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(NOT (parent_host = child_host AND parent_native_session_id = child_native_session_id)),
    CHECK(evidence_kind IN ('codex_claude_launch', 'codex_cli_launch') OR (
        completion_operation_index IS NULL AND segment_rollout_id IS NULL
        AND launch_ordinal IS NULL AND completion_ordinal IS NULL
    )),
    CHECK(evidence_kind <> 'codex_thread_spawn' OR (
        child_host = 'codex' AND parent_host = 'codex'
        AND witness = 'rollout_opening_session_meta'
        AND parent_session_id IS NULL AND first_record_uuid IS NULL
        AND launch_call_id IS NULL AND launch_operation_index IS NULL
        AND process_session_id IS NULL AND completion_call_id IS NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
    )),
    CHECK(evidence_kind <> 'cli_artifact_create' OR (
        child_host = 'claude' AND parent_host = 'codex'
        AND witness = 'claude_cli_redirected_json_result'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL AND process_session_id IS NOT NULL
        AND completion_call_id IS NOT NULL AND output_read_call_id IS NOT NULL
        AND provider_result_uuid IS NOT NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(process_session_id) BETWEEN 1 AND 256
        AND process_session_id NOT GLOB '*[^!-~]*'
        AND length(completion_call_id) BETWEEN 1 AND 256
        AND completion_call_id NOT GLOB '*[^!-~]*'
        AND length(output_read_call_id) BETWEEN 1 AND 256
        AND output_read_call_id NOT GLOB '*[^!-~]*'
        AND length(provider_result_uuid) BETWEEN 1 AND 256
        AND provider_result_uuid NOT GLOB '*[^!-~]*'
    )),
    CHECK(evidence_kind <> 'codex_claude_launch' OR (
        child_host = 'claude' AND parent_host = 'codex'
        AND witness = 'codex_exec_session_id_launch'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL AND completion_call_id IS NOT NULL
        AND segment_rollout_id IS NOT NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND typeof(completion_operation_index) = 'integer' AND completion_operation_index >= 0
        AND ((launch_ordinal IS NULL AND completion_ordinal IS NULL) OR (
            typeof(launch_ordinal) = 'integer' AND typeof(completion_ordinal) = 'integer'
            AND launch_ordinal >= 0 AND completion_ordinal > launch_ordinal))
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
        AND (process_session_id IS NULL OR (length(process_session_id) BETWEEN 1 AND 256
            AND process_session_id NOT GLOB '*[^!-~]*'))
        AND length(completion_call_id) BETWEEN 1 AND 256
        AND completion_call_id NOT GLOB '*[^!-~]*'
        AND length(segment_rollout_id) = 36
        AND segment_rollout_id NOT GLOB '*[^0-9a-f-]*'
    )),
    -- A Codex agent's recorded fresh `codex exec --json` launch, and its
    -- whole chain: the same fields as `codex_claude_launch`, a Codex child.
    CHECK(evidence_kind <> 'codex_cli_launch' OR (
        child_host = 'codex' AND parent_host = 'codex'
        AND witness = 'codex_exec_json_thread_started'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL AND completion_call_id IS NOT NULL
        AND segment_rollout_id IS NOT NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND typeof(completion_operation_index) = 'integer' AND completion_operation_index >= 0
        AND ((launch_ordinal IS NULL AND completion_ordinal IS NULL) OR (
            typeof(launch_ordinal) = 'integer' AND typeof(completion_ordinal) = 'integer'
            AND launch_ordinal >= 0 AND completion_ordinal > launch_ordinal))
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
        AND (process_session_id IS NULL OR (length(process_session_id) BETWEEN 1 AND 256
            AND process_session_id NOT GLOB '*[^!-~]*'))
        AND length(completion_call_id) BETWEEN 1 AND 256
        AND completion_call_id NOT GLOB '*[^!-~]*'
        AND length(segment_rollout_id) = 36
        AND segment_rollout_id NOT GLOB '*[^0-9a-f-]*'
    )),
    -- A Claude agent's own recorded `Bash` call whose literal `claude -p`
    -- prompt is exactly the Claude child's first input: the exact parent,
    -- the child's first input, the call and the launch's place in it.
    CHECK(evidence_kind <> 'claude_prompt_launch' OR (
        child_host = 'claude' AND parent_host = 'claude'
        AND witness = 'claude_bash_literal_prompt'
        AND parent_session_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND launch_call_id IS NOT NULL
        AND process_session_id IS NULL AND completion_call_id IS NULL
        AND output_read_call_id IS NULL AND provider_result_uuid IS NULL
        AND parent_session_id <> child_session_id
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        AND length(parent_session_id) BETWEEN 1 AND 256
        AND parent_session_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND length(launch_call_id) BETWEEN 1 AND 256
        AND launch_call_id NOT GLOB '*[^!-~]*'
    ))
);
INSERT INTO session_creation_relations_0032(rowid,child_session_id,child_host,
    child_native_session_id,parent_host,parent_native_session_id,evidence_kind,
    evidence_version,witness,state,recorded_at,parent_session_id,first_record_uuid,
    launch_call_id,launch_operation_index,process_session_id,completion_call_id,
    output_read_call_id,provider_result_uuid,completion_operation_index,segment_rollout_id,
    launch_ordinal,completion_ordinal)
SELECT rowid,child_session_id,child_host,child_native_session_id,parent_host,
    parent_native_session_id,evidence_kind,evidence_version,witness,state,recorded_at,
    parent_session_id,first_record_uuid,launch_call_id,launch_operation_index,
    process_session_id,completion_call_id,output_read_call_id,provider_result_uuid,
    completion_operation_index,segment_rollout_id,launch_ordinal,completion_ordinal
FROM session_creation_relations;
DROP TRIGGER session_creation_relations_immutable_update;
DROP TRIGGER session_creation_relations_immutable_delete;
DROP TABLE session_creation_relations;
ALTER TABLE session_creation_relations_0032 RENAME TO session_creation_relations;
CREATE INDEX session_creation_relations_parent
    ON session_creation_relations(parent_host, parent_native_session_id);
CREATE INDEX session_creation_relations_launch
    ON session_creation_relations(parent_session_id, launch_call_id, launch_operation_index)
    WHERE evidence_kind IN ('cli_artifact_create', 'codex_claude_launch', 'codex_cli_launch',
                            'claude_prompt_launch');
CREATE TRIGGER session_creation_relations_immutable_update
BEFORE UPDATE ON session_creation_relations
WHEN NOT (
    OLD.state = 'accepted' AND NEW.state = 'conflicted'
    AND NEW.child_session_id IS OLD.child_session_id
    AND NEW.child_host IS OLD.child_host
    AND NEW.child_native_session_id IS OLD.child_native_session_id
    AND NEW.parent_host IS OLD.parent_host
    AND NEW.parent_native_session_id IS OLD.parent_native_session_id
    AND NEW.evidence_kind IS OLD.evidence_kind
    AND NEW.evidence_version IS OLD.evidence_version
    AND NEW.witness IS OLD.witness
    AND NEW.recorded_at IS OLD.recorded_at
    AND NEW.parent_session_id IS OLD.parent_session_id
    AND NEW.first_record_uuid IS OLD.first_record_uuid
    AND NEW.launch_call_id IS OLD.launch_call_id
    AND NEW.launch_operation_index IS OLD.launch_operation_index
    AND NEW.process_session_id IS OLD.process_session_id
    AND NEW.completion_call_id IS OLD.completion_call_id
    AND NEW.output_read_call_id IS OLD.output_read_call_id
    AND NEW.provider_result_uuid IS OLD.provider_result_uuid
    AND NEW.completion_operation_index IS OLD.completion_operation_index
    AND NEW.segment_rollout_id IS OLD.segment_rollout_id
    AND NEW.launch_ordinal IS OLD.launch_ordinal
    AND NEW.completion_ordinal IS OLD.completion_ordinal
)
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;
CREATE TRIGGER session_creation_relations_immutable_delete
BEFORE DELETE ON session_creation_relations
BEGIN SELECT RAISE(ABORT, 'session creation relation is immutable'); END;
-- Dropping the table dropped the saved-warning triggers on it; they are
-- recreated exactly as migration 0029 wrote them.
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

-- `claude_prompt_launch` child facts: the same launch as the relation, a
-- Claude session's own `Bash` call, its launch's place in that call and the
-- child's first input. No history file is named.
CREATE TABLE session_child_facts_0032 (
    child_session_id TEXT NOT NULL REFERENCES sessions(session_id),
    child_host TEXT NOT NULL CHECK(child_host IN ('claude', 'codex')),
    child_native_session_id TEXT NOT NULL CHECK(
        length(child_native_session_id) BETWEEN 1 AND 256
        AND child_native_session_id NOT GLOB '*[^!-~]*'
    ),
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN (
        'codex_thread_spawn', 'codex_guardian', 'codex_claude_launch', 'claude_bash_launch',
        'codex_cli_launch', 'claude_prompt_launch')),
    evidence_version INTEGER NOT NULL CHECK(
        typeof(evidence_version) = 'integer' AND evidence_version > 0
    ),
    source_native_session_id TEXT NOT NULL CHECK(
        length(source_native_session_id) BETWEEN 1 AND 256
        AND source_native_session_id NOT GLOB '*[^!-~]*'
    ),
    source_rollout_id TEXT,
    launch_call_id TEXT,
    launch_operation_index INTEGER,
    first_record_uuid TEXT,
    state TEXT NOT NULL CHECK(state IN ('accepted', 'withheld')),
    recorded_at INTEGER NOT NULL CHECK(typeof(recorded_at) = 'integer'),
    FOREIGN KEY(first_record_uuid, child_session_id) REFERENCES records(uuid, session_id),
    CHECK(evidence_kind NOT IN ('codex_thread_spawn', 'codex_guardian') OR (
        child_host = 'codex' AND source_native_session_id = child_native_session_id
        AND source_rollout_id IS NULL AND launch_call_id IS NULL
        AND launch_operation_index IS NULL AND first_record_uuid IS NULL
    )),
    CHECK(evidence_kind NOT IN ('codex_claude_launch', 'claude_bash_launch',
                                'claude_prompt_launch') OR (
        child_host = 'claude' AND source_native_session_id <> child_native_session_id
        AND launch_call_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
    )),
    CHECK(evidence_kind <> 'codex_claude_launch' OR (
        length(source_rollout_id) = 36 AND source_rollout_id NOT GLOB '*[^0-9a-f-]*'
    )),
    CHECK(evidence_kind NOT IN ('claude_bash_launch', 'claude_prompt_launch')
          OR source_rollout_id IS NULL),
    -- A Codex launch of a Codex child: another thread's call, operation and
    -- history file, and the child's first input, each a bounded token.
    CHECK(evidence_kind <> 'codex_cli_launch' OR (
        child_host = 'codex' AND source_native_session_id <> child_native_session_id
        AND launch_call_id IS NOT NULL AND first_record_uuid IS NOT NULL
        AND length(launch_call_id) BETWEEN 1 AND 256 AND launch_call_id NOT GLOB '*[^!-~]*'
        AND length(first_record_uuid) BETWEEN 1 AND 256
        AND first_record_uuid NOT GLOB '*[^!-~]*'
        AND typeof(launch_operation_index) = 'integer' AND launch_operation_index >= 0
        -- A CHECK passes on NULL, so the history file says so.
        AND source_rollout_id IS NOT NULL
        AND length(source_rollout_id) = 36 AND source_rollout_id NOT GLOB '*[^0-9a-f-]*'
    ))
);
INSERT INTO session_child_facts_0032(rowid,child_session_id,child_host,child_native_session_id,
    evidence_kind,evidence_version,source_native_session_id,source_rollout_id,launch_call_id,
    launch_operation_index,first_record_uuid,state,recorded_at)
SELECT rowid,child_session_id,child_host,child_native_session_id,evidence_kind,
    evidence_version,source_native_session_id,source_rollout_id,launch_call_id,launch_operation_index,
    first_record_uuid,state,recorded_at
FROM session_child_facts;
DROP TRIGGER session_child_facts_immutable_update;
DROP TRIGGER session_child_facts_immutable_delete;
DROP TABLE session_child_facts;
ALTER TABLE session_child_facts_0032 RENAME TO session_child_facts;
CREATE UNIQUE INDEX session_child_facts_anchor ON session_child_facts(
    child_session_id, evidence_kind, source_native_session_id,
    ifnull(source_rollout_id, ''), ifnull(launch_call_id, ''),
    ifnull(launch_operation_index, -1));
CREATE INDEX session_child_facts_launch ON session_child_facts(
    evidence_kind, source_native_session_id, launch_call_id, launch_operation_index)
    WHERE launch_call_id IS NOT NULL;
CREATE TRIGGER session_child_facts_immutable_update
BEFORE UPDATE ON session_child_facts
WHEN NOT (
    OLD.state = 'accepted' AND NEW.state = 'withheld'
    AND NEW.child_session_id IS OLD.child_session_id
    AND NEW.child_host IS OLD.child_host
    AND NEW.child_native_session_id IS OLD.child_native_session_id
    AND NEW.evidence_kind IS OLD.evidence_kind
    AND NEW.evidence_version IS OLD.evidence_version
    AND NEW.source_native_session_id IS OLD.source_native_session_id
    AND NEW.source_rollout_id IS OLD.source_rollout_id
    AND NEW.launch_call_id IS OLD.launch_call_id
    AND NEW.launch_operation_index IS OLD.launch_operation_index
    AND NEW.first_record_uuid IS OLD.first_record_uuid
    AND NEW.recorded_at IS OLD.recorded_at
)
BEGIN SELECT RAISE(ABORT, 'a child fact is immutable'); END;
CREATE TRIGGER session_child_facts_immutable_delete
BEFORE DELETE ON session_child_facts
BEGIN SELECT RAISE(ABORT, 'a child fact is immutable'); END;

-- The new detector, the Claude literal-prompt launch match, can only find
-- Claude children. A finished check of any other host stays finished: it
-- carries the new detector versions as it is. Claude sessions check again.
UPDATE session_child_checks
SET detector_versions = 'check1.spawn2.scan11.claude6.cli1.bash3.prompt1'
WHERE detector_versions = 'check1.spawn2.scan11.claude6.cli1.bash3'
  AND completed_generation IS NOT NULL
  AND session_id IN (SELECT session_id FROM sessions WHERE host <> 'claude');
