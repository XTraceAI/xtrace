import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { executionBudget, verifyAppInventory } from './hosted-release.mjs';
import {
  previousRelease,
  requireUpdateHost,
  verifyPreviousDownload,
  verifyPreviousInventory,
  verifyProductionFeed,
  updateBudget,
  stopUpdateProcesses,
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
    assert.match(report.error, /explicit hosted real-identity exception/);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('production prerequisite accepts canonical bytes and rejects changed version, URL, signature and body', async () => {
  const bytes = await readFile(new URL('./fixtures/hosted-update-latest.json', import.meta.url));
  assert.equal(verifyProductionFeed(bytes).version, '0.1.6');
  for (const text of [
    bytes.toString().replace('0.1.6', '0.1.5'),
    bytes.toString().replace('github.com', 'github.net'),
    bytes.toString().replace('signature', 'signaturf'),
    bytes.toString().replace('testing', 'TestinG'),
    '{}',
    '',
    bytes.toString() + '\n',
  ])
    assert.throws(() => verifyProductionFeed(Buffer.from(text)), /prerequisite unmet/);
  const changed = Buffer.from(bytes);
  changed[changed.indexOf('signature') + 15] ^= 1;
  assert.throws(() => verifyProductionFeed(changed));
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

test('update ordering verifies old files before one press, adopts before inspection, and checks native data after exit', () => {
  const ordered = [
    'verifyProductionFeed(await',
    'verifyPreviousDownload(await',
    "'old-extract'",
    "'launch-services'",
    "'update-ready',",
    'report.beforePressInventory',
    "'arm-update',",
    'pressAttempted = true',
    "'update-press',",
    "'adopt-update',",
    "'successor-environment'",
    "'updated-dashboard',",
    'report.updatedInventory',
    "'stop-successor'",
    "'all-exited'",
    'report.sentinelRetained',
    'PRAGMA integrity_check',
  ];
  let prior = -1;
  for (const needle of ordered) {
    const at = runner.indexOf(needle);
    assert.ok(at > prior, needle);
    prior = at;
  }
  assert.equal((runner.match(/\['launch', receipt/g) || []).length, 1);
  assert.doesNotMatch(runner, /killall|pkill|\/usr\/bin\/open|TAURI_CONFIG|GITHUB_TOKEN|GH_TOKEN/);
  const cleanup = runner.split('budget.beginCleanup();')[1];
  assert.ok(
    cleanup.indexOf('await stopUpdateProcesses(') < cleanup.indexOf('await rm(release.bundle'),
  );
  assert.ok(cleanup.indexOf("'cleanup-all-exited'") < cleanup.indexOf('await rm(release.bundle'));
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
  assert.deepEqual(minutes, [1, 2, 2, 2, 15, 2]);
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
  assert.equal((driver.match(/displayedVersion\(last, "0\.1\.6"\)/g) || []).length, 2);
  assert.equal((driver.match(/displayedVersion\(last, "0\.1\.5"\)/g) || []).length, 1);
  assert.match(driver, /displayedVersion\(rows, version\)/);
  assert.doesNotMatch(driver, /hasText\([^\n]*"v(?:0\.1\.[56]|" \+ version)/);
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
func record(_ pid: Int32, _ date: Double, _ id: String = "ai.xtrace.app", _ path: String = "/Applications/XTrace Desktop.app") -> Receipt {
  Receipt(pid: pid, launchDate: date, bundle: path, executable: path + "/Contents/MacOS/xtrace-desktop", identifier: id)
}
let old = record(10, 100), next = record(11, 201)
let t = Transition(original: old, armedAt: 200)
assert(try! successor(t, [old], originalAlive: true) == nil)
assert(try! successor(t, [old, next], originalAlive: true) == nil)
assert(try! successor(t, [], originalAlive: false) == nil)
assert(try! successor(t, [next], originalAlive: false)?.pid == 11)
for bad in [[record(10, 201)], [record(11, 199)], [next, record(12, 202)], [next, next],
            [record(11, 201, "ai.xtrace.desktop")], [record(11, 201, "ai.xtrace.app", "/other")],
            [Receipt(pid: 11, launchDate: 201, bundle: old.bundle, executable: "/wrong", identifier: old.identifier)]] {
  do { _ = try successor(t, bad, originalAlive: false); fatalError("Accepted invalid successor") } catch {}
}
func text(_ value: String) -> [String: String] { [kAXValueAttribute: value] }
func versionRow(_ value: String, _ parent: String = "footer", _ position: Int = 0,
                _ workspace: Bool = true) -> [String: String] {
  [kAXRoleAttribute: "AXStaticText", kAXValueAttribute: value, "captureParent": parent,
   "captureChildIndex": String(position), "captureWorkspace": workspace ? "true" : "false"]
}
// Captured hosted AXStaticText values were "v" and "0.1.6". Relationships
// below are synthetic: the old flat capture did not record their parent IDs.
let splitVersion = [versionRow("v", "footer", 4), versionRow("0.1.6", "footer", 5)]
assert(displayedVersion(splitVersion, "0.1.6"))
assert(displayedVersion([versionRow("v0.1.6")], "0.1.6"))
for invalid in [
  [versionRow("v", "footer", 4), versionRow("0.1.6", "other-pane", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.6", "footer", 6)],
  [versionRow("0.1.6", "footer", 4), versionRow("v", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.5", "footer", 5)],
  [versionRow("v", "footer", 4, false), versionRow("0.1.6", "footer", 5)],
  [versionRow("v", "footer", 4), versionRow("0.1.6", "footer", 5, false)],
  [versionRow("v0.1.6", "main-pane", 0, false)],
  [versionRow("Update 0.1.6 is ready. Your local data stays on this Mac.")],
  [versionRow("v0.1.5"), versionRow("0.1.6", "update-banner", 1)],
  [text("v0.1.6")],
  [[kAXRoleAttribute: "AXButton", kAXValueAttribute: "v0.1.6", "captureWorkspace": "true"]]
] { assert(!displayedVersion(invalid, "0.1.6")) }
let rows: [[String: String]] = [[kAXRoleAttribute: kAXWindowRole], [kAXRoleAttribute: "AXWebArea"],
  versionRow("v0.1.5"), text("What your agents did"),
  text("No agent activity was recorded in this range. Measured zeros are shown as 0."),
  text("Update 0.1.6 is ready. Your local data stays on this Mac."),
  [kAXRoleAttribute: kAXButtonRole, kAXTitleAttribute: "Restart to update"]]
assert(updateReady(rows))
for index in rows.indices { assert(!updateReady(Array(rows.enumerated().filter { $0.offset != index }.map { $0.element }))) }
assert(!updateReady(rows + [rows.last!]))
assert(!updateReady(rows + [text("Welcome to XTrace")]))
assert(!updateReady(rows + [[kAXRoleAttribute: "AXAlert"]]))
assert(!updateReady(rows + [[kAXSubroleAttribute: "AXDialog"]]))
assert(!updateReady(rows + [text("Dashboard metrics could not be loaded.")]))
let updated = rows.map { $0 == versionRow("v0.1.5") ? versionRow("v0.1.6") : $0 }
assert(!updateReady(updated)); assert(updateDashboard(updated, "0.1.6"))
let splitDashboard = updated.filter { $0 != versionRow("v0.1.6") } + splitVersion
assert(updateDashboard(splitDashboard, "0.1.6"))
let splitOldDashboard = rows.filter { $0 != versionRow("v0.1.5") } +
    [versionRow("v", "footer", 4), versionRow("0.1.5", "footer", 5)]
assert(updateReady(splitOldDashboard))
assert(!updateDashboard(updated + [text("Welcome to XTrace")], "0.1.6"))
let wrongTarget = rows.map { $0 == rows[5] ? text("Update 0.1.7 is ready. Your local data stays on this Mac.") : $0 }
assert(!updateReady(wrongTarget))
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
