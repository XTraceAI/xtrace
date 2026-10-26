import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync } from 'node:fs';
import { isDeepStrictEqual } from 'node:util';
import { pathToFileURL } from 'node:url';

const ordinaryActions = ['opened', 'reopened', 'synchronize', 'ready_for_review'];
const sha = /^[a-f0-9]{40}$/;
const textOnly =
  /^(?:(?:AGENTS|README|CONTRIBUTING|CODE_OF_CONDUCT|SECURITY|TRADEMARKS|TELEMETRY|DEVELOPING|THIRD_PARTY_NOTICES)\.md|\.github\/pull_request_template\.md|\.agents\/skills\/xtrace-pr-preparation\/SKILL\.md|(?:docs|change-notes)\/(?:[^/]+\/)*[^/]+\.md|scripts\/pr\/[^/]+\.mjs)$/;

// This is an allowlist: new or uncertain inputs always keep browser validation.
export function selectUi({ eventName, event, githubSha, repo = process.cwd() }) {
  try {
    const pr = event?.pull_request;
    if (
      eventName !== 'pull_request' ||
      !ordinaryActions.includes(event?.action) ||
      (event.changes && Object.hasOwn(event.changes, 'base')) ||
      !sha.test(pr?.base?.sha ?? '') ||
      !sha.test(pr?.head?.sha ?? '') ||
      !sha.test(githubSha ?? '')
    )
      return true;
    const git = (...args) =>
      execFileSync('git', args, {
        cwd: repo,
        encoding: 'utf8',
        timeout: 10_000,
        maxBuffer: 8 * 1024 * 1024,
        stdio: ['ignore', 'pipe', 'pipe'],
      }).trimEnd();
    const head = git('rev-parse', '--verify', 'HEAD');
    if (head !== githubSha) return true;
    for (const revision of [pr.base.sha, pr.head.sha])
      if (git('rev-parse', '--verify', `${revision}^{commit}`) !== revision) return true;
    if (head !== pr.head.sha) {
      const parents = git('show', '-s', '--format=%P', head).split(' ');
      if (!isDeepStrictEqual(parents, [pr.base.sha, pr.head.sha])) return true;
    }
    const base = git('merge-base', pr.base.sha, head);
    if (base !== pr.base.sha) return true;
    // Disable rename detection so both sides of a rename must be safe.
    const paths = git('diff', '--no-renames', '--name-only', '-z', base, head, '--')
      .split('\0')
      .filter(Boolean);
    if (paths.length === 0) return true;
    return paths.some((path) => {
      if (textOnly.test(path)) return false;
      if (path !== 'package.json') return true;
      const manifests = [base, head].map((revision) => {
        const manifest = JSON.parse(git('show', `${revision}:package.json`));
        if (
          !manifest?.scripts ||
          typeof manifest.scripts !== 'object' ||
          Array.isArray(manifest.scripts)
        )
          throw new Error('Invalid scripts');
        delete manifest.scripts['test:ci'];
        return manifest;
      });
      return !isDeepStrictEqual(...manifests);
    });
  } catch {
    return true;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let event;
  try {
    event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
  } catch {
    // Missing or malformed event data keeps the full UI suite.
  }
  const value = selectUi({
    eventName: process.env.GITHUB_EVENT_NAME,
    event,
    githubSha: process.env.GITHUB_SHA,
  });
  const output = `run_ui=${value}\n`;
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, output);
  process.stdout.write(output);
}
