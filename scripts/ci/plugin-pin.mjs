import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdtempSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CheckError } from './policy.mjs';

// The pin names one reviewed producer revision. Conformance runs against that
// exact checkout, and the reader sources it lists are verified by Git object
// identity, so a checkout at the right commit with edited files cannot pass.
const REPOSITORY = 'https://github.com/XTraceAI/agent-plugins.git';
const OBJECT_ID = /^[0-9a-f]{40}$/;
const VERSION = /^\d+\.\d+\.\d+$/;
const RELATIVE = /^[A-Za-z0-9_][A-Za-z0-9_./-]*$/;

export function parsePin(text) {
  let pin;
  try {
    pin = JSON.parse(text);
  } catch {
    throw new CheckError('.plugin-pin must be a JSON object.');
  }
  if (!pin || typeof pin !== 'object' || Array.isArray(pin))
    throw new CheckError('.plugin-pin must be a JSON object.');
  const keys = Object.keys(pin).sort().join(',');
  if (keys !== 'commit,plugin_root,plugin_version,reader_sources,repository')
    throw new CheckError('.plugin-pin must declare exactly its known keys.');
  if (pin.repository !== REPOSITORY)
    throw new CheckError('.plugin-pin repository is not the reviewed public producer.');
  if (!OBJECT_ID.test(pin.commit)) throw new CheckError('.plugin-pin commit must be a full SHA.');
  if (!RELATIVE.test(pin.plugin_root) || pin.plugin_root.split('/').includes('..'))
    throw new CheckError('.plugin-pin plugin_root must be a relative path.');
  if (!VERSION.test(pin.plugin_version))
    throw new CheckError('.plugin-pin plugin_version must be a release version.');
  const sources = pin.reader_sources;
  if (!sources || typeof sources !== 'object' || Array.isArray(sources))
    throw new CheckError('.plugin-pin reader_sources must map paths to object IDs.');
  const entries = Object.entries(sources);
  if (entries.length === 0) throw new CheckError('.plugin-pin must list reader sources.');
  for (const [path, id] of entries) {
    if (!RELATIVE.test(path) || path.split('/').includes('..') || !OBJECT_ID.test(id))
      throw new CheckError('.plugin-pin reader_sources must map relative paths to object IDs.');
  }
  return pin;
}

export async function readPin(root) {
  let text;
  try {
    text = await readFile(resolve(root, '.plugin-pin'), 'utf8');
  } catch {
    throw new CheckError('.plugin-pin is missing.');
  }
  return parsePin(text);
}

// Resolves the pin against a checkout: HEAD and every listed reader source
// must be the exact pinned objects. `git` runs the command and returns stdout.
export function verifyCheckout(pin, git) {
  const head = git(['rev-parse', 'HEAD']).trim();
  if (head !== pin.commit) throw new CheckError('Plugin checkout is not at the pinned commit.');
  for (const [path, id] of Object.entries(pin.reader_sources)) {
    let actual;
    try {
      actual = git(['rev-parse', '--verify', `HEAD:${path}`]).trim();
    } catch {
      throw new CheckError(`Pinned reader source is absent: ${path}`);
    }
    if (actual !== id) throw new CheckError(`Pinned reader source differs: ${path}`);
  }
  if (git(['status', '--porcelain']).trim())
    throw new CheckError('Plugin checkout has local modifications.');
  return head;
}

// Freshness: the pin is reviewed, so it never moves by itself. This check
// only says whether the newest released producer still ships the same reader
// objects. It warns and never fails a release; a malformed pin still fails.
const RELEASE = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const GIT_TIMEOUT_MS = 120_000;

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1)
    if (left[index] !== right[index]) return left[index] - right[index];
  return 0;
}

// Release tags are `<plugin>--vX.Y.Z`. The highest version wins, whatever
// order the tags were made in; other plugins' tags and prereleases are ignored.
export function latestRelease(tags, plugin) {
  const prefix = `${plugin}--`;
  let best;
  for (const tag of tags) {
    if (!tag.startsWith(prefix)) continue;
    const match = RELEASE.exec(tag.slice(prefix.length));
    if (!match) continue;
    const parts = match.slice(1).map(Number);
    if (!best || compareVersions(parts, best.parts) > 0) best = { tag, parts };
  }
  if (!best) throw new CheckError(`no ${plugin} release tag was found`);
  return { tag: best.tag, version: best.parts.join('.') };
}

// `git ls-remote --tags --refs` prints `<object>\trefs/tags/<name>` lines.
export function tagsFromLsRemote(stdout) {
  return stdout
    .split('\n')
    .map((line) => line.split('\t')[1]?.trim())
    .filter((ref) => ref?.startsWith('refs/tags/'))
    .map((ref) => ref.slice('refs/tags/'.length));
}

// Object IDs of the listed paths at one revision; an absent path is left out.
export function objectsAt(git, revision, paths) {
  const commit = git(['rev-parse', '--verify', `${revision}^{commit}`]).trim();
  const found = new Map();
  for (const path of paths) {
    let id;
    try {
      id = git(['rev-parse', '--verify', '--quiet', `${commit}:${path}`]).trim();
    } catch {
      continue;
    }
    if (OBJECT_ID.test(id)) found.set(path, id);
  }
  return found;
}

export function pluginName(pin) {
  return pin.plugin_root.split('/').pop();
}

