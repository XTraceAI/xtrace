import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
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
const runner = await readFile(new URL('./hosted-update.mjs', import.meta.url), 'utf8');
const launcher = await readFile(new URL('./hosted-launch.swift', import.meta.url), 'utf8');
const driver = await readFile(new URL('./hosted-ax.swift', import.meta.url), 'utf8');
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
    version: '0.1.8',
  });
  for (const text of [
    bytes.toString().replace('0.1.8', '0.1.7'),
    bytes.toString().replace('github.com', 'github.net'),
    bytes.toString().replace('signature', 'signaturf'),
    bytes.toString().replace('Synthetic', 'synthetic'),
    '{}',
    '',
    bytes.toString() + '\n',
  ])
    assert.throws(
      () => verifyProductionFeed(Buffer.from(text), feedPins(bytes)),
      /prerequisite unmet: canonical 0\.1\.8/,
    );
  const changed = Buffer.from(bytes);
  changed[changed.indexOf('signature') + 15] ^= 1;
  assert.throws(() => verifyProductionFeed(changed, feedPins(bytes)), /prerequisite unmet/);
  // Even matching pins cannot admit a feed for another version, platform or archive.
  for (const change of [
    { version: '0.1.7' },
    {
      platforms: {
        'darwin-aarch64': { url: 'https://example.invalid/a.tar.gz', signature: 'c3ludGhldGlj' },
      },
    },
    {
      platforms: {
        'darwin-aarch64': {
          url: `https://github.com/XTraceAI/xtrace-desktop/releases/download/v0.1.7/${previousRelease.archive}`,
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
    assert.match(JSON.parse(bytes).placeholder, /exact published 0\.1\.8 latest\.json/);
    assert.throws(() => verifyProductionFeed(bytes), /prerequisite unmet/);
  } else assert.equal(verifyProductionFeed(bytes).version, release.version);
});

test('the update starts from the published 0.1.7 in its earlier folder', () => {
  assert.equal(previousRelease.version, '0.1.7');
  assert.equal(previousRelease.tag, 'v0.1.7');
  assert.equal(previousRelease.source, 'e743e900fb40d32bdbe90709bd54b7d9bde32540');
  assert.equal(previousRelease.bundle, '/Applications/XTrace Desktop.app');
  assert.equal(previousRelease.archive, 'XTrace-Desktop-0.1.7-macos-arm64.app.tar.gz');
  assert.equal(previousRelease.bytes, 11064049);
  assert.equal(previousRelease.inventoryEntries, 79);
  assert.equal(previousRelease.identifier, release.identifier);
  assert.match(sentinel, /across 0\.1\.7 to 0\.1\.8\.\n$/);
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
    installed: ['XTrace Desktop.app'],
    restarted: ['XTrace Desktop.app'],
    quit: ['XTrace.app'],
    reopened: ['XTrace.app'],
  });
  const others = ['Safari.app', 'Xcode_16.4.app', 'Utilities'];
  assert.deepEqual(requireBundleStep('restarted', [...others, 'XTrace Desktop.app']), [
    'XTrace Desktop.app',
  ]);
  assert.deepEqual(requireBundleStep('quit', ['XTrace.app', ...others]), ['XTrace.app']);
  for (const [step, names] of [
    // (a) after restart the folder must keep its earlier name; no XTrace.app yet.
    ['restarted', ['XTrace.app']],
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
    CFBundleShortVersionString: '0.1.8',
    CFBundleVersion: '0.1.8',
    CFBundleExecutable: 'xtrace-desktop',
  };
  assert.doesNotThrow(() => requireAppIdentity(plist, release.version));
  assert.throws(() => requireAppIdentity(plist, previousRelease.version));
  for (const change of [
    { CFBundleIdentifier: 'ai.xtrace.desktop' },
    { CFBundleShortVersionString: '0.1.7' },
    { CFBundleVersion: '0.1.7' },
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

test('synthetic update in place, then rename on quit, keeps one identical inventory', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'update-rename-'));
  try {
    const applications = join(temporary, 'Applications');
    const earlier = join(applications, 'XTrace Desktop.app');
    await mkdir(join(earlier, 'Contents'), { recursive: true });
    await writeFile(join(earlier, 'Contents/Info.plist'), 'earlier');
    // The updater replaces the folder's contents under the same name.
    await rm(earlier, { recursive: true });
    await mkdir(join(earlier, 'Contents'), { recursive: true });
    await writeFile(join(earlier, 'Contents/Info.plist'), 'candidate');
    const restarted = await appInventory(earlier, 'XTrace.app');
    const values = { ...release, inventoryEntries: 3, inventorySha256: restarted.sha256 };
    assert.deepEqual(requireBundleStep('restarted', await readdir(applications)), [
      'XTrace Desktop.app',
    ]);
    assert.deepEqual(await verifyAppInventory(earlier, values), restarted);
    await rename(earlier, join(applications, 'XTrace.app'));
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

test('update ordering: old files before one press, restart in place, normal quit renames, reopen without Welcome', () => {
  const ordered = [
    'requireReleaseValues()',
    'verifyProductionFeed(await',
    'verifyPreviousDownload(await',
    "'old-extract'",
    "requireBundleStep('installed'",
    "['earlier-launch', receipt",
    "'update-ready',",
    'report.databaseIdentity = await databaseIdentity()',
    'report.beforePressInventory',
    "['earlier-arm-update', receipt",
    'pressAttempted = true',
    "'update-press',",
    "['earlier-adopt-update', receipt",
    "'successor-environment'",
    "'updated-dashboard',",
    "requireBundleStep('restarted'",
    'verifyAppInventory(earlierBundle)',
    "validateSignature(earlierBundle, 'updated-app')",
    "dataKept('After restart')",
    "['earlier-quit', successorReceipt",
    "['earlier-absent', receipt",
    'report.normalQuit = true',
    "requireBundleStep('quit'",
    'verifyAppInventory(renamedBundle)',
    'Renamed app contents differ',
    "validateSignature(renamedBundle, 'renamed-app')",
    "dataKept('After quit')",
    'report.databaseAfterUpdate = await verifyHostedDatabase(',
    "['launch', reopenReceipt",
    "['inspect', reopenReceipt",
    "'reopened-dashboard',",
    "['quit', reopenReceipt",
    "['absent', reopenReceipt",
    "requireBundleStep('reopened'",
    "dataKept('After reopening')",
    'report.database = await verifyHostedDatabase(',
    'updaterLeftovers(await readdir(env.TMPDIR))',
    'updaterLeftovers(await readdir(tmpdir()))',
  ];
  let prior = -1;
  for (const needle of ordered) {
    const at = runner.indexOf(needle);
    assert.ok(at > prior, needle);
    prior = at;
  }
  // One launch of each folder: the earlier app once, the renamed app once.
  assert.equal((runner.match(/\['earlier-launch', receipt/g) || []).length, 1);
  assert.equal((runner.match(/\['launch', /g) || []).length, 1);
  // The updated app is only quit normally; force exists only in cleanup's stop.
  const work = runner.split('budget.beginCleanup();')[0];
  assert.doesNotMatch(work, /'earlier-stop'|\['stop', /);
  assert.doesNotMatch(runner, /killall|pkill|\/usr\/bin\/open|TAURI_CONFIG|GITHUB_TOKEN|GH_TOKEN/);
  const cleanup = runner.split('budget.beginCleanup();')[1];
  assert.ok(cleanup.indexOf('await stopUpdateProcesses(') < cleanup.indexOf('await rm(bundle'));
  assert.ok(cleanup.indexOf("'cleanup-all-exited'") < cleanup.indexOf('await rm(bundle'));
  assert.ok(cleanup.indexOf("'cleanup-reopened-exited'") < cleanup.indexOf('await rm(bundle'));
  assert.match(
    cleanup,
    /\['unregister-earlier', earlierBundle\],\s*\['unregister-renamed', renamedBundle\]/,
  );
  assert.ok(
    cleanup.indexOf('await rm(staging') < cleanup.indexOf('report.cleanupConfirmed = true'),
  );
  assert.match(cleanup, /catch \(error\) \{\s*report.cleanupError/);
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

test('update workflow leaves cleanup/upload time and denies wrong authorization before checkout', () => {
  assert.doesNotMatch(
    job,
    /secrets\.|environment:|contents: write|continue-on-error|release-dmg.mjs/,
  );
  assert.match(job, /if: always\(\)/);
  const minutes = [...job.matchAll(/^ {8}timeout-minutes: (\d+)$/gm)].map((m) => Number(m[1]));
  assert.deepEqual(minutes, [1, 2, 2, 3, 17, 2]);
  assert.equal(Number(job.match(/^ {4}timeout-minutes: (\d+)$/m)[1]), 28);
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
      const receipt = launcher.slice(
        launcher.indexOf('struct Receipt:'),
        launcher.indexOf('// BEGIN TRANSITION POLICY'),
      );
      const transition = launcher
        .split('// BEGIN TRANSITION POLICY:')[1]
        .split('\n')
        .slice(1)
        .join('\n')
        .split('// END TRANSITION POLICY')[0];
      const ui = driver
        .split('// BEGIN UPDATE UI POLICY:')[1]
        .split('\n')
        .slice(1)
        .join('\n')
        .split('// END UPDATE UI POLICY')[0];
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
            // The updater writes into the earlier folder; a successor elsewhere is not adopted.
            [record(11, 201, "ai.xtrace.app", ProductBundle.current)],
            [Receipt(pid: 11, startTime: 201, bundle: old.bundle, executable: "/wrong", identifier: old.identifier)]] {
  do { _ = try successor(t, bad, originalAlive: false); fatalError("Accepted invalid successor") } catch {}
}
// Only the earlier folder is updated in place: no transition starts from XTrace.app.
let currentOriginal = Transition(original: record(10, 100, "ai.xtrace.app", ProductBundle.current), armedAt: 200)
do { _ = try successor(currentOriginal, [], originalAlive: true); fatalError("Accepted a transition outside the earlier folder") } catch {}
func text(_ value: String) -> [String: String] { [kAXValueAttribute: value] }
func versionRow(_ value: String, _ parent: String = "footer", _ position: Int = 0,
                _ workspace: Bool = true) -> [String: String] {
  [kAXRoleAttribute: "AXStaticText", kAXValueAttribute: value, "captureParent": parent,
   "captureChildIndex": String(position), "captureWorkspace": workspace ? "true" : "false"]
}
// Historical hosted AXStaticText values were "v" and "0.1.6". The 0.1.7 and
// 0.1.8 rows and relationships below are synthetic, not a new runtime capture.
assert(Release.candidate == "0.1.8" && Release.earlier == "0.1.7")
assert(Release.bundle("update-ready") == "/Applications/XTrace Desktop.app")
assert(Release.bundle("update-press") == "/Applications/XTrace Desktop.app")
assert(Release.bundle("updated-dashboard") == "/Applications/XTrace Desktop.app")
for mode in ["reopened-dashboard", "first-launch", "telemetry-notice", "unknown"] {
  assert(Release.bundle(mode) == "/Applications/XTrace.app")
}
let splitVersion = [versionRow("v", "footer", 4), versionRow("0.1.8", "footer", 5)]
assert(displayedVersion(splitVersion, "0.1.8"))
assert(displayedVersion([versionRow("v0.1.8")], "0.1.8"))
for invalid in [
  [versionRow("v", "footer", 4), versionRow("0.1.8", "other-pane", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.8", "footer", 6)],
  [versionRow("0.1.8", "footer", 4), versionRow("v", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.7", "footer", 5)],
  [versionRow("v", "footer", 4, false), versionRow("0.1.8", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.8", "footer", 5, false)],
  [versionRow("v0.1.8", "main-pane", 0, false)],
  [versionRow("Update 0.1.8 is ready. Your local data stays on this Mac.")],
  [versionRow("v0.1.7"), versionRow("0.1.8", "update-banner", 1)],
  [text("v0.1.8")],
  [[kAXRoleAttribute: "AXButton", kAXValueAttribute: "v0.1.8", "captureWorkspace": "true"]]
] { assert(!displayedVersion(invalid, "0.1.8")) }
let rows: [[String: String]] = [[kAXRoleAttribute: kAXWindowRole], [kAXRoleAttribute: "AXWebArea"],
  versionRow("v0.1.7"), text("What your agents did"),
  text("No agent activity was recorded in this range. Measured zeros are shown as 0."),
  text("Update 0.1.8 is ready. Your local data stays on this Mac."),
  [kAXRoleAttribute: kAXButtonRole, kAXTitleAttribute: "Restart to update"]]
assert(updateReady(rows))
for index in rows.indices { assert(!updateReady(Array(rows.enumerated().filter { $0.offset != index }.map { $0.element }))) }
assert(!updateReady(rows + [rows.last!]))
assert(!updateReady(rows + [text("Welcome to XTrace")]))
assert(!updateReady(rows + [[kAXRoleAttribute: "AXAlert"]]))
assert(!updateReady(rows + [[kAXSubroleAttribute: "AXDialog"]]))
assert(!updateReady(rows + [text("Dashboard metrics could not be loaded.")]))
let updated = rows.map { $0 == versionRow("v0.1.7") ? versionRow("v0.1.8") : $0 }
assert(!updateReady(updated)); assert(updateDashboard(updated, "0.1.8"))
let splitDashboard = updated.filter { $0 != versionRow("v0.1.8") } + splitVersion
assert(updateDashboard(splitDashboard, "0.1.8"))
let splitOldDashboard = rows.filter { $0 != versionRow("v0.1.7") } +
    [versionRow("v", "footer", 4), versionRow("0.1.7", "footer", 5)]
assert(updateReady(splitOldDashboard))
// After the restart and after reopening the renamed app: Dashboard, never Welcome.
assert(!updateDashboard(updated + [text("Welcome to XTrace")], "0.1.8"))
assert(!updateDashboard(rows, "0.1.8"))
let wrongTarget = rows.map { $0 == rows[5] ? text("Update 0.1.9 is ready. Your local data stays on this Mac.") : $0 }
assert(!updateReady(wrongTarget))
let earlierTarget = rows.map { $0 == rows[5] ? text("Update 0.1.7 is ready. Your local data stays on this Mac.") : $0 }
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
