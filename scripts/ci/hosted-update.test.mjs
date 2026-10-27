import { updateControl } from './fixtures/hosted-update-controls.mjs';
import { cleanupUpdate, prepareUpdateInstall } from './hosted-update-lifecycle.mjs';
import { hostedSwift, swiftPolicy } from './hosted-swift.mjs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  appInventory,
  executionBudget,
  missingReleaseValues,
  release,
  sha256,
  verifyAppInventory,
} from './hosted-release.mjs';
import {
  bundleSteps,
  previousRelease,
  requireAppIdentity,
  requireBundleStep,
  requireSameDatabase,
  requireUpdateHost,
  sentinel,
  updaterLeftovers,
  verifyPreviousDownload,
  verifyPreviousInventory,
  verifyProductionFeed,
  updateBudget,
  stopUpdateProcesses,
  xtraceEntries,
} from './hosted-update-checks.mjs';

const sha = 'a'.repeat(40);
const host = () => ({
  mode: 'update',
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
const launcher = await hostedSwift('hosted-launch');
const driver = await hostedSwift('hosted-ax');
const workflow = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const job = workflow.split('  signed-update:\n')[1];

test('update requires every hosted, identity and exact source guard', () => {
  assert.doesNotThrow(() => requireUpdateHost(host()));
  for (const change of [
    { mode: 'first-launch' },
    { allowReleaseIdentity: undefined },
    { allowReleaseIdentity: 'false' },
    { allowReleaseIdentity: true },
    { platform: 'linux' },
    { osVersion: '26.5' },
    { arch: 'x64' },
    { head: 'b'.repeat(40) },
  ])
    assert.throws(() => requireUpdateHost({ ...host(), ...change }));
  for (const key of Object.keys(host().env)) {
    const input = host();
    input.env[key] = 'invalid';
    assert.throws(() => requireUpdateHost(input), key);
  }
});

test('actual update entrypoint denies local invocation before downloads or staging', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'update-denied-'));
  try {
    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('./hosted-update.mjs', import.meta.url))],
      { cwd: temporary, env: { PATH: '/usr/bin:/bin' }, encoding: 'utf8' },
    );
    assert.equal(result.status, 1);
    const output = join(temporary, 'artifacts/hosted-update');
    assert.deepEqual(await readdir(output), ['report.json']);
    const report = JSON.parse(await readFile(join(output, 'report.json'), 'utf8'));
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

// A synthetic feed in the published shape, with pins made from its own bytes.
const syntheticFeed = (change = {}) =>
  Buffer.from(
    `${JSON.stringify(
      {
        version: release.version,
        notes: 'Synthetic notes.',
        platforms: {
          'darwin-aarch64': {
            url: `https://github.com/XTraceAI/xtrace-desktop/releases/download/${release.tag}/${release.archive}`,
            signature: 'c3ludGhldGlj',
          },
        },
        ...change,
      },
      null,
      2,
    )}\n`,
  );
const feedPins = (bytes) => ({ ...release, feedBytes: bytes.length, feedSha256: sha256(bytes) });

test('production prerequisite accepts only the pinned bytes of the candidate feed', () => {
  const bytes = syntheticFeed();
  assert.deepEqual(verifyProductionFeed(bytes, feedPins(bytes)), {
    bytes: bytes.length,
    sha256: sha256(bytes),
    version: release.version,
  });
  for (const text of [
    bytes.toString().replace(release.version, previousRelease.version),
    bytes.toString().replace('github.com', 'github.net'),
    bytes.toString().replace('signature', 'signaturf'),
    bytes.toString().replace('Synthetic', 'synthetic'),
    '{}',
    '',
    bytes.toString() + '\n',
  ])
    assert.throws(
      () => verifyProductionFeed(Buffer.from(text), feedPins(bytes)),
      /prerequisite unmet: canonical/,
    );
  const changed = Buffer.from(bytes);
  changed[changed.indexOf('signature') + 15] ^= 1;
  assert.throws(() => verifyProductionFeed(changed, feedPins(bytes)), /prerequisite unmet/);
  // Even matching pins cannot admit a feed for another version, platform or archive.
  for (const change of [
    { version: previousRelease.version },
    {
      platforms: {
        'darwin-aarch64': { url: 'https://example.invalid/a.tar.gz', signature: 'c3ludGhldGlj' },
      },
    },
    {
      platforms: {
        'darwin-aarch64': {
          url: `https://github.com/XTraceAI/xtrace-desktop/releases/download/${previousRelease.tag}/${previousRelease.archive}`,
          signature: 'c3ludGhldGlj',
        },
      },
    },
    { platforms: {} },
  ]) {
    const other = syntheticFeed(change);
    assert.throws(() => verifyProductionFeed(other, feedPins(other)), /different version/);
  }
});

