---
protocol: 1
id: claude-md-doctor
name: CLAUDE.md Doctor
version: 0.5.1+rubric.1
description: Checks a project's CLAUDE.md and AGENTS.md: size against official guidance, dead references, stale claims, and how well each rule held up in the project's own Claude Code history.
target: project
---

XTrace's rubric for the unchanged claude-md-doctor skill. The skill's exam
writes `report.json` (from its `diagnosis.json`) and `report.html` into the
parent of its work folder, which is the grade folder. Judge that diagnosis;
do not examine the project again and do not add, drop or re-rate a finding.

## Health of the memory files (100 points)

Award points by this rule, which belongs to this skill:

- Start at 100.
- Subtract 20 for each diagnosis with severity `critical` and 5 for each
  diagnosis with severity `warn`. Diagnoses with severity `info` cost
  nothing. Count every diagnosis, not only the ones you list as findings.
- The result is never below 0.
- If the exam's grade is F (an actively misleading or unloadable file),
  award 0.
- If the project has no memory file of its own (the skill's Mode B: intake
  found no CLAUDE.md or AGENTS.md inside the project; files above the project
  or in the user's own folder do not count), award at most 40.

If `report.json` has no `diagnosis` object, or lists the diagnosis stage as
not run, there is nothing to judge: write no result. A diagnosis object with
no diagnoses in it is a clean exam: judge it by the rule above.

## Findings

One finding per diagnosis in `report.json`, most severe first, keeping the
exam's own order within a severity. Severity `critical` stays `critical`,
`warn` becomes `warning`, `info` stays `info`. The title is the diagnosis
title, the detail is its detail, and the fix is its prescription; shorten a
text only to fit the result's length limits.

The summary is the exam's chief complaint. The report is `report.html`. The
run is not complete when `report.json` lists any skipped or incomplete stage.
