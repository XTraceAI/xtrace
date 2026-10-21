//! A grading skill: a normal, unchanged `SKILL.md` folder plus XTrace's
//! rubric, `xtrace-rubric.md` ([`crate::rubric`]), and the prompt a run
//! gives Claude Code.
//!
//! The rubric's header names the skill (id, name, version, description);
//! its criteria say how the skill's own output becomes a score. The
//! built-in skills ([`BUILTIN`]) are bundled under `vendor/<id>/skill`, and
//! a built-in's rubric must carry its built-in id
//! ([`GradingSkill::open_builtin`]). A skill the user installed
//! ([`crate::installed`]) and added to Scores is opened from its own folder
//! and must still carry the id it was added with ([`GradingSkill::open_as`]).

use crate::{
    protocol::{
        MAX_DETAIL_CHARS, MAX_FIX_CHARS, MAX_INSIGHTS, MAX_SCORE, MAX_SUMMARY_CHARS,
        MAX_TITLE_CHARS, REPORT_EXTENSION, RESULT_FILE,
    },
    rubric::{self, RUBRIC_FILE, Rubric, RubricError},
};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// The skills XTrace ships. The first is the default.
pub const BUILTIN: &[&str] = &["claude-md-doctor"];
/// The instructions' file name in a skill folder.
pub const INSTRUCTIONS_FILE: &str = "SKILL.md";
/// The placeholder Claude Code substitutes with a skill's own folder.
pub const SKILL_DIR_PLACEHOLDER: &str = "${CLAUDE_SKILL_DIR}";
/// Largest `SKILL.md` a skill may have.
pub const MAX_INSTRUCTIONS_BYTES: u64 = 512 * 1024;

/// Why a skill folder is not a usable grading skill. No path in messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    #[error("the grading skill is not a built-in skill")]
    Unknown,
    #[error("the grading skill folder is missing")]
    Missing,
    #[error("the grading skill folder's path is not valid UTF-8")]
    PathNotUtf8,
    #[error("the grading skill has no readable SKILL.md")]
    InstructionsUnreadable,
    #[error("{0}")]
    Rubric(#[from] RubricError),
    #[error("the grading skill's rubric names another skill")]
    IdMismatch,
}

impl SkillError {
    /// Whether the skill's rubric is what is wrong (missing or invalid).
    pub fn is_rubric(self) -> bool {
        matches!(self, Self::Rubric(_) | Self::IdMismatch)
    }
}

/// A skill folder that passed validation, with its folder resolved.
#[derive(Clone, Debug)]
pub struct GradingSkill {
    dir: PathBuf,
    /// The rubric file it was opened with: the folder's own, or one XTrace
    /// keeps for it ([`crate::rubric_store`]).
    rubric_file: PathBuf,
    rubric: Rubric,
    instructions: String,
}

/// The bundled folder of a built-in skill under a vendor root.
pub fn bundled_dir(vendor_root: &Path, id: &str) -> Result<PathBuf, SkillError> {
    if !BUILTIN.contains(&id) {
        return Err(SkillError::Unknown);
    }
    Ok(vendor_root.join(id).join("skill"))
}

impl GradingSkill {
    /// Open and validate a skill folder: its `SKILL.md` and its rubric.
    pub fn open(dir: &Path) -> Result<Self, SkillError> {
        Self::open_with(dir, None)
    }

    /// [`Self::open`] with the rubric read from `rubric_file` instead of the
    /// folder's own `xtrace-rubric.md` when given (the one rule that picks
    /// it is [`crate::rubric_store::applicable`]).
    pub fn open_with(dir: &Path, rubric_file: Option<&Path>) -> Result<Self, SkillError> {
        let dir = std::fs::canonicalize(dir).map_err(|_| SkillError::Missing)?;
        if !dir.is_dir() {
            return Err(SkillError::Missing);
        }
        if dir.to_str().is_none() {
            return Err(SkillError::PathNotUtf8);
        }
        let instructions = read_capped(&dir.join(INSTRUCTIONS_FILE), MAX_INSTRUCTIONS_BYTES)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(SkillError::InstructionsUnreadable)?;
        let rubric_file = rubric_file.map_or_else(|| dir.join(RUBRIC_FILE), Path::to_path_buf);
        let rubric = rubric::read_file(&rubric_file)?;
        Ok(Self {
            dir,
            rubric_file,
            rubric,
            instructions,
        })
    }

    /// Open a skill folder that must be the built-in skill `id`: the
    /// folder's rubric carries exactly that id.
    pub fn open_builtin(dir: &Path, id: &str) -> Result<Self, SkillError> {
        if !BUILTIN.contains(&id) {
            return Err(SkillError::Unknown);
        }
        Self::open_as(dir, id)
    }

