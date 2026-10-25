import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  release,
  requireFirstLaunchHost,
  firstLaunchEnvironment,
  firstLaunchBaselinePaths,
  appInventory,
  verifyAppInventory,
  missingReleaseValues,
  releaseValueNames,
  requireReleaseValues,
  sha256,
  verifyDownload,
  verifyTag,
  executionBudget,
  firstLaunchBudget,
} from './hosted-release.mjs';

const sha = 'a'.repeat(40);
const host = () => ({
  mode: 'first-launch',
  allowReleaseIdentity: 'true',
  platform: 'darwin',
  arch: 'arm64',
  osVersion: '14.8.9',
  head: sha,
  env: {
    GITHUB_ACTIONS: 'true',
    GITHUB_EVENT_NAME: 'workflow_dispatch',
    RUNNER_ENVIRONMENT: 'github-hosted',
    RUNNER_OS: 'macOS',
    RUNNER_ARCH: 'ARM64',
    CANDIDATE_SHA: sha,
    WORKFLOW_SHA: sha,
    SOURCE_SHA: sha,
    WORKFLOW_REF: 'refs/heads/main',
    WORKFLOW_REF_TYPE: 'branch',
    DEFAULT_BRANCH: 'main',
  },
});

test('first launch requires explicit true plus every hosted-source guard; update never enables', () => {
  assert.doesNotThrow(() => requireFirstLaunchHost(host()));
  for (const change of [
    { allowReleaseIdentity: undefined },
    { allowReleaseIdentity: false },
    { allowReleaseIdentity: 'false' },
    { allowReleaseIdentity: '1' },
    { mode: 'update' },
    { mode: 'probe' },
    { platform: 'linux' },
    { arch: 'x64' },
    { osVersion: '26.5' },
    { head: 'b'.repeat(40) },
  ])
    assert.throws(() => requireFirstLaunchHost({ ...host(), ...change }));
  for (const [key, value] of Object.entries({
    GITHUB_EVENT_NAME: 'push',
    RUNNER_ENVIRONMENT: 'self-hosted',
    GITHUB_ACTIONS: 'false',
    WORKFLOW_SHA: 'b'.repeat(40),
    WORKFLOW_REF: 'refs/heads/topic',
  })) {
    const input = host();
    input.env[key] = value;
    assert.throws(() => requireFirstLaunchHost(input), key);
  }
});

test('actual first-launch entrypoint fails without opt-in before staging or app commands', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'first-launch-denied-'));
  try {
    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('./hosted-first-launch.mjs', import.meta.url))],
      { cwd: temporary, env: { PATH: '/usr/bin:/bin' }, encoding: 'utf8' },
    );
    assert.equal(result.status, 1);
    const output = join(temporary, 'artifacts/hosted-first-launch');
    assert.deepEqual(await readdir(output), ['report.json']);
    const report = JSON.parse(await readFile(join(output, 'report.json'), 'utf8'));
    assert.equal(report.status, 'failed');
    assert.equal(report.releasedAppLaunched, false);
    assert.deepEqual(report.validationLimits, {
      launchConfiguration: 'synthetic data with telemetry disabled',
      defaultTelemetryNoticeValidated: false,
      defaultTelemetryBehaviorValidated: false,
    });
    assert.match(report.error, /explicit hosted real-identity exception/);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('first launch adds fresh provider, preference and current/old WebKit baselines', () => {
  const paths = firstLaunchBaselinePaths('/home/runner');
  for (const path of [
    '.claude',
    '.claude.json',
    '.codex',
    '.cursor',
    '.config/gh',
    'Library/WebKit/ai.xtrace.app',
    'Library/WebKit/ai.xtrace.desktop',
    'Library/Preferences/ai.xtrace.app.plist',
    'Library/Caches/ai.xtrace.app',
    'Library/Application Support/Cursor',
  ])
    assert.ok(paths.includes(join('/home/runner', path)), path);
  const env = firstLaunchEnvironment('/owned');
  assert.equal(env.GH_CONFIG_DIR, '/owned/gh');
  assert.equal(env.SSH_AUTH_SOCK, '');
  assert.equal(env.XTRACE_DATA_DIR, '/owned/data');
  assert.equal(env.XTRACE_NATIVE_HOME, '/owned/home');
  assert.equal(env.CLAUDE_CONFIG_DIR, '/owned/claude');
  assert.equal(env.XTRACE_TELEMETRY, '0');
  for (const key of [
    'HOME',
    'GITHUB_TOKEN',
    'GH_TOKEN',
    'OPENAI_API_KEY',
    'ANTHROPIC_API_KEY',
    'XTRACE_GH',
  ])
    assert.equal(key in env, false);
});

