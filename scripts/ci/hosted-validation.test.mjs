import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  baselinePaths,
  probeEnvironment,
  requireDriverResult,
  requireFreshBaseline,
  requireProbeHost,
} from './hosted-validation.mjs';

const sha = 'a'.repeat(40);
const valid = () => ({
  mode: 'probe',
  platform: 'darwin',
  arch: 'arm64',
  osVersion: '14.7.2',
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
    DEFAULT_BRANCH: 'main',
    WORKFLOW_REF: 'refs/heads/main',
    WORKFLOW_REF_TYPE: 'branch',
  },
});

test('probe requires the exact hosted OS, architecture, event and workflow/source identity', () => {
  assert.doesNotThrow(() => requireProbeHost(valid()));
  for (const change of [
    { platform: 'linux' },
    { arch: 'x64' },
    { osVersion: '15.1' },
    { head: 'b'.repeat(40) },
    { head: '' },
  ]) {
    assert.throws(() => requireProbeHost({ ...valid(), ...change }));
  }
  for (const [key, value] of Object.entries({
    GITHUB_ACTIONS: 'false',
    GITHUB_EVENT_NAME: 'push',
    RUNNER_ENVIRONMENT: 'self-hosted',
    RUNNER_OS: 'Linux',
    RUNNER_ARCH: 'X64',
    CANDIDATE_SHA: 'b'.repeat(40),
    WORKFLOW_SHA: 'b'.repeat(40),
    SOURCE_SHA: 'b'.repeat(40),
    WORKFLOW_REF: 'refs/heads/topic',
    WORKFLOW_REF_TYPE: 'tag',
    DEFAULT_BRANCH: '',
  })) {
    const input = valid();
    input.env[key] = value;
    assert.throws(() => requireProbeHost(input), key);
  }
});

test('reviewed tags require valid release syntax and the exact workflow commit', () => {
  for (const tag of ['v0.1.6', 'v0.1.6-probe.1+qa.14']) {
    const input = valid();
    Object.assign(input.env, { WORKFLOW_REF: `refs/tags/${tag}`, WORKFLOW_REF_TYPE: 'tag' });
    assert.doesNotThrow(() => requireProbeHost(input));
    input.env.WORKFLOW_SHA = 'b'.repeat(40);
    assert.throws(() => requireProbeHost(input));
  }
  for (const tag of ['v01.1.6', 'v0.1.6-01', 'probe', 'v0.1.6;echo bad']) {
    const input = valid();
    Object.assign(input.env, { WORKFLOW_REF: `refs/tags/${tag}`, WORKFLOW_REF_TYPE: 'tag' });
    assert.throws(() => requireProbeHost(input), tag);
  }
});

test('real release modes remain disabled even with a claimed identity exception', () => {
  for (const mode of ['first-launch', 'update', 'release', '', undefined]) {
    const input = valid();
    input.mode = mode;
    input.env.ALLOW_RELEASE_IDENTITY = 'true';
    assert.throws(() => requireProbeHost(input), /Only the harmless test-ID probe/);
  }
});

test('probe app environment is a small allowlist with synthetic directories and no HOME override', () => {
  const env = probeEnvironment('/owned/staging.noindex');
  assert.deepEqual(
    Object.keys(env).sort(),
    [
      'CLAUDE_CONFIG_DIR',
      'LANG',
      'PATH',
      'TMPDIR',
      'XTRACE_DATA_DIR',
      'XTRACE_NATIVE_HOME',
      'XTRACE_TELEMETRY',
    ].sort(),
  );
  assert.equal(env.PATH, '/usr/bin:/bin:/usr/sbin:/sbin');
  for (const key of ['TMPDIR', 'XTRACE_DATA_DIR', 'XTRACE_NATIVE_HOME', 'CLAUDE_CONFIG_DIR'])
    assert.ok(env[key].startsWith('/owned/staging.noindex/'));
});

test('fresh baseline rejects installed apps, old/live data, provider paths and XTrace processes', () => {
  const paths = baselinePaths('/synthetic-home');
  for (const required of [
    '/Applications/XTrace Desktop.app',
    '/Applications/XTrace.app',
    '/synthetic-home/Applications/XTrace.app',
    '/synthetic-home/Library/Application Support/ai.xtrace.app',
    '/synthetic-home/Library/Application Support/ai.xtrace.desktop',
    '/synthetic-home/.claude/local/claude',
    '/synthetic-home/.local/bin/codex',
    '/opt/homebrew/bin/claude',
    '/usr/local/bin/codex',
    '/usr/bin/claude',
    '/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex',
  ])
    assert.ok(paths.includes(required), required);
  for (const path of paths) assert.throws(() => requireFreshBaseline([path], ''));
  for (const process of [
    '42 /Applications/XTrace Desktop.app/Contents/MacOS/xtrace-desktop',
    '43 /Applications/XTrace.app/Contents/MacOS/xtrace-desktop --flag',
    '90 /tmp/xtrace-desktop',
    '90 xtrace-desktop',
  ])
    assert.throws(() => requireFreshBaseline([], process));
  assert.doesNotThrow(() => requireFreshBaseline([], '1 /sbin/launchd\n42 /usr/bin/node'));
});