    /// Open a skill folder whose rubric must carry exactly the id `id`.
    pub fn open_as(dir: &Path, id: &str) -> Result<Self, SkillError> {
        Self::open_as_with(dir, None, id)
    }

    /// [`Self::open_as`] with the rubric from `rubric_file` when given.
    pub fn open_as_with(
        dir: &Path,
        rubric_file: Option<&Path>,
        id: &str,
    ) -> Result<Self, SkillError> {
        let skill = Self::open_with(dir, rubric_file)?;
        if skill.id() != id {
            return Err(SkillError::IdMismatch);
        }
        Ok(skill)
    }

    /// The resolved skill folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The rubric file it was opened with.
    pub fn rubric_path(&self) -> PathBuf {
        self.rubric_file.clone()
    }

    /// Whether its rubric is the folder's own `xtrace-rubric.md` (otherwise
    /// it is one XTrace keeps, which a run places in the staged copy).
    pub fn rubric_in_folder(&self) -> bool {
        self.rubric_file == self.dir.join(RUBRIC_FILE)
    }

    pub fn rubric(&self) -> &Rubric {
        &self.rubric
    }

    pub fn id(&self) -> &str {
        &self.rubric.id
    }

    pub fn name(&self) -> &str {
        &self.rubric.name
    }

    pub fn description(&self) -> &str {
        &self.rubric.description
    }

    /// The version a run is stored and shown with ([`Rubric::stored_version`]).
    pub fn version(&self) -> String {
        self.rubric.stored_version()
    }