test('the committed feed fixture is the pinned candidate feed once the build values are filled', async () => {
  const bytes = await readFile(new URL('./fixtures/hosted-update-latest.json', import.meta.url));
  if (missingReleaseValues().length) {
    // Until then it is a placeholder that no runner can accept.
    assert.match(JSON.parse(bytes).placeholder, /exact published .* latest\.json/);
    assert.throws(() => verifyProductionFeed(bytes), /prerequisite unmet/);
  } else assert.equal(verifyProductionFeed(bytes).version, release.version);
});

test('the update starts from distinct pinned earlier release inputs', () => {
  assert.match(previousRelease.version, /^\d+\.\d+\.\d+$/);
  assert.notEqual(previousRelease.version, release.version);
  assert.equal(previousRelease.tag, `v${previousRelease.version}`);
  assert.match(previousRelease.source, /^[a-f0-9]{40}$/);
  assert.notEqual(previousRelease.source, release.source);
  assert.equal(previousRelease.bundle, '/Applications/XTrace.app');
  assert.equal(previousRelease.bundle, release.bundle);
  assert.equal(previousRelease.archive, `XTrace-${previousRelease.version}-macos-arm64.app.tar.gz`);
  assert.ok(previousRelease.bytes > 0);
  assert.ok(previousRelease.inventoryEntries > 0);
  assert.equal(previousRelease.identifier, release.identifier);
  assert.ok(sentinel.endsWith(`across ${previousRelease.version} to ${release.version}.\n`));
});

