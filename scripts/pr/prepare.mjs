import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import { validateDescription } from '../ci/check-pr.mjs';

const usage = `Usage: node scripts/pr/prepare.mjs --input facts.json [--base origin/main] [--ui] [--output body.md]
Prepares and validates a PR body; never submits it or uploads screenshots.
JSON: { "change": "Actual problem and resulting behavior", "verification": "Actual checks, results, and checks not run", "screenshots": [{ "theme": "light", "url": "https://..." }], "screenshotMissingReason": "Why evidence is unavailable" }
Screenshots and their omission reason are optional. UI detection checks changed paths
against the base's merge point, including working changes and untracked files.
Use --ui for visible changes outside the detected UI/native paths.`;

function content(value, name) {
  if (
    typeof value !== 'string' ||
    !value.trim() ||
    /^(?:TODO|TBD|N\/A|placeholder|replace me|<[^>]+>|\[[^\]]+\])[.!\s]*$/i.test(value.trim()) ||
    /^\s*##\s/m.test(value)
  )
    throw new Error(
      `${name} needs actual content without extra level-two headings or placeholders.`,
    );
  return value.trim();
}

async function main() {
  const { values } = parseArgs({
    options: {
      input: { type: 'string' },
      base: { type: 'string', default: 'origin/main' },
      ui: { type: 'boolean', default: false },
      output: { type: 'string' },
      help: { type: 'boolean', default: false },
    },
  });
  if (values.help) return console.log(usage);
  if (!values.input) throw new Error(usage);
  const facts = JSON.parse(await readFile(values.input, 'utf8'));
  let body = `## Change\n\n${content(facts.change, 'change')}\n\n## Verification\n\n${content(facts.verification, 'verification')}\n`;
  // Validate the supplied checks before evidence notes can make the section substantive.
  validateDescription(body);
  const screenshots = facts.screenshots ?? [];
  if (!Array.isArray(screenshots)) throw new Error('screenshots must be an array.');
  for (const shot of screenshots) {
    if (
      !shot ||
      !['light', 'dark'].includes(shot.theme) ||
      typeof shot.url !== 'string' ||
      !/^https:\/\/[^\s<>]+$/.test(shot.url)
    )
      throw new Error('Each screenshot needs theme light or dark and an HTTPS URL.');
    // URL parsing rejects malformed hosts without making a network request.
    new URL(shot.url);
  }
  const reason =
    facts.screenshotMissingReason === undefined
      ? 'Not captured; reason not supplied.'
      : content(facts.screenshotMissingReason, 'screenshotMissingReason');
  const git = (...args) =>
    execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  const mergePoint = git('merge-base', values.base, 'HEAD');
  const paths = [
    ...git('diff', '--name-only', '-z', mergePoint, '--').split('\0'),
    ...git('ls-files', '--others', '--exclude-standard', '-z').split('\0'),
  ];
  const ui =
    values.ui ||
    paths.some((path) =>
      /^apps\/desktop\/(?:ui\/|src-tauri\/(?:src\/|icons\/|tauri\.conf))/.test(path),
    );
  if (ui || screenshots.length) {
    body += '\n### UI evidence\n';
    for (const shot of screenshots) body += `\n![${shot.theme} appearance](<${shot.url}>)\n`;
    const missing = ['light', 'dark'].filter(
      (theme) => !screenshots.some((shot) => shot.theme === theme),
    );
    if (ui && missing.length) {
      const message = `Screenshots not attached for ${missing.join(' and ')} appearance. ${reason}`;
      body += `\n${message}\n`;
      console.error(`Warning: ${message}`);
    }
  }
  validateDescription(body);
  if (values.output) await writeFile(values.output, body, { flag: 'wx' });
  process.stdout.write(body);
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