test('missing-control success requires the expected timeout, not missing Accessibility or a crash', () => {
  const result = {
    status: 3,
    stdout: JSON.stringify({
      code: 3,
      reason: 'Missing control timed out without changing the fixture',
      nodes: [],
    }),
  };
  assert.equal(requireDriverResult(result, 3).code, 3);
  for (const change of [
    { status: 0 },
    { status: 4 },
    { signal: 'SIGTERM' },
    { error: new Error('timeout') },
    { stdout: '{}' },
    { stdout: 'not JSON' },
  ])
    assert.throws(() => requireDriverResult({ ...result, ...change }, 3));
});

test('positive control requires the observed action result, not a window-only exit zero', () => {
  const result = {
    status: 0,
    stdout: JSON.stringify({
      code: 0,
      reason: 'Observed window, AXPress and changed visible label',
      nodes: [],
    }),
  };
  assert.equal(requireDriverResult(result, 0).code, 0);
  assert.throws(() =>
    requireDriverResult(
      { ...result, stdout: JSON.stringify({ code: 0, reason: 'Window exists', nodes: [] }) },
      0,
    ),
  );
});

const workflow = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const jobs = Object.fromEntries(
  [
    ...workflow
      .split('\njobs:\n')[1]
      .matchAll(/^ {2}([a-z0-9-]+):\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n|$(?![\s\S]))/gm),
  ].map(([, name, body]) => [name, body]),
);

test('workflow keeps native as default and puts probe outside native validation and signing', () => {
  assert.match(workflow, /default: native/);
  assert.match(jobs.native, /if: inputs.delivery_mode == 'native' \|\| inputs.delivery_mode == ''/);
  assert.match(
    jobs.package,
    /if: \(inputs.delivery_mode == 'native' \|\| inputs.delivery_mode == ''\) && github.ref/,
  );
  assert.match(jobs.package, /needs: native/);
  assert.match(jobs['ui-capability-probe'], /if: inputs.delivery_mode == 'probe'/);
  for (const name of ['native', 'ui-capability-probe'])
    assert.match(jobs[name], /needs: validate-mode/);
  assert.doesNotMatch(
    jobs['ui-capability-probe'],
    /secrets\.|environment:|check:native|release-dmg|contents: write|continue-on-error/,
  );
  assert.match(jobs['ui-capability-probe'], /node scripts\/ci\/hosted-validation.mjs probe/);
});

test('actual mode allowlist shell accepts defined modes and rejects unknown modes', () => {
  const script = jobs['validate-mode'].split('        run: |\n')[1].replace(/^ {10}/gm, '');
  for (const mode of ['', 'native', 'probe', 'first-launch', 'update', 'anything']) {
    const result = spawnSync('bash', ['--noprofile', '--norc', '-e', '-c', script], {
      env: { PATH: '/usr/bin:/bin', DELIVERY_MODE: mode },
      encoding: 'utf8',
    });
    assert.equal(
      result.status === 0,
      ['', 'native', 'probe', 'first-launch', 'update'].includes(mode),
      mode,
    );
  }
});

test('actual probe shell guard rejects tag/source mismatches and self-hosted runners before checkout', () => {
  const script = jobs['ui-capability-probe']
    .split('        run: |\n')[1]
    .split(/\n {6}- /)[0]
    .replace(/^ {10}/gm, '');
  const defaults = { ...valid().env, DELIVERY_MODE: 'probe', PATH: '/usr/bin:/bin' };
  for (const [change, accepted] of [
    [{}, true],
    [{ WORKFLOW_REF: 'refs/tags/v0.1.6-probe.1', WORKFLOW_REF_TYPE: 'tag' }, true],
    [{ WORKFLOW_SHA: 'b'.repeat(40) }, false],
    [{ SOURCE_SHA: 'b'.repeat(40) }, false],
    [{ WORKFLOW_REF: 'refs/tags/wrong', WORKFLOW_REF_TYPE: 'tag' }, false],
    [{ RUNNER_ENVIRONMENT: 'self-hosted' }, false],
    [{ DELIVERY_MODE: 'first-launch' }, false],
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
        'uname() { echo arm64; }; sw_vers() { echo 14.7; };\n' + script,
      ],
      { env: { ...defaults, ...change }, encoding: 'utf8' },
    );
    assert.equal(result.status === 0, accepted, result.stderr);
  }
});