    /// The prompt for one run: the XTrace header (who and where, and that
    /// the project is read-only), the skill's own instructions with
    /// [`SKILL_DIR_PLACEHOLDER`] replaced by its folder (a `-p` run loads no
    /// skill, so nothing else substitutes it), and the final XTrace step
    /// that judges the skill's output with the rubric and writes
    /// `result.json`. Paths must already be resolved and UTF-8.
    pub fn prompt(&self, project: &str, grade: &str, work: &str) -> String {
        let dir = self.dir.to_str().unwrap_or_default();
        let rubric = self.rubric_path();
        let rubric = rubric.to_str().unwrap_or_default();
        let criteria = self
            .rubric
            .criteria
            .iter()
            .map(|criterion| {
                format!(
                    r#"{{"name": {}, "max": {}, "awarded": 0}}"#,
                    serde_json::Value::from(criterion.name.as_str()),
                    criterion.points
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let instructions = self.instructions.replace(SKILL_DIR_PLACEHOLDER, dir);
        format!(
            "XTrace grading run (headless). REPO = {project}. WORK = {work}. GRADE = {grade}.\n\
             REPO is the project to examine and is read-only: write files only under WORK or \
             GRADE, never inside REPO. In the skill instructions below, REPO and WORK mean \
             exactly these folders, whatever the instructions say they default to; wherever a \
             step or script takes a work or output folder, give it WORK. Nobody can answer \
             questions during this run. Follow the skill instructions on REPO from their first \
             step to their last, then do the XTrace step after them.\n\n\
             ===== Skill instructions =====\n\n{instructions}\n\n\
             ===== End of skill instructions =====\n\n\
             XTrace step (after the skill is done, in every mode):\n\
             1. Read the rubric at {rubric}.\n\
             2. Judge the skill's own output from this run (what it reported and the files it \
             wrote under GRADE) against each criterion of the rubric, following the rubric's \
             guidance. Do not examine REPO again and do not change what the skill found.\n\
             3. Write {grade}/{RESULT_FILE}: one JSON object with exactly the keys protocol, \
             score, summary, insights, criteria, report and complete:\n\
             - \"protocol\": 1.\n\
             - \"criteria\": one object per rubric criterion, in this order, with the names and \
             max points exactly as given here: [{criteria}]; set each \"awarded\" to a whole \
             number from 0 to its \"max\".\n\
             - \"score\": the sum of the awarded points, a whole number from 0 to {MAX_SCORE}.\n\
             - \"summary\": one or two plain sentences, at most {MAX_SUMMARY_CHARS} characters.\n\
             - \"insights\": the findings, most severe first, at most {MAX_INSIGHTS}, each \
             {{\"severity\": \"critical\" | \"warning\" | \"info\" | \"pass\", \"title\": at most \
             {MAX_TITLE_CHARS} characters, \"detail\": optional, at most {MAX_DETAIL_CHARS} \
             characters, \"fix\": optional, at most {MAX_FIX_CHARS} characters}}; follow the \
             rubric's Findings guidance when it has one.\n\
             - \"report\": the path, relative to GRADE, of the .{REPORT_EXTENSION} report the \
             skill wrote inside GRADE but not under WORK; null when there is none.\n\
             - \"complete\": false when the skill skipped a step or could not finish, otherwise \
             true.\n\
             If the skill produced nothing the rubric can judge, write no {RESULT_FILE} and say \
             why."
        )
    }
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vendor() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor"))
    }

    fn bundled() -> GradingSkill {
        GradingSkill::open_builtin(
            &bundled_dir(&vendor(), "claude-md-doctor").unwrap(),
            "claude-md-doctor",
        )
        .unwrap()
    }

    #[test]
    fn the_bundled_skill_is_a_valid_grader_with_its_built_in_id() {
        let skill = bundled();
        assert_eq!(skill.id(), "claude-md-doctor");
        assert_eq!(skill.name(), "CLAUDE.md Doctor");
        assert!(
            skill
                .version()
                .starts_with(&format!("{} #", skill.rubric().version))
        );
        assert_eq!(bundled_dir(&vendor(), "other"), Err(SkillError::Unknown));
    }

    #[test]
    fn the_bundled_rubric_encodes_the_agreed_score_rule() {
        let skill = bundled();
        let rubric = skill.rubric();
        assert_eq!(rubric.criteria.len(), 1);
        assert_eq!(rubric.criteria[0].points, 100);
        let guidance = &rubric.criteria[0].guidance;
        for rule in [
            "Start at 100",
            "20 for each",
            "`critical`",
            "5 for each",
            "`warn`",
            "`info`",
            "never below 0",
            "grade is F",
            "award 0",
            "at most 40",
        ] {
            assert!(guidance.contains(rule), "{rule}");
        }
        let findings = rubric.findings.as_deref().unwrap();
        for rule in ["`critical`", "`warn`", "`warning`", "prescription", "fix"] {
            assert!(findings.contains(rule), "{rule}");
        }
    }

    #[test]
    fn the_prompt_has_the_header_the_skill_and_the_xtrace_step() {
        let skill = bundled();
        let prompt = skill.prompt("/r", "/g", "/g/work");
        assert!(prompt.starts_with(
            "XTrace grading run (headless). REPO = /r. WORK = /g/work. GRADE = /g.\n\
             REPO is the project to examine and is read-only: write files only under WORK or \
             GRADE, never inside REPO."
        ));
        assert!(!prompt.contains(SKILL_DIR_PLACEHOLDER));
        assert!(prompt.contains(&format!("`SCRIPTS` = `{}/scripts`", skill.dir().display())));
        let rubric = format!("1. Read the rubric at {}.", skill.rubric_path().display());
        let (skill_part, step) = prompt
            .split_once("===== End of skill instructions =====")
            .unwrap();
        assert!(skill_part.contains("## Stage 6 — report"));
        assert!(step.contains(&rubric));
        assert!(step.contains("3. Write /g/result.json"));
        assert!(
            step.contains(r#"[{"name": "Health of the memory files", "max": 100, "awarded": 0}]"#)
        );
        assert!(step.contains("write no result.json"));
    }

    #[test]
    fn a_missing_or_invalid_rubric_refuses_the_skill() {
        let root = tempfile::Builder::new()
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
            .unwrap();
        let dir = root.path().join("skill");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(INSTRUCTIONS_FILE), "Do it.").unwrap();
        let open = || GradingSkill::open(&dir).map(|skill| skill.id().to_owned());
        assert_eq!(open(), Err(SkillError::Rubric(RubricError::Missing)));
        assert!(open().unwrap_err().is_rubric());
        std::fs::write(dir.join(RUBRIC_FILE), "no header").unwrap();
        assert_eq!(open(), Err(SkillError::Rubric(RubricError::NoHeader)));
        std::fs::write(
            dir.join(RUBRIC_FILE),
            "---\nprotocol: 1\nid: other\nname: O\nversion: 1\ndescription: d\n\
             target: project\n---\n## All (100 points)\nAll.\n",
        )
        .unwrap();
        assert_eq!(open(), Ok("other".to_owned()));
        assert_eq!(
            GradingSkill::open_builtin(&dir, "claude-md-doctor").map(|_| ()),
            Err(SkillError::IdMismatch)
        );
        assert!(SkillError::IdMismatch.is_rubric());
        assert_eq!(
            GradingSkill::open_builtin(&dir, "other").map(|_| ()),
            Err(SkillError::Unknown)
        );
        std::fs::remove_file(dir.join(INSTRUCTIONS_FILE)).unwrap();
        assert_eq!(open(), Err(SkillError::InstructionsUnreadable));
        assert!(!SkillError::InstructionsUnreadable.is_rubric());
    }
}
