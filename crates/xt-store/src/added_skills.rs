//! Skills added to the Scores page (`grading_added_skills`, migration 26):
//! one row per installed grading skill the user added, keyed by its rubric
//! id.
//!
//! A row holds what finds the skill again (its folder, or for a Claude Code
//! plugin's skill `plugin:<plugin key>/<skill folder name>`) and its rubric's
//! name, description and stored version when it was added, so the skill
//! keeps a name if its folder or rubric later goes away. Adding refuses an id
//! or a location already stored ([`Store::added_skill_insert`]); removing a
//! row ([`Store::added_skill_remove`]) touches no stored grade. Locations are
//! local paths and plugin names, not conversation content; they are never
//! sent anywhere.

use crate::{Result, Store};
use rusqlite::params;

/// Most added skills one list returns.
pub const MAX_ADDED_SKILLS: u32 = 500;

/// One added skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedSkill {
    /// The rubric's id: lowercase letters, digits and `-`.
    pub skill_id: String,
    /// What finds the skill: its absolute folder path when it was added, or
    /// `plugin:<plugin key>/<skill folder name>`.
    pub location: String,
    pub name: String,
    pub description: String,
    /// The rubric's stored version when it was added.
    pub rubric_version: String,
    pub added_at_ms: i64,
}

impl Store {
    /// Store an added skill. False, and nothing changes, when its id or its
    /// location is already stored.
    pub fn added_skill_insert(&mut self, skill: &AddedSkill) -> Result<bool> {
        let changed = self.connection.execute(
            "INSERT INTO grading_added_skills
                 (skill_id,location,name,description,rubric_version,added_at)
             VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING",
            params![
                skill.skill_id,
                skill.location,
                skill.name,
                skill.description,
                skill.rubric_version,
                skill.added_at_ms
            ],
        )?;
        Ok(changed == 1)
    }

    /// Remove an added skill. False when it was not stored.
    pub fn added_skill_remove(&mut self, skill_id: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM grading_added_skills WHERE skill_id=?1",
            [skill_id],
        )? == 1)
    }

    /// The added skills, oldest first (by when they were added, then id), at
    /// most [`MAX_ADDED_SKILLS`].
    pub fn added_skills(&self) -> Result<Vec<AddedSkill>> {
        Ok(self
            .connection
            .prepare(
                "SELECT skill_id,location,name,description,rubric_version,added_at
                 FROM grading_added_skills ORDER BY added_at, skill_id LIMIT ?1",
            )?
            .query_map([MAX_ADDED_SKILLS], |row| {
                Ok(AddedSkill {
                    skill_id: row.get(0)?,
                    location: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    rubric_version: row.get(4)?,
                    added_at_ms: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
