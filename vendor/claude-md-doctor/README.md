# Bundled grading skill: claude-md-doctor

`skill/` is the `skills/claude-md-doctor` folder of
[agent-clinic/claude-md-doctor](https://github.com/agent-clinic/claude-md-doctor)
(MIT, upstream version 0.5.1) at the commit pinned in `pin.json`, unchanged.
`LICENSE` is the upstream repository's license, byte for byte. XTrace's grading
runner (`xt-judge`) runs this folder headless with Claude Code; see
`crates/xt-judge`.

## XTrace's addition

One file is added, listed under `added` in `pin.json`:

- `skill/xtrace-rubric.md`: XTrace's rubric for this skill
  ([format](../../docs/GRADING-RUBRIC.md)). Its header keeps the built-in id
  `claude-md-doctor`, so grades stored before the rubric still match. Its one
  criterion holds the agreed score rule: start at 100, minus 20 for each
  critical diagnosis and 5 for each warning, never below 0; info costs nothing;
  grade F scores 0; a project with no memory file of its own scores at most 40.
  Its Findings section turns the exam's diagnoses into findings.

No upstream file is patched. The run's final XTrace step, not the skill, reads
the rubric and writes `result.json`.

`cargo test -p xt-judge` checks every upstream file against its pinned Git blob
ID (`crates/xt-judge/tests/vendored_skill.rs`) and that nothing else is in the
folder.

## Refreshing the pin

1. Fetch the new commit: `git clone https://github.com/agent-clinic/claude-md-doctor.git`
   then `git archive <commit> skills/claude-md-doctor LICENSE`.
2. Replace `skill/` with the new `skills/claude-md-doctor` (keep
   `skill/xtrace-rubric.md`) and `LICENSE` with the new license.
3. Update `pin.json`: `commit`, `skill_tree`, `upstream_version`,
   `license_blob` and `upstream_files` (from
   `git ls-tree -r <commit> skills/claude-md-doctor`).
4. Check that the rubric still describes the new version's report (its
   diagnosis fields and severities), bump the rubric's `version`, and run
   `cargo test -p xt-judge`.
