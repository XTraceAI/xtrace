---
name: xtrace-pr-preparation
description: Prepare or update XTrace Desktop pull request descriptions with the repository template, actual verification, and preferred UI screenshots. Use before opening or editing a PR, including small changes.
---

# Prepare an XTrace Desktop PR

Run from the XTrace Desktop Git checkout. This skill grants no new permission
to push, publish, create a PR, or upload evidence; use the task's existing authorization.

If this is an older checkout without `scripts/pr/prepare.mjs`, follow the template
and evidence guidance below manually. Use `validateDescription` from
`scripts/ci/check-pr.mjs` if available. Report that the helper was unavailable;
do not claim it ran or fetch/cherry-pick unrelated code merely to obtain it.

1. Read `.github/pull_request_template.md` and the Pull requests section of
   `CONTRIBUTING.md`. Inspect the final diff. Explain the concrete problem and
   resulting behavior, and link a relevant public issue or acceptance contract
   when applicable. Include only publication-safe facts.
2. Record the checks actually run, their results, each relevant acceptance case's
   setup/action/expected/actual result, and checks not run with reasons. Preserve
   the repository's required review and native checks; this helper verifies text,
   not the code or the truth of its claims.
3. For visible UI changes, prefer screenshots of the changed state in both light
   and dark appearance, using synthetic or redacted data. For a tooltip, capture
   it open. Inspect every image before an authorized upload; never upload private
   screenshots automatically. If screenshots are unavailable, give a short reason.
   Missing screenshots warn without blocking preparation or CI. Screenshots do not
   replace the native interaction checks in `CONTRIBUTING.md`.
4. Write a temporary JSON file outside tracked source with `change` and
   `verification` strings. Optionally add `screenshots` as objects with `theme`
   (`light` or `dark`) and an uploaded HTTPS `url`, and `screenshotMissingReason`.
   Generate the exact description with:

   ```sh
   node scripts/pr/prepare.mjs --input /tmp/pr-facts.json --base origin/main --output /tmp/pr-body.md
   ```

   Use the actual target ref for `--base`. The helper examines changed UI/native
   paths, including working changes and untracked files; add `--ui` when any visible
   change falls outside those paths. Path detection is a reminder, not proof that
   the change affects the UI. Output files must be new to avoid accidental overwrite.

5. Read the complete generated output and warnings before submission. Resolve
   incorrect facts. Submit the exact validated Markdown through the authorized PR
   connector or `gh pr create/edit --body-file /tmp/pr-body.md`; do not rewrite
   headings during submission. This preparation command never contacts GitHub.
   Read the saved PR back to verify its body and screenshot links. If verification
   differs from what was submitted, correct it within the task's authorization.