// Synthetic stand-ins for the signed build values; never the real ones.
const syntheticValues = () => ({
  ...release,
  bytes: 7,
  sha256: sha256('content'),
  inventoryEntries: 2,
  inventorySha256: 'b'.repeat(64),
  archiveBytes: 1,
  archiveSha256: 'c'.repeat(64),
  signatureBytes: 1,
  signatureSha256: 'd'.repeat(64),
  feedBytes: 1,
  feedSha256: 'e'.repeat(64),
});

test('the candidate release values describe one source and matching artifact names', () => {
  assert.match(release.version, /^\d+\.\d+\.\d+$/);
  assert.equal(release.tag, `v${release.version}`);
  assert.match(release.source, /^[a-f0-9]{40}$/);
  assert.equal(release.bundle, '/Applications/XTrace.app');
  assert.equal(release.identifier, 'ai.xtrace.app');
  assert.equal(release.dmg, `XTrace-${release.version}-macos-arm64.dmg`);
  assert.equal(release.archive, `XTrace-${release.version}-macos-arm64.app.tar.gz`);
  assert.ok(Object.isFrozen(release));
});

test('build values: every unfilled, malformed or missing value is named and refused', () => {
  const values = syntheticValues();
  assert.deepEqual(missingReleaseValues(values), []);
  assert.doesNotThrow(() => requireReleaseValues(values));
  assert.deepEqual(releaseValueNames.length, 10);
  for (const name of releaseValueNames)
    for (const bad of [null, undefined, 0, -1, 1.5, '1', 'A'.repeat(64), 'a'.repeat(63), '']) {
      if (typeof values[name] === 'string' && typeof bad === 'number' && bad > 0) continue;
      const changed = { ...values, [name]: bad };
      assert.deepEqual(missingReleaseValues(changed), [name], `${name}=${bad}`);
      assert.throws(() => requireReleaseValues(changed), new RegExp(`not filled.*${name}`));
    }
});

test('the committed build values are either all filled or the runners refuse', () => {
  const missing = missingReleaseValues();
  if (missing.length) {
    // Before the coordinator fills them from the signed build: all ten unset.
    assert.deepEqual(missing, [...releaseValueNames]);
    assert.throws(() => requireReleaseValues(), /Signed 0\.1\.8 build values are not filled/);
  } else assert.doesNotThrow(() => requireReleaseValues());
});

test('download and lightweight-tag checks reject different bytes, size or source identity', async () => {
  assert.throws(() => verifyDownload(Buffer.alloc(0)));
  assert.throws(() => verifyDownload(Buffer.from('content')), 'unfilled or different real pins');
  const values = syntheticValues();
  assert.doesNotThrow(() => verifyDownload(Buffer.from('content'), values));
  for (const changed of ['contenT', 'content\n', 'conten'])
    assert.throws(() => verifyDownload(Buffer.from(changed), values));
  const tag = { ref: `refs/tags/${release.tag}`, object: { type: 'commit', sha: release.source } };
  assert.equal(
    await verifyTag(tag, () => assert.fail('Lightweight tags need no lookup')),
    release.source,
  );
  for (const changed of [
    { ...tag, ref: `refs/tags/${release.tag}-unrelated` },
    { ...tag, object: { type: 'commit', sha } },
    { ...tag, object: { type: 'tree', sha: release.source } },
    {},
  ])
    await assert.rejects(verifyTag(changed));
});

