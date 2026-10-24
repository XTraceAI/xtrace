//! Grading runs: a grading skill run headless by Claude Code on one project
//! folder, without the user's personal Claude Code settings, inside Claude
//! Code's sandbox (the project read-only, no network), returning a validated
//! XTrace grader protocol v1 result scored with the skill's rubric.
//!
//! * [`skill`] — a grading skill folder (an unchanged `SKILL.md` +
//!   `xtrace-rubric.md`) and the built-in skills bundled under `vendor/`.
//! * [`rubric`] — the strict reader of `xtrace-rubric.md`.
//! * [`installed`] — the skills the user installed, and whether each has a
//!   valid rubric.
//! * [`rubric_store`] — which rubric applies to a skill (its folder's own, or
//!   one XTrace keeps), and saving a new one.
//! * [`refusals`] — the rubric maker's refusals kept across restarts, and
//!   whether one still applies to a skill.
//! * [`skill_link`] — skills from a public GitHub link: the link read,
//!   the downloaded archive unpacked within bounds, the skills it holds,
//!   whether each can be kept, keeping one and where it came from.
//! * [`maker`] — one rubric-making run: Claude Code reads a skill with the
//!   bundled `xtrace-rubric-maker` helper and writes its rubric, checked by
//!   [`rubric`].
//! * [`runner`] — one bounded, cancellable run with an allow-listed
//!   environment and a fixed Claude Code command line.
//! * [`step`] — the step a run is on, read from Claude Code's output as it
//!   goes, for the progress shown while it runs.
//! * [`protocol`] — the strict reader of the `result.json` a run leaves.
//! * [`staging`] — the private, whitespace-free folder a run happens in, and
//!   what of it is kept afterwards.
//!
//! The `claude` executable is found by the shared resolver in
//! [`xt_probes::executable`]; the child process is contained by
//! [`xt_probes::process`]. Storing results and the app's commands live in
//! `xt-store` and the desktop app.

pub mod installed;
pub mod maker;
pub mod protocol;
pub mod refusals;
pub mod rubric;
pub mod rubric_store;
pub mod runner;
pub mod skill;
pub mod skill_link;
pub mod staging;
pub mod step;

pub use protocol::{CriterionScore, GradeResult, Insight, ResultError, Severity};
pub use rubric::{Rubric, RubricError};
pub use runner::{GradeError, GradeRun, Limits};
pub use skill::{GradingSkill, SkillError};
pub use staging::{CopyLimits, Staging};
pub use step::StepCell;
pub use xt_probes::process::CancelFlag;
