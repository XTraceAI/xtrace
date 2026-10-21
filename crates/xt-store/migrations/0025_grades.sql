-- One row per grading run: a built-in grading skill run on one project at
-- the user's request. A row is inserted `running` before the run starts and
-- is finished exactly once, into `succeeded`, `failed`, `cancelled` or
-- `interrupted` (the app closed during the run). Only a succeeded run has a
-- score (0-100, the skill's own) and a completeness flag; its summary,
-- insights and report file name are content, registered with content
-- retention, so a purge may clear them later. A failed or interrupted run
-- has a typed reason code and never a path. The report file name is
-- relative to the run's own grade folder. Additive only: no existing table
-- changes.
CREATE TABLE grades (
    run_id TEXT PRIMARY KEY NOT NULL CHECK(
        typeof(run_id) = 'text' AND length(run_id) = 36
        AND substr(run_id, 9, 1) = '-' AND substr(run_id, 14, 1) = '-'
        AND substr(run_id, 19, 1) = '-' AND substr(run_id, 24, 1) = '-'
        AND length(replace(run_id, '-', '')) = 32
        AND replace(run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_path TEXT NOT NULL CHECK(
        typeof(project_path) = 'text' AND length(project_path) BETWEEN 2 AND 4096
        AND project_path GLOB '/*'
    ),
    skill_id TEXT NOT NULL CHECK(
        typeof(skill_id) = 'text' AND length(skill_id) BETWEEN 1 AND 64
        AND skill_id NOT GLOB '*[^a-z0-9-]*'
    ),
    skill_version TEXT NOT NULL CHECK(
        typeof(skill_version) = 'text' AND length(skill_version) BETWEEN 1 AND 200
    ),
    started_at INTEGER NOT NULL CHECK(typeof(started_at) = 'integer' AND started_at >= 0),
    finished_at INTEGER CHECK(
        finished_at IS NULL OR (typeof(finished_at) = 'integer' AND finished_at >= started_at)
    ),
    status TEXT NOT NULL CHECK(status IN ('running', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    score INTEGER CHECK(score IS NULL OR (typeof(score) = 'integer' AND score BETWEEN 0 AND 100)),
    complete INTEGER CHECK(complete IS NULL OR (typeof(complete) = 'integer' AND complete IN (0, 1))),
    summary TEXT CHECK(summary IS NULL OR (typeof(summary) = 'text' AND length(summary) <= 1000)),
    insights_json TEXT CHECK(
        insights_json IS NULL OR (json_valid(insights_json) AND json_type(insights_json) = 'array')
    ),
    report_file TEXT CHECK(
        report_file IS NULL OR (
            typeof(report_file) = 'text' AND length(report_file) BETWEEN 1 AND 512
            AND report_file NOT GLOB '/*' AND report_file NOT LIKE '%..%'
        )
    ),
    failure_reason TEXT CHECK(
        failure_reason IS NULL OR (
            typeof(failure_reason) = 'text' AND length(failure_reason) BETWEEN 1 AND 64
            AND failure_reason NOT GLOB '*[^a-z_]*'
        )
    ),
    CHECK(CASE status
        WHEN 'running' THEN finished_at IS NULL AND score IS NULL AND complete IS NULL
            AND summary IS NULL AND insights_json IS NULL AND report_file IS NULL
            AND failure_reason IS NULL
        WHEN 'succeeded' THEN finished_at IS NOT NULL AND score IS NOT NULL
            AND complete IS NOT NULL AND failure_reason IS NULL
        WHEN 'cancelled' THEN finished_at IS NOT NULL AND score IS NULL AND complete IS NULL
            AND summary IS NULL AND insights_json IS NULL AND report_file IS NULL
            AND failure_reason IS NULL
        ELSE finished_at IS NOT NULL AND score IS NULL AND complete IS NULL
            AND summary IS NULL AND insights_json IS NULL AND report_file IS NULL
            AND failure_reason IS NOT NULL
    END)
);
-- At most one run is running at a time.
CREATE UNIQUE INDEX grades_one_running ON grades(status) WHERE status = 'running';
CREATE INDEX grades_by_started ON grades(started_at DESC, run_id DESC);
CREATE INDEX grades_by_project_skill ON grades(project_path, skill_id, started_at DESC, run_id DESC);