test('synthetic annotated release tag resolves once through a fixed repository URL', async () => {
  const annotationSHA = 'c'.repeat(40);
  const ref = {
    ref: `refs/tags/${release.tag}`,
    object: { type: 'tag', sha: annotationSHA, url: 'https://untrusted.invalid/tag' },
  };
  const annotation = {
    sha: annotationSHA,
    tag: release.tag,
    object: { type: 'commit', sha: release.source, url: 'https://untrusted.invalid/commit' },
  };
  const requests = [];
  assert.equal(
    await verifyTag(ref, async (url) => {
      requests.push(url);
      return annotation;
    }),
    release.source,
  );
  assert.deepEqual(requests, [
    `https://api.github.com/repos/XTraceAI/xtrace-desktop/git/tags/${annotationSHA}`,
  ]);
  for (const changed of [
    { ...ref, ref: `refs/tags/${release.tag}-unrelated` },
    { ...ref, object: { type: 'tree', sha: annotationSHA } },
    { ...ref, object: { type: 'tag', sha: '../other-repository' } },
    { ...ref, object: { type: 'tag', sha: 'A'.repeat(40) } },
  ])
    await assert.rejects(verifyTag(changed, () => assert.fail('Invalid refs must not be fetched')));
  for (const changed of [
    { ...annotation, sha: 'b'.repeat(40) },
    { ...annotation, tag: `${release.tag}-unrelated` },
    { ...annotation, object: { type: 'commit', sha: 'b'.repeat(40) } },
    { ...annotation, object: { type: 'tag', sha: annotationSHA } },
    { ...annotation, object: { type: 'blob', sha: release.source } },
    {},
  ]) {
    let calls = 0;
    await assert.rejects(
      verifyTag(ref, async () => {
        calls++;
        return changed;
      }),
    );
    assert.equal(calls, 1, 'Nested or invalid annotations must not trigger another lookup');
  }
  await assert.rejects(
    verifyTag(ref, async () => {
      throw new Error('API unavailable');
    }),
    /API unavailable/,
  );
});

