-- Skills the user added to the Scores page from the skills they installed:
-- one row per added skill, keyed by its rubric id. `location` finds the
-- skill again: for a skill in a skills folder, its folder where it was found
-- when it was added (an absolute path on this machine); for a Claude Code
-- plugin's skill, `plugin:<plugin key>/<skill folder name>`, which the app
-- resolves to the plugin's current install folder each time, so a plugin
-- update that installs into a new folder keeps the skill. `name`,
-- `description` and `rubric_version` are the rubric's when it was added (the
-- version with a short hash of the rubric file), so a card whose folder or
-- rubric later goes away still has a name. Built-in skills are never stored
-- here. Removing a row takes the skill off Scores and nothing else: its
-- stored grades stay. Additive only: no existing table changes.
CREATE TABLE grading_added_skills (
    skill_id TEXT PRIMARY KEY NOT NULL CHECK(
        typeof(skill_id) = 'text' AND length(skill_id) BETWEEN 1 AND 64
        AND skill_id NOT GLOB '*[^a-z0-9-]*'
    ),
    location TEXT NOT NULL UNIQUE CHECK(
        typeof(location) = 'text' AND length(location) BETWEEN 2 AND 4096
        AND (location GLOB '/*' OR location GLOB 'plugin:?*/?*')
    ),
    name TEXT NOT NULL CHECK(typeof(name) = 'text' AND length(name) BETWEEN 1 AND 200),
    description TEXT NOT NULL CHECK(typeof(description) = 'text' AND length(description) <= 1000),
    rubric_version TEXT NOT NULL CHECK(
        typeof(rubric_version) = 'text' AND length(rubric_version) BETWEEN 1 AND 200
    ),
    added_at INTEGER NOT NULL CHECK(typeof(added_at) = 'integer' AND added_at >= 0)
);