test('old archive and both inventories reject unchecked synthetic input', async () => {
  assert.throws(() => verifyPreviousDownload(Buffer.alloc(0)));
  assert.throws(() => verifyPreviousDownload(Buffer.alloc(previousRelease.bytes)));
  const temporary = await mkdtemp(join(tmpdir(), 'update-map-'));
  try {
    const app = join(temporary, 'XTrace Desktop.app');
    await mkdir(app);
    await writeFile(join(app, 'extra'), 'synthetic');
    await assert.rejects(verifyPreviousInventory(app));
    await assert.rejects(verifyAppInventory(app));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('one rule names the app folder after restart, quit and reopen, and rejects copies', () => {
  assert.deepEqual(bundleSteps, {
    installed: ['XTrace.app'],
    restarted: ['XTrace.app'],
    quit: ['XTrace.app'],
    reopened: ['XTrace.app'],
  });
  const others = ['Safari.app', 'Xcode_16.4.app', 'Utilities'];
  assert.deepEqual(requireBundleStep('restarted', [...others, 'XTrace.app']), ['XTrace.app']);
  assert.deepEqual(requireBundleStep('quit', ['XTrace.app', ...others]), ['XTrace.app']);
  for (const [step, names] of [
    // Every phase requires only the same fixed XTrace.app folder.
    ['restarted', ['XTrace Desktop.app']],
    ['restarted', ['XTrace Desktop.app', 'XTrace.app']],
    ['restarted', []],
    // (b) after a normal quit only XTrace.app may remain.
    ['quit', ['XTrace Desktop.app']],
    ['quit', ['XTrace Desktop.app', 'XTrace.app']],
    ['quit', []],
    // (c) no backup or extraction copy may be left beside it.
    ['quit', ['XTrace.app', 'XTrace Desktop 2.app']],
    ['quit', ['XTrace.app', 'tauri_current_appAbc123']],
    ['reopened', ['XTrace.app', 'xtrace.app.backup']],
    ['unknown', ['XTrace.app']],
  ])
    assert.throws(() => requireBundleStep(step, [...others, ...names]), undefined, `${step}`);
  assert.deepEqual(xtraceEntries(['b', 'XTrace.app', 'a']), ['XTrace.app']);
  assert.deepEqual(
    updaterLeftovers(['tauri_current_appX1', 'tauri_updated_appY2', 'other', 'tauri_rpm_update']),
    ['tauri_current_appX1', 'tauri_updated_appY2'],
  );
  assert.deepEqual(updaterLeftovers(['update-sentinel.txt', 'TemporaryItems']), []);
});

test('identity, version and database checks reject a different app or a replaced database', () => {
  const plist = {
    CFBundleIdentifier: 'ai.xtrace.app',
    CFBundleShortVersionString: release.version,
    CFBundleVersion: release.version,
    CFBundleExecutable: 'xtrace-desktop',
  };
  assert.doesNotThrow(() => requireAppIdentity(plist, release.version));
  assert.throws(() => requireAppIdentity(plist, previousRelease.version));
  for (const change of [
    { CFBundleIdentifier: 'ai.xtrace.desktop' },
    { CFBundleShortVersionString: previousRelease.version },
    { CFBundleVersion: previousRelease.version },
    { CFBundleExecutable: 'XTrace' },
  ])
    assert.throws(() => requireAppIdentity({ ...plist, ...change }, release.version));
  const database = { ino: 7, dev: 1, birthtimeMs: 1000.5 };
  assert.doesNotThrow(() => requireSameDatabase(database, { ...database }, 'step'));
  for (const change of [{ ino: 8 }, { dev: 2 }, { birthtimeMs: 1001 }])
    assert.throws(
      () => requireSameDatabase(database, { ...database, ...change }, 'After quit'),
      /After quit: the synthetic database was replaced/,
    );
  assert.throws(() => requireSameDatabase(undefined, database, 'step'));
  assert.throws(() => requireSameDatabase(database, undefined, 'step'));
});

test('synthetic update in place keeps the same folder and identical candidate inventory', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'update-in-place-'));
  try {
    const applications = join(temporary, 'Applications');
    const earlier = join(applications, 'XTrace.app');
    await mkdir(join(earlier, 'Contents'), { recursive: true });
    await writeFile(join(earlier, 'Contents/Info.plist'), 'earlier');
    // The updater replaces the folder's contents under the same name.
    await rm(earlier, { recursive: true });
    await mkdir(join(earlier, 'Contents'), { recursive: true });
    await writeFile(join(earlier, 'Contents/Info.plist'), 'candidate');
    const restarted = await appInventory(earlier, 'XTrace.app');
    const values = { ...release, inventoryEntries: 3, inventorySha256: restarted.sha256 };
    assert.deepEqual(requireBundleStep('restarted', await readdir(applications)), ['XTrace.app']);
    assert.deepEqual(await verifyAppInventory(earlier, values), restarted);
    assert.deepEqual(requireBundleStep('quit', await readdir(applications)), ['XTrace.app']);
    assert.deepEqual(await verifyAppInventory(join(applications, 'XTrace.app'), values), restarted);
    // A copy left beside it fails the quit step, whatever it holds.
    await mkdir(join(applications, 'XTrace Desktop.app'));
    assert.throws(() => requireBundleStep('quit', []), /no XTrace app/);
    await assert.rejects(async () => requireBundleStep('quit', await readdir(applications)));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('update deadline has a separate nonrenewable cleanup reserve', () => {
  let clock = 0;
  const budget = executionBudget(() => clock, updateBudget);
  clock = updateBudget.workMs - 5;
  assert.equal(budget.remaining(100), 5);
  clock += 5;
  assert.throws(() => budget.remaining(1));
  budget.beginCleanup();
  assert.equal(budget.remaining(999999), updateBudget.cleanupMs);
  clock += updateBudget.cleanupMs;
  budget.beginCleanup();
  assert.throws(() => budget.remaining(1));
});

test('update observes old files, presses once, restarts in place, quits normally and reopens', async () => {
  const expected = [
    'attempt:launch',
    'earlier-launch',
    'read:original',
    'earlier-inspect',
    'update-ready',
    'databaseIdentity',
    'previousInventory',
    'earlier-arm-update',
    'attempt:press',
    'update-press',
    'earlier-adopt-update',
    'read:successor',
    'earlier-inspect',
    'updated-dashboard',
    'entries:/Applications',
    'inventory:restarted',
    'identity:updated-plist',
    'signature:updated-app',
    'data:After restart',
    'earlier-quit',
    'earlier-absent',
    'entries:/Applications',
    'inventory:afterQuit',
    'identity:after-quit-plist',
    'signature:after-quit-app',
    'quarantine:after-quit',
    'data:After quit',
    'database',
    'attempt:reopen',
    'launch',
    'read:reopen',
    'inspect',
    'reopened-dashboard',
    'quit',
    'absent',
    'entries:/Applications',
    'inventory:reopened',
    'quarantine:installed-after-exit',
    'data:After reopening',
    'database',
    'entries:/owned/tmp',
  ];
  const { calls, report, run } = updateControl();
  await run();
  assert.deepEqual(calls.slice(0, -1), expected);
  assert.ok(calls.at(-1).startsWith('entries:'));
  assert.equal(report.restartPressed, true);
  assert.equal(report.normalQuit, true);
  assert.equal(report.reopenExitConfirmed, true);
  assert.equal(report.nativeDatabaseVerified, true);
  assert.deepEqual(report.beforePressInventory, { old: true });
  assert.deepEqual(report.restarted.inventory, report.afterQuit.inventory);
});

test('prelaunch verifies feed and old archive before extraction, installation and launch', async () => {
  for (const failed of [null, 'feed', 'archive']) {
    const calls = [];
    const record = (name) => {
      calls.push(name);
      if (name === failed) throw new Error(name);
    };
    const control = updateControl({
      prepare: () =>
        prepareUpdateInstall(
          {
            report: {},
            owned: '/owned',
            output: '/output',
            feedPath: '/owned/latest.json',
            fetch: async (name) => record(name),
            checked: async (name) => record(name),
            exists: async () => false,
            markInstalled: () => record('owned'),
            appIdentity: async () => record('identity'),
            validateSignature: async () => record('signature'),
            quarantineAt: async () => record('quarantine'),
          },
          {
            readFile: async () => Buffer.from('synthetic'),
            writeFile: async () => {},
            readdir: async () => ['XTrace.app'],
            verifyProductionFeed: () => record('feed'),
            verifyPreviousDownload: () => record('archive'),
            verifyPreviousInventory: async () => record('inventory'),
          },
        ),
    });
    if (failed) await assert.rejects(control.run(), new RegExp(failed));
    else await control.run();
    const expected = [
      'production-feed',
      'feed',
      'old-download',
      'archive',
      'quarantine',
      'old-extract',
      'inventory',
      'owned',
      'install',
      'inventory',
      'identity',
      'signature',
      'quarantine',
    ];
    assert.deepEqual(calls, failed ? expected.slice(0, expected.indexOf(failed) + 1) : expected);
    assert.equal(control.calls.includes('attempt:launch'), !failed);
    if (failed) assert.deepEqual(control.calls, []);
  }
});

test('prelaunch default feed verifier refuses invalid bytes before old download or install', async () => {
  const root = await mkdtemp(join(tmpdir(), 'update-prerequisite-'));
  try {
    const feedPath = join(root, 'latest.json');
    await writeFile(feedPath, '{}');
    const calls = [];
    const control = updateControl({
      prepare: () =>
        prepareUpdateInstall({
          report: {},
          owned: root,
          output: root,
          feedPath,
          fetch: async (name) => calls.push(name),
          checked: async () => assert.fail('No extraction or installation'),
        }),
    });
    await assert.rejects(control.run(), /prerequisite unmet/);
    assert.deepEqual(calls, ['production-feed']);
    assert.deepEqual(control.calls, []);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('failed update transition or normal quit prevents after-quit acceptance, database acceptance and reopen', async () => {
  for (const failAt of ['earlier-adopt-update', 'earlier-quit', 'earlier-absent']) {
    const control = updateControl({ failAt });
    await assert.rejects(control.run(), new RegExp(failAt));
    assert.equal(control.calls.includes('inventory:afterQuit'), false);
    assert.equal(control.calls.includes('database'), false);
    assert.equal(control.calls.includes('attempt:reopen'), false);
    assert.equal(control.report.normalQuit, undefined);
  }
});

test('candidate changes after normal quit prevent database acceptance and reopening', async () => {
  const control = updateControl();
  const inventory = control.effects.verifyAppInventory;
  let reads = 0;
  control.effects.verifyAppInventory = async (bundle) => ({
    ...(await inventory(bundle)),
    phase: ++reads,
  });
  await assert.rejects(control.run(), /contents after normal quit differ/);
  assert.equal(control.report.normalQuit, true);
  assert.equal(control.calls.includes('database'), false);
  assert.equal(control.calls.includes('attempt:reopen'), false);
});

test('failed reopening, Dashboard observation or normal exit prevents completed acceptance', async () => {
  for (const failAt of ['launch', 'reopened-dashboard', 'quit', 'absent']) {
    const control = updateControl({ failAt });
    await assert.rejects(control.run(), new RegExp(failAt));
    assert.equal(control.report.normalQuit, true);
    assert.equal(control.report.reopenExitConfirmed, undefined);
    assert.equal(control.report.reopened, undefined);
    assert.equal(control.report.nativeDatabaseVerified, undefined);
    assert.equal(control.report.noLeftoverCopies, undefined);
  }
});

test('updater copies in Applications or either temporary root reject the lifecycle', async () => {
  for (const contaminated of ['/Applications', '/owned/tmp', tmpdir()]) {
    const control = updateControl();
    const entries = control.effects.readdir;
    control.effects.readdir = async (path) => [
      ...(await entries(path)),
      ...(path === contaminated ? ['tauri_updated_appOwned'] : []),
    ];
    await assert.rejects(control.run(), /expected only|Updater copies were left/);
    assert.equal(control.report.noLeftoverCopies, undefined);
  }
});

test('cleanup confirms both owners absent before unregistering or removing files', async () => {
  for (const failed of [null, 'cleanup-all-exited', 'cleanup-reopened-exited', 'unregister-app']) {
    const calls = [],
      report = {};
    const failure = await cleanupUpdate({
      report,
      launchAttempted: true,
      pressAttempted: false,
      reopenAttempted: true,
      receipt: 'original',
      successorReceipt: 'successor',
      reopenReceipt: 'reopen',
      helper: 'helper',
      environmentFile: 'environment',
      automationEnvironment: {},
      checked: async (name) => {
        calls.push(name);
        if (name === failed) throw new Error(name);
      },
      installed: true,
      staging: '/owned/staging',
      exists: async () => true,
      rm: async (path) => calls.push(`remove:${path}`),
    });
    const expected = [
      'cleanup-stop-original',
      'cleanup-all-exited',
      'cleanup-stop-reopened',
      'cleanup-reopened-exited',
      'unregister-app',
      `remove:${previousRelease.bundle}`,
      'remove:/owned/staging',
    ];
    assert.deepEqual(calls, failed ? expected.slice(0, expected.indexOf(failed) + 1) : expected);
    assert.equal(report.cleanupConfirmed, failed ? undefined : true);
    assert.equal(failure?.message, failed || undefined);
    assert.equal(report.cleanupError, failed || undefined);
  }
});

test('cleanup retains app and staging if ownership receipt or successor confirmation is missing', async () => {
  for (const kind of ['receipt', 'successor']) {
    const calls = [],
      report = {};
    const failure = await cleanupUpdate({
      report,
      launchAttempted: true,
      pressAttempted: true,
      receipt: 'original',
      successorReceipt: 'successor',
      helper: 'helper',
      exists: async (path) => kind !== 'receipt' && path === 'original',
      checked: async (name) => {
        calls.push(name);
        throw new Error('Successor cannot be confirmed');
      },
      installed: true,
      staging: '/owned/staging',
      rm: async () => assert.fail('Uncertain ownership must retain files'),
    });
    assert.ok(failure);
    assert.equal(report.cleanupConfirmed, undefined);
    assert.deepEqual(calls, kind === 'receipt' ? [] : ['cleanup-adopt', 'cleanup-stop-original']);
  }
});

test('actual process cleanup stops known owners and retains uncertain transitions on failure', async () => {
  for (const [state, failedAction, expected] of [
    [
      { pressAttempted: false, successorExists: false },
      null,
      ['stopOriginal', 'confirmAbsent', 'remove'],
    ],
    [
      { pressAttempted: true, successorExists: false },
      null,
      ['adopt', 'stopSuccessor', 'confirmAbsent', 'remove'],
    ],
    [
      { pressAttempted: true, successorExists: true },
      null,
      ['stopSuccessor', 'confirmAbsent', 'remove'],
    ],
    [{ pressAttempted: true, successorExists: false }, 'adopt', ['adopt', 'stopOriginal']],
    [{ pressAttempted: true, successorExists: true }, 'stopSuccessor', ['stopSuccessor']],
    [{ pressAttempted: false, successorExists: false }, 'stopOriginal', ['stopOriginal']],
    [
      { pressAttempted: true, successorExists: true },
      'confirmAbsent',
      ['stopSuccessor', 'confirmAbsent'],
    ],
  ]) {
    const calls = [];
    const actions = Object.fromEntries(
      ['adopt', 'stopOriginal', 'stopSuccessor', 'confirmAbsent'].map((name) => [
        name,
        async () => {
          calls.push(name);
          if (name === failedAction) throw new Error(name + ' failed');
        },
      ]),
    );
    const cleanup = async () => {
      await stopUpdateProcesses(state, actions);
      calls.push('remove');
    };
    if (failedAction) await assert.rejects(cleanup(), new RegExp(failedAction));
    else await cleanup();
    assert.deepEqual(calls, expected);
  }
  const calls = [];
  await assert.rejects(
    stopUpdateProcesses(
      { pressAttempted: true, successorExists: false },
      {
        adopt: async () => {
          calls.push('adopt');
          throw new Error('late successor uncertain');
        },
        stopOriginal: async () => {
          calls.push('stopOriginal');
          throw new Error('identity changed');
        },
        stopSuccessor: () => assert.fail('No successor receipt'),
        confirmAbsent: () => assert.fail('Uncertain transition cannot be cleared by a snapshot'),
      },
    ),
    /late successor uncertain; original stop also failed: identity changed/,
  );
  assert.deepEqual(calls, ['adopt', 'stopOriginal']);
});

test('update entrypoint excludes broad process termination and command/credential overrides', async () => {
  const source = await readFile(new URL('./hosted-update.mjs', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /killall|pkill|\/usr\/bin\/open|TAURI_CONFIG|GITHUB_TOKEN|GH_TOKEN/);
});

test('update workflow leaves cleanup/upload time and denies wrong authorization before checkout', () => {
  assert.doesNotMatch(
    job,
    /secrets\.|environment:|contents: write|continue-on-error|release-dmg.mjs/,
  );
  assert.match(job, /if: always\(\)/);
  const minutes = [...job.matchAll(/^ {8}timeout-minutes: (\d+)$/gm)].map((m) => Number(m[1]));
  assert.ok(minutes.every((value) => value > 0));

  assert.ok(
    minutes.reduce((a, b) => a + b, 0) < Number(job.match(/^ {4}timeout-minutes: (\d+)$/m)[1]),
  );
  assert.ok((updateBudget.workMs + updateBudget.cleanupMs) / 60000 < minutes[4]);
  const script = job
    .split('        run: |\n')[1]
    .split(/\n {6}- /)[0]
    .replace(/^ {10}/gm, '');
  for (const change of [
    {},
    { ALLOW_RELEASE_IDENTITY: 'false' },
    { DELIVERY_MODE: 'first-launch' },
    { RUNNER_ENVIRONMENT: 'self-hosted' },
    { WORKFLOW_SHA: 'b'.repeat(40) },
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
          DELIVERY_MODE: 'update',
          ALLOW_RELEASE_IDENTITY: 'true',
          ...change,
        },
        encoding: 'utf8',
      },
    );
    assert.equal(result.status === 0, Object.keys(change).length === 0, result.stderr);
  }
});

test('first-launch and update checks share the scoped displayed-version predicate', () => {
  assert.equal((driver.match(/displayedVersion\(last, Release\.candidate\)/g) || []).length, 3);
  assert.equal((driver.match(/displayedVersion\(last, Release\.earlier\)/g) || []).length, 1);
  assert.match(driver, /displayedVersion\(rows, version\)/);
  assert.doesNotMatch(driver, /hasText\([^\n]*"v(?:0\.1\.[678]|" \+ version)/);
  // Versions and folders are written once in each Swift helper, and agree with the runners.
  assert.doesNotMatch(
    driver.replace(/enum Release \{[\s\S]*?\n\}/, ''),
    /0\.1\.\d|XTrace(?: Desktop)?\.app/,
  );
  assert.match(driver, new RegExp(`static let candidate = "${release.version}"`));
  assert.match(driver, new RegExp(`static let earlier = "${previousRelease.version}"`));
  assert.match(driver, new RegExp(`static let candidateBundle = "${release.bundle}"`));
  assert.match(driver, new RegExp(`static let earlierBundle = "${previousRelease.bundle}"`));
  const product = launcher.match(/enum ProductBundle \{[\s\S]*?\n\}/)[0];
  assert.match(product, new RegExp(`static let earlier = "${previousRelease.bundle}"`));
  assert.match(product, new RegExp(`static let current = "${release.bundle}"`));
  assert.doesNotMatch(
    launcher.replace(product, '').replace(/^\/\/.*$/gm, ''),
    /\/Applications\/XTrace/,
  );
});

// Compile the actual pure Swift decisions, not a JS copy of those decisions.
// This never runs the launch helper or queries a process/Accessibility API.
test(
  'Swift ownership and rendered-UI negative controls use the production predicates',
  { skip: process.platform !== 'darwin', timeout: 120000 },
  async () => {
    const temporary = await mkdtemp(join(tmpdir(), 'update-swift-policy-'));
    try {
      const receipt = await swiftPolicy('RECEIPT');
      const transition = await swiftPolicy('TRANSITION POLICY');
      const ui = await swiftPolicy('UPDATE UI POLICY');
      const controls = `
func record(_ pid: Int32, _ start: UInt64, _ id: String = "ai.xtrace.app", _ path: String = ProductBundle.earlier) -> Receipt {
  Receipt(pid: pid, startTime: start, bundle: path, executable: path + "/Contents/MacOS/xtrace-desktop", identifier: id)
}
let old = record(10, 100), next = record(11, 201)
let t = Transition(original: old, armedAt: 200)
assert(try! successor(t, [old], originalAlive: true) == nil)
assert(try! successor(t, [old, next], originalAlive: true) == nil)
assert(try! successor(t, [], originalAlive: false) == nil)
assert(try! successor(t, [next], originalAlive: false)?.pid == 11)
// Original absent from AppKit candidates is still alive if its kernel identity exists.
assert(try! successor(t, [next], originalAlive: kernelIsAlive(expected: 100, observed: 100, missingProcess: false)) == nil)
assert(try! !kernelIsAlive(expected: 100, observed: nil, missingProcess: true))
// The actual candidate-start decision drops only kernel-confirmed stale entries.
func retainedCandidates(_ observations: [(Receipt, UInt64?, Bool)]) throws -> [Receipt] {
  try observations.compactMap { receipt, observed, missing in
    guard try liveCandidateStart(observed, missingProcess: missing) != nil else { return nil }
    return receipt
  }
}
let afterExit = try! retainedCandidates([(old, nil, true), (next, 201, false)])
assert(afterExit.count == 1)
assert(try! successor(t, afterExit, originalAlive: false)?.pid == next.pid)
let staleOnly = try! retainedCandidates([(old, nil, true)])
assert(staleOnly.isEmpty) // The same empty candidate list is used by the absent check.
assert(try! successor(t, staleOnly, originalAlive: false) == nil)
for (observed, missing): (UInt64?, Bool) in [(nil, false), (0, false), (0, true)] {
  do { _ = try retainedCandidates([(old, observed, missing)]); fatalError("Unknown or invalid live candidate was skipped") } catch {}
}
for (observed, missing): (UInt64?, Bool) in [(nil, false), (101, false), (101, true)] {
  do { _ = try kernelIsAlive(expected: 100, observed: observed, missingProcess: missing); fatalError("Lost kernel identity accepted") } catch {}
}
for bad in [[record(10, 201)], [record(11, 199)], [next, record(12, 202)], [next, next],
            [record(11, 201, "ai.xtrace.desktop")], [record(11, 201, "ai.xtrace.app", "/other")],
            // A successor at any other folder must not be adopted.
            [record(11, 201, "ai.xtrace.app", "/Applications/Other.app")],
            [Receipt(pid: 11, startTime: 201, bundle: old.bundle, executable: "/wrong", identifier: old.identifier)]] {
  do { _ = try successor(t, bad, originalAlive: false); fatalError("Accepted invalid successor") } catch {}
}
// No transition may start outside the fixed installation folder.
let currentOriginal = Transition(original: record(10, 100, "ai.xtrace.app", "/Applications/Other.app"), armedAt: 200)
do { _ = try successor(currentOriginal, [], originalAlive: true); fatalError("Accepted a transition outside the earlier folder") } catch {}
func text(_ value: String) -> [String: String] { [kAXValueAttribute: value] }
func versionRow(_ value: String, _ parent: String = "footer", _ position: Int = 0,
                _ workspace: Bool = true) -> [String: String] {
  [kAXRoleAttribute: "AXStaticText", kAXValueAttribute: value, "captureParent": parent,
   "captureChildIndex": String(position), "captureWorkspace": workspace ? "true" : "false"]
}
// Synthetic rows exercise split version labels and the current release relationship.
assert(Release.candidate == "${release.version}" && Release.earlier == "${previousRelease.version}")
assert(Release.bundle("update-ready") == "/Applications/XTrace.app")
assert(Release.bundle("update-press") == "/Applications/XTrace.app")
assert(Release.bundle("updated-dashboard") == "/Applications/XTrace.app")
for mode in ["reopened-dashboard", "first-launch", "telemetry-notice", "unknown"] {
  assert(Release.bundle(mode) == "/Applications/XTrace.app")
}
let splitVersion = [versionRow("v", "footer", 4), versionRow("${release.version}", "footer", 5)]
assert(displayedVersion(splitVersion, "${release.version}"))
assert(displayedVersion([versionRow("v${release.version}")], "${release.version}"))
for invalid in [
  [versionRow("v", "footer", 4), versionRow("${release.version}", "other-pane", 5)],
  [versionRow("v", "footer", 4), versionRow("${release.version}", "footer", 6)],
  [versionRow("${release.version}", "footer", 4), versionRow("v", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("${previousRelease.version}", "footer", 5)],
  [versionRow("v", "footer", 4, false), versionRow("${release.version}", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("${release.version}", "footer", 5, false)],
  [versionRow("v${release.version}", "main-pane", 0, false)],
  [versionRow("Update ${release.version} is ready. Your local data stays on this Mac.")],
  [versionRow("v${previousRelease.version}"), versionRow("${release.version}", "update-banner", 1)],
  [text("v${release.version}")],
  [[kAXRoleAttribute: "AXButton", kAXValueAttribute: "v${release.version}", "captureWorkspace": "true"]]
] { assert(!displayedVersion(invalid, "${release.version}")) }
let rows: [[String: String]] = [[kAXRoleAttribute: kAXWindowRole], [kAXRoleAttribute: "AXWebArea"],
  versionRow("v${previousRelease.version}"), text("What your agents did"),
  text("No agent activity was recorded in this range. Measured zeros are shown as 0."),
  text("Update ${release.version} is ready. Your local data stays on this Mac."),
  [kAXRoleAttribute: kAXButtonRole, kAXTitleAttribute: "Restart to update"]]
assert(updateReady(rows))
for index in rows.indices { assert(!updateReady(Array(rows.enumerated().filter { $0.offset != index }.map { $0.element }))) }
assert(!updateReady(rows + [rows.last!]))
assert(!updateReady(rows + [text("Welcome to XTrace")]))
assert(!updateReady(rows + [[kAXRoleAttribute: "AXAlert"]]))
assert(!updateReady(rows + [[kAXSubroleAttribute: "AXDialog"]]))
assert(!updateReady(rows + [text("Dashboard metrics could not be loaded.")]))
let updated = rows.map { $0 == versionRow("v${previousRelease.version}") ? versionRow("v${release.version}") : $0 }
assert(!updateReady(updated)); assert(updateDashboard(updated, "${release.version}"))
let splitDashboard = updated.filter { $0 != versionRow("v${release.version}") } + splitVersion
assert(updateDashboard(splitDashboard, "${release.version}"))
let splitOldDashboard = rows.filter { $0 != versionRow("v${previousRelease.version}") } +
    [versionRow("v", "footer", 4), versionRow("${previousRelease.version}", "footer", 5)]
assert(updateReady(splitOldDashboard))
// After the restart and after reopening XTrace.app: Dashboard, never Welcome.
assert(!updateDashboard(updated + [text("Welcome to XTrace")], "${release.version}"))
assert(!updateDashboard(rows, "${release.version}"))
let wrongTarget = rows.map { $0 == rows[5] ? text("Update 99.98.97 is ready. Your local data stays on this Mac.") : $0 }
assert(!updateReady(wrongTarget))
let earlierTarget = rows.map { $0 == rows[5] ? text("Update ${previousRelease.version} is ready. Your local data stays on this Mac.") : $0 }
assert(!updateReady(earlierTarget))
print("Swift transition and update UI controls passed")
`;
      await writeFile(
        join(temporary, 'main.swift'),
        'import AppKit\nimport ApplicationServices\n' + receipt + transition + ui + controls,
      );
      const binary = join(temporary, 'controls');
      const result = spawnSync(
        '/usr/bin/xcrun',
        [
          'swiftc',
          '-module-cache-path',
          join(temporary, 'cache'),
          join(temporary, 'main.swift'),
          '-o',
          binary,
        ],
        { encoding: 'utf8', timeout: 100000 },
      );
      assert.equal(result.status, 0, result.stderr);
      const ran = spawnSync(binary, [], { encoding: 'utf8', timeout: 10000 });
      assert.equal(ran.status, 0, ran.stderr);
      assert.match(ran.stdout, /controls passed/);
    } finally {
      await rm(temporary, { recursive: true, force: true });
    }
  },
);