test('canonical inventory binds every path, byte, size, directory and mode and rejects links', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'hosted-inventory-'));
  try {
    const app = join(temporary, 'XTrace Desktop.app');
    await mkdir(app, { mode: 0o755 });
    await chmod(app, 0o755);
    const file = join(app, 'file');
    await writeFile(file, 'content');
    await chmod(file, 0o644);
    const original = await appInventory(app);
    assert.deepEqual(original, {
      entries: 2,
      sha256: sha256(
        JSON.stringify([
          ['XTrace Desktop.app', true, 0, 0o755, null],
          ['XTrace Desktop.app/file', false, 7, 0o644, sha256('content')],
        ]),
      ),
    });
    await assert.rejects(verifyAppInventory(app), /differ/);
    await writeFile(file, 'changed');
    assert.notEqual((await appInventory(app)).sha256, original.sha256);
    await writeFile(file, 'content');
    await chmod(file, 0o755);
    assert.notEqual((await appInventory(app)).sha256, original.sha256);
    await chmod(file, 0o644);
    await rename(file, join(app, 'renamed'));
    assert.notEqual((await appInventory(app)).sha256, original.sha256);
    await mkdir(join(app, 'extra'));
    assert.equal((await appInventory(app)).entries, 3);
    await symlink('/outside', join(app, 'link'));
    await assert.rejects(appInventory(app), /link or special/);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('the candidate inventory names paths after XTrace.app wherever the folder is', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'hosted-inventory-root-'));
  try {
    const earlier = join(temporary, 'XTrace Desktop.app');
    await mkdir(earlier, { mode: 0o755 });
    await chmod(earlier, 0o755);
    await writeFile(join(earlier, 'file'), 'content');
    await chmod(join(earlier, 'file'), 0o644);
    const labelled = await appInventory(earlier, 'XTrace.app');
    assert.equal(
      labelled.sha256,
      sha256(
        JSON.stringify([
          ['XTrace.app', true, 0, 0o755, null],
          ['XTrace.app/file', false, 7, 0o644, sha256('content')],
        ]),
      ),
    );
    const values = { ...syntheticValues(), inventorySha256: labelled.sha256 };
    // The updater writes the candidate into the earlier folder; a quit renames it.
    assert.deepEqual(await verifyAppInventory(earlier, values), labelled);
    const renamed = join(temporary, 'XTrace.app');
    await rename(earlier, renamed);
    assert.deepEqual(await appInventory(renamed), labelled);
    assert.deepEqual(await verifyAppInventory(renamed, values), labelled);
    await writeFile(join(renamed, 'file'), 'changed');
    await assert.rejects(verifyAppInventory(renamed, values), /differ/);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

const workflow = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const job = workflow.split('  signed-first-launch:\n')[1].split('  signed-update:\n')[0];

test('work deadlines cap remaining commands and reserve cleanup independently without resets', () => {
  let clock = 0;
  const budget = executionBudget(() => clock);
  assert.equal(budget.remaining(30_000), 30_000);
  clock = firstLaunchBudget.workMs - 120;
  assert.equal(budget.remaining(30_000), 120);
  clock += 120;
  assert.throws(() => budget.remaining(1), /work time budget exhausted/);
  budget.beginCleanup();
  assert.equal(budget.remaining(200_000), firstLaunchBudget.cleanupMs);
  clock += firstLaunchBudget.cleanupMs - 20;
  budget.beginCleanup();
  assert.equal(budget.remaining(30_000), 20);
  clock += 20;
  assert.throws(() => budget.remaining(1), /Cleanup time budget exhausted/);
});

test('workflow step budgets leave time for cleanup and evidence before the job ceiling', () => {
  for (const telemetry of [false, true]) {
    const selected = job.replace(
      /\$\{\{ inputs.delivery_mode == 'telemetry' && (\d+) \|\| (\d+) \}\}/g,
      (_, yes, no) => (telemetry ? yes : no),
    );
    const stepMinutes = [...selected.matchAll(/^ {8}timeout-minutes: (\d+)$/gm)].map((m) =>
      Number(m[1]),
    );
    const jobMinutes = Number(selected.match(/^ {4}timeout-minutes: (\d+)$/m)[1]);
    const appMinutes = stepMinutes.at(-2);
    assert.ok(stepMinutes.every((minutes) => minutes > 0));
    assert.ok(stepMinutes.reduce((sum, minutes) => sum + minutes, 0) < jobMinutes);
    assert.ok((firstLaunchBudget.workMs + firstLaunchBudget.cleanupMs) / 60000 < appMinutes);
  }
});

test('signed first-launch job needs explicit opt-in before checkout and receives no signing credentials', () => {
  assert.match(workflow, /allow_release_identity:\n[\s\S]*?default: false\n {8}type: boolean/);
  assert.match(job, /if: inputs.delivery_mode == 'first-launch'/);
  assert.match(job, /needs: validate-mode/);
  assert.doesNotMatch(
    job,
    /secrets\.|environment:|contents: write|release-dmg.mjs|check:native|continue-on-error/,
  );
  assert.match(job, /node scripts\/ci\/hosted-first-launch.mjs/);
  assert.ok(
    job.indexOf('test "$ALLOW_RELEASE_IDENTITY" = true') < job.indexOf('uses: actions/checkout@'),
  );
  const script = job
    .split('        run: |\n')[1]
    .split(/\n {6}- /)[0]
    .replace(/^ {10}/gm, '');
  for (const [change, expected] of [
    [{}, true],
    [{ ALLOW_RELEASE_IDENTITY: 'false' }, false],
    [{ ALLOW_RELEASE_IDENTITY: '' }, false],
    [{ DELIVERY_MODE: 'telemetry' }, true],
    [{ DELIVERY_MODE: 'update' }, false],
    [{ RUNNER_ENVIRONMENT: 'self-hosted' }, false],
    [{ WORKFLOW_SHA: 'b'.repeat(40) }, false],
  ]) {
    const result = spawnSync(
      'bash',
      [
        '--noprofile',
        '--norc',
        '-e',
        '-o',
        'pipefail',
        '-c',
        'uname() { echo arm64; }; sw_vers() { echo 14.8.9; };\n' + script,
      ],
      {
        env: {
          ...host().env,
          PATH: '/usr/bin:/bin',
          DELIVERY_MODE: 'first-launch',
          ALLOW_RELEASE_IDENTITY: 'true',
          ...change,
        },
        encoding: 'utf8',
      },
    );
    assert.equal(result.status === 0, expected, result.stderr);
  }
});