// `source` supplies `tags()` and `objects(tag, paths)`. Any lookup failure is
// reported as `unknown`, never as up to date.
export function checkFreshness(pin, source) {
  const plugin = pluginName(pin);
  const pinned = pin.plugin_version;
  const paths = Object.keys(pin.reader_sources);
  let release;
  let objects;
  try {
    release = latestRelease(source.tags(), plugin);
    objects = source.objects(release.tag, paths);
  } catch (error) {
    const reason =
      error instanceof CheckError ? error.message : `the release lookup failed (${error?.message})`;
    return { state: 'unknown', plugin, pinned, reason };
  }
  const differing = paths
    .filter((path) => objects.get(path) !== pin.reader_sources[path])
    .map((path) => ({ path, missing: !objects.has(path) }));
  return {
    state: differing.length ? 'stale' : 'current',
    plugin,
    pinned,
    latest: release.version,
    differing,
  };
}

export function describeFreshness(result) {
  const { plugin, pinned } = result;
  if (result.state === 'current')
    return `The bundled plugin readers are up to date with ${plugin} ${result.latest}, the latest release.`;
  if (result.state === 'unknown')
    return (
      `Could not check whether the bundled plugin readers (${plugin} ${pinned}) are up to date: ` +
      `${result.reason}. This is not a pass; rerun pnpm check:plugin-pin.`
    );
  const paths = result.differing
    .map(({ path, missing }) => (missing ? `${path} (no longer present)` : path))
    .join(', ');
  return (
    `The bundled plugin readers are from ${plugin} ${pinned}, but the latest release is ` +
    `${plugin} ${result.latest}. These reader paths changed: ${paths}. To update, refresh ` +
    'vendor/agent-plugins with scripts/vendor-readers.sh and update .plugin-pin.'
  );
}

// Workflow command values must escape these characters to stay on one line.
const escapeCommand = (text) =>
  text.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A');

export function reportFreshness(result, { env = process.env, log = console.log } = {}) {
  const message = describeFreshness(result);
  const warning = result.state !== 'current';
  if (warning && env.GITHUB_ACTIONS === 'true')
    log(`::warning title=Plugin readers::${escapeCommand(message)}`);
  else log(warning ? `Warning: ${message}` : message);
  if (env.GITHUB_STEP_SUMMARY) {
    try {
      appendFileSync(env.GITHUB_STEP_SUMMARY, `- ${warning ? 'Warning: ' : ''}${message}\n`);
    } catch {
      log('Warning: the result could not be added to the job summary.');
    }
  }
  return message;
}

function runGit(args, cwd) {
  try {
    return execFileSync('git', ['-c', 'credential.helper=', ...args], {
      cwd,
      encoding: 'utf8',
      env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: GIT_TIMEOUT_MS,
    });
  } catch (error) {
    const detail = String(error.stderr ?? '')
      .split('\n')
      .find((line) => line.trim());
    throw new CheckError(`git ${args[0]} failed${detail ? ` (${detail.trim()})` : ''}`);
  }
}

// A supplied checkout answers from its own tags, with no network.
export function checkoutSource(directory, git = runGit) {
  return {
    tags: () => git(['tag', '--list'], directory).split('\n').filter(Boolean),
    objects: (tag, paths) => objectsAt((args) => git(args, directory), `refs/tags/${tag}`, paths),
  };
}

// Without a checkout: list the published tags, then fetch only the chosen tag's
// commit and trees (no file contents, no credentials) into a temporary directory.
export function remoteSource(repository, git = runGit, makeDirectory = makeTemporary) {
  return {
    tags: () => tagsFromLsRemote(git(['ls-remote', '--tags', '--refs', repository])),
    objects: (tag, paths) => {
      const directory = makeDirectory();
      try {
        git(['init', '-q'], directory.path);
        git(
          [
            'fetch',
            '-q',
            '--depth',
            '1',
            '--filter=blob:none',
            '--no-tags',
            repository,
            `refs/tags/${tag}`,
          ],
          directory.path,
        );
        return objectsAt((args) => git(args, directory.path), 'FETCH_HEAD', paths);
      } finally {
        directory.remove();
      }
    },
  };
}

function makeTemporary() {
  const path = mkdtempSync(join(tmpdir(), 'xtrace-plugin-pin-'));
  return { path, remove: () => rmSync(path, { recursive: true, force: true }) };
}

const USAGE =
  'Usage: node scripts/ci/plugin-pin.mjs <repository|commit|plugin_root|plugin_version>\n' +
  '       node scripts/ci/plugin-pin.mjs freshness [--checkout AGENT_PLUGINS_CHECKOUT]';

export function parseFreshnessArgs(args) {
  if (args.length === 0) return {};
  if (args.length === 2 && args[0] === '--checkout' && args[1]) return { checkout: args[1] };
  throw new CheckError(USAGE);
}

async function main(args) {
  const [command, ...rest] = args;
  if (command === 'freshness') {
    const options = parseFreshnessArgs(rest);
    const pin = await readPin(process.cwd());
    const source = options.checkout
      ? checkoutSource(resolve(options.checkout))
      : remoteSource(pin.repository);
    console.log(
      options.checkout
        ? `Checking ${pluginName(pin)} release tags in ${resolve(options.checkout)} (run git fetch --tags there first to see new releases).`
        : `Checking ${pluginName(pin)} release tags in ${pin.repository}.`,
    );
    reportFreshness(checkFreshness(pin, source));
    return;
  }
  if (!['repository', 'commit', 'plugin_root', 'plugin_version'].includes(command) || rest.length)
    throw new CheckError(USAGE);
  console.log((await readPin(process.cwd()))[command]);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof CheckError ? error.message : 'Pin could not be read.');
    process.exitCode = 1;
  });
}
