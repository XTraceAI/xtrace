import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import { appInventory, executionBudget, sha256 } from './hosted-release.mjs';
import {
  acceptResult,
  fixture,
  requireFailureHost,
  serveFixtures,
  verifyFixture,
} from './hosted-updater-failures.mjs';

const sha = 'a'.repeat(40);
const host = () => ({
  mode: 'updater-failures',
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
    SOURCE_SHA: sha,
    WORKFLOW_SHA: sha,
    DEFAULT_BRANCH: 'main',
    WORKFLOW_REF: 'refs/heads/main',
    WORKFLOW_REF_TYPE: 'branch',
  },
});
const source = await readFile(new URL('./hosted-updater-failures.mjs', import.meta.url), 'utf8');
const observer = await readFile(new URL('./hosted-updater-cancel.swift', import.meta.url), 'utf8');
const rust = await readFile(
  new URL('../../apps/desktop/src-tauri/examples/updater_failures.rs', import.meta.url),
  'utf8',
);
const workflow = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const job = workflow.split('\n  updater-failures:\n')[1].split('\n  signed-first-launch:\n')[0];

const section = (name) =>
  source.split(`// BEGIN ${name}`)[1].split('\n').slice(1).join('\n').split(`// END ${name}`)[0];

test('actual Apple signature command uses a literal Apple anchor and rejects all failed verification results', () => {
  for (const result of [
    { status: 0, signal: null, stdout: '', stderr: '' },
    { status: 3, signal: null, stdout: '', stderr: 'requirement rejected' },
    { status: 0, signal: 'SIGKILL', stdout: '', stderr: '' },
    { status: 0, error: new Error('spawn failure'), stdout: '', stderr: '' },
  ]) {
    const report = { commands: [] },
      calls = [];
    const command = runInNewContext(`${section('CHECKED COMMAND')}; command`, {
      report,
      systemEnvironment: {},
      budget: { remaining: (n) => n },
      spawnSync: (...args) => {
        calls.push(args);
        return result;
      },
    });
    const run = () =>
      runInNewContext(section('APPLE SIGNATURE CHECK'), {
        command,
        securityExecutable: '/system/SecurityAgent',
      });
    if (result.status === 0 && !result.signal && !result.error) assert.doesNotThrow(run);
    else assert.throws(run);
    assert.equal(calls[0][0], '/usr/bin/codesign');
    assert.deepEqual(Array.from(calls[0][1]), [
      '--verify',
      '--strict',
      '-R',
      '=anchor apple',
      '/system/SecurityAgent',
    ]);
    assert.equal(report.commands[0].status, result.status);
  }
});

const registryDump = (...paths) =>
  Buffer.from(
    `Status: Database is seeded\nbundle id: 42 (0x2a)\n${paths.map((path) => `path:                       ${path} (0xd660)\n`).join('')}`,
  );
const registryExists = async (path) => {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error.code === 'ENOENT') return false;
    throw error;
  }
};
async function registryFixture(overrides = {}) {
  const output = await realpath(await mkdtemp(join(tmpdir(), 'registry-proof-')));
  const parent = join(output, 'run'),
    owned = join(parent, 'staging.noindex');
  const helperName = `XTraceUpdaterTest-${'a'.repeat(32)}`;
  const helper = join(owned, `${helperName}.app`),
    target = join(owned, 'valid-target', 'XTrace Desktop.app');
  await mkdir(helper, { recursive: true });
  await mkdir(target, { recursive: true });
  const calls = [],
    report = { error: 'original setup failure' };
  const dump =
    overrides.dump?.({ owned, helper, target }) ||
    registryDump('/System/Applications/TextEdit.app', `${owned}-neighbor/${helperName}.app`);
  const cleanup = runInNewContext(`${section('OWNED REGISTRY CLEANUP')}; cleanupBundles`, {
    owned,
    parent,
    helperName,
    cases: ['valid'],
    output,
    report,
    Buffer,
    sha256,
    dirname,
    join,
    realpath,
    lstat,
    exists: registryExists,
    writeFile: overrides.writeFile || writeFile,
    rm: overrides.rm || rm,
    budget: { remaining: (n) => n },
    systemEnvironment: {},
    spawnSync: (executable, args, options) => {
      calls.push({ executable, args: Array.from(args), options });
      if (args[0] === '-u')
        return (
          overrides.unregister?.(args[1]) || { status: 0, signal: null, stdout: '', stderr: '' }
        );
      assert.deepEqual(Array.from(args), ['-dump']);
      return {
        status: 0,
        signal: null,
        stdout: dump,
        stderr: Buffer.alloc(0),
        ...overrides.dumpResult,
      };
    },
  });
  return {
    output,
    parent,
    owned,
    helper,
    target,
    calls,
    report,
    dump,
    cleanup,
    dispose: () => rm(output, { recursive: true, force: true }),
  };
}

test('actual owned cleanup proves raw suffix-bearing registry absence once, then filesystem absence', async () => {
  const fixture = await registryFixture();
  try {
    await fixture.cleanup();
    assert.equal(await registryExists(fixture.parent), false);
    assert.deepEqual(
      fixture.calls.map((call) => call.args[0]),
      ['-u', '-u', '-dump'],
    );
    assert.deepEqual(
      fixture.calls.map((call) => call.options.maxBuffer),
      [4, 4, 64].map((n) => n * 1024 * 1024),
    );
    assert.ok(fixture.calls.every((call) => call.options.timeout === 30000));
    const proof = fixture.report.registryCleanup;
    assert.equal(proof.absenceConfirmed, true);
    assert.equal(proof.filesystemAbsent, true);
    assert.deepEqual(await readFile(proof.dump.log), fixture.dump);
    assert.equal(proof.dump.sha256, sha256(fixture.dump));
    assert.equal(proof.dump.bytes, fixture.dump.length);
    assert.equal(proof.dump.stdout, undefined);
    assert.equal(fixture.report.error, 'original setup failure');
  } finally {
    await fixture.dispose();
  }
});

test('exact observed unregister failure needs independent absence; other failures and every present owned path retain staging', async () => {
  const missing = (path) => ({
    status: 1,
    signal: null,
    stdout: '',
    stderr: `failed to scan ${path}: -10814\n from spotlight`,
  });
  const absent = await registryFixture({ unregister: missing });
  try {
    await absent.cleanup();
    assert.equal(absent.report.registryCleanup.unregister[0].status, 1);
    assert.equal(absent.report.registryCleanup.unregister[0].reportedMissing, true);
    assert.equal(absent.report.registryCleanup.absenceConfirmed, true);
  } finally {
    await absent.dispose();
  }
  for (const change of [
    { dump: ({ helper }) => registryDump(helper) },
    { dump: ({ target }) => registryDump(target) },
    { dump: ({ owned }) => registryDump(join(owned, 'unexpected.app')) },
    { unregister: () => missing('/wrong/owned/path.app') },
    { unregister: () => ({ status: 2, signal: null, stdout: '', stderr: 'unexpected failure' }) },
    { unregister: (path) => ({ ...missing(path), error: new Error('timeout') }) },
    { unregister: (path) => ({ ...missing(path), signal: 'SIGKILL' }) },
  ]) {
    const fixture = await registryFixture({ unregister: missing, ...change });
    try {
      await assert.rejects(fixture.cleanup);
      assert.equal(await registryExists(fixture.parent), true);
      assert.notEqual(fixture.report.registryCleanup.filesystemAbsent, true);
      assert.equal(fixture.calls.filter((call) => call.args[0] === '-dump').length, 1);
    } finally {
      await fixture.dispose();
    }
  }
});

test('dump errors, malformed/partial output and failed removal cannot establish cleanup', async () => {
  for (const change of [
    { dump: () => Buffer.alloc(0) },
    { dump: () => Buffer.from('Status: seeded\n') },
    { dump: () => Buffer.from('Status: seeded\nbundle id: 42\npath: file:///unsupported\n') },
    { dump: () => registryDump('/System/Applications/TextEdit.app').subarray(0, -1) },
    { dump: () => Buffer.from([255, 254]) },
    { dumpResult: { status: 1 } },
    { dumpResult: { error: Object.assign(new Error('maxBuffer exceeded'), { code: 'ENOBUFS' }) } },
    { dumpResult: { error: new Error('timeout'), signal: 'SIGKILL' } },
    { dumpResult: { signal: 'SIGTERM' } },
    { dumpResult: { stderr: Buffer.from('database could not be read') } },
    {
      rm: async () => {
        throw new Error('removal failed');
      },
    },
    { rm: async () => {} },
    {
      writeFile: async () => {
        throw new Error('raw evidence not saved');
      },
    },
  ]) {
    const fixture = await registryFixture(change);
    try {
      await assert.rejects(fixture.cleanup);
      assert.equal(await registryExists(fixture.parent), true);
      assert.notEqual(fixture.report.registryCleanup.filesystemAbsent, true);
    } finally {
      await fixture.dispose();
    }
  }
});

test('actual cleanup keeps process, evidence and authorization failures ahead of registry mutation/removal', async () => {
  const cleanup = source
    .split('budget.beginCleanup();')[1]
    .split('// Always close the owned listener')[0];
  for (const gate of ['process', 'evidence', 'authorization']) {
    let registryCalls = 0;
    const report = {
      cases: gate === 'evidence' ? [{ finalObservation: { targetError: 'unreadable' } }] : [],
    };
    const result = await runInNewContext(
      `(async () => { ${cleanup}; return { report, failure }; })()`,
      {
        report,
        failure: new Error('original setup failure'),
        processes: [],
        command: () => {
          if (gate === 'process') throw new Error('process identity unknown');
          return '';
        },
        preserveCase: async () => {},
        permissionStarted: gate === 'authorization',
        cancelledInstall: () => false,
        server: undefined,
        targetParent: undefined,
        owned: '/owned/staging.noindex',
        parent: '/owned',
        cleanupBundles: async () => {
          registryCalls++;
        },
      },
    );
    assert.equal(registryCalls, 0);
    assert.ok(report.cleanupError);
    assert.notEqual(report.cleanupConfirmed, true);
    assert.equal(result.failure.message, 'original setup failure');
  }
});

test('SDK mode requires every hosted/source guard and refuses even reviewed tags', () => {
  assert.doesNotThrow(() => requireFailureHost(host()));
  for (const key of Object.keys(host().env)) {
    const changed = host();
    changed.env[key] = 'wrong';
    assert.throws(() => requireFailureHost(changed), key);
  }
  for (const changed of [
    { mode: 'update' },
    { platform: 'linux' },
    { arch: 'x64' },
    { osVersion: '15.0' },
    { head: 'b'.repeat(40) },
  ])
    assert.throws(() => requireFailureHost({ ...host(), ...changed }));
  const tag = host();
  tag.env.WORKFLOW_REF = 'refs/tags/v0.1.7';
  tag.env.WORKFLOW_REF_TYPE = 'tag';
  assert.throws(() => requireFailureHost(tag));
});

test('local entrypoint refuses before fixture/download/process preparation', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'sdk-guard-'));
  try {
    const run = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('./hosted-updater-failures.mjs', import.meta.url))],
      { cwd: directory, env: { PATH: '/usr/bin:/bin' }, encoding: 'utf8', timeout: 10000 },
    );
    assert.equal(run.status, 1);
    const files = await readdir(join(directory, 'artifacts/hosted-updater-failures'));
    assert.deepEqual(files, ['report.json']);
    const report = JSON.parse(
      await readFile(join(directory, 'artifacts/hosted-updater-failures/report.json')),
    );
    assert.equal(report.releasedAppLaunched, false);
    assert.equal(report.processes.length, 0);
  } finally {
    await rm(directory, { recursive: true });
  }
});

test('fixture pins reject substituted content and dependency lock', () => {
  assert.throws(() =>
    verifyFixture(
      Buffer.alloc(fixture.bytes),
      Buffer.alloc(fixture.signatureBytes),
      Buffer.from('lock'),
    ),
  );
  assert.throws(() => verifyFixture(Buffer.alloc(0), Buffer.alloc(0), Buffer.alloc(0)));
  assert.match(source, /sha256\(lock\) !== fixture.lockSha256/);
});

test('actual owned HTTP routes serve exact, corrupt, stalled and truncated fixture bodies without redirects', async () => {
  const archive = Buffer.alloc(8192, 37);
  const server = await serveFixtures(archive, 'owned-signature');
  const base = `http://127.0.0.1:${server.port}`;
  try {
    for (const name of ['valid', 'corrupt', 'version', 'permission', 'timeout', 'interrupted']) {
      const response = await fetch(`${base}/${name}/latest.json`, { redirect: 'error' });
      assert.equal(response.status, 200);
      assert.equal(response.headers.get('location'), null);
      const manifest = await response.json();
      assert.equal(manifest.version, name === 'version' ? '0.1.8' : '0.1.7');
      assert.equal(manifest.platforms['darwin-aarch64'].signature, 'owned-signature');
      const download = await fetch(manifest.platforms['darwin-aarch64'].url, {
        redirect: 'error',
        ...(name === 'timeout' ? { signal: AbortSignal.timeout(200) } : {}),
      });
      assert.equal(download.status, 200);
      assert.equal(download.headers.get('location'), null);
      if (name === 'timeout' || name === 'interrupted') {
        await assert.rejects(() => download.arrayBuffer());
        continue;
      }
      const body = Buffer.from(await download.arrayBuffer());
      const expected = Buffer.from(archive);
      if (name === 'corrupt') expected[4096] ^= 1;
      assert.deepEqual(body, expected);
    }
    for (const [path, options] of [
      ['/unknown/archive', {}],
      ['/valid/archive', { method: 'POST' }],
    ]) {
      const rejected = await fetch(base + path, options);
      assert.equal(rejected.status, 404);
      assert.equal(rejected.headers.get('location'), null);
    }
    assert.equal(server.receipts.length, 12);
    for (const row of server.receipts) assert.equal(row.responseStatus, 200);
    assert.equal(server.receipts.find((row) => row.path === '/timeout/archive').sentBytes, 4096);
    assert.equal(
      server.receipts.find((row) => row.path === '/interrupted/archive').sentBytes,
      4096,
    );
  } finally {
    await server.close();
  }
});

test('actual child closure rejects expired budgets before spawn and records killed/malformed children', async () => {
  const output = await mkdtemp(join(tmpdir(), 'sdk-child-'));
  try {
    let clock = 0,
      spawnCount = 0;
    const budget = executionBudget(() => clock, { workMs: 1000, cleanupMs: 1000 });
    const processes = [];
    const code = source
      .split('// BEGIN OWNED CHILD:')[1]
      .split('\n')
      .slice(1)
      .join('\n')
      .split('// END OWNED CHILD')[0];
    const launch = runInNewContext(`${code}; launch`, {
      budget,
      processes,
      output,
      spawn: (...args) => {
        spawnCount++;
        return spawn(...args);
      },
      process,
      join,
      writeFile,
      setTimeout,
      clearTimeout,
    });
    clock = 1000;
    assert.throws(() => launch('expired', process.execPath, ['-e', 'process.exit(0)'], 100, {}));
    assert.equal(spawnCount, 0);
    assert.equal(processes.length, 0);
    clock = 0;
    const malformed = launch(
      'malformed',
      process.execPath,
      ['-e', 'console.log("invalid")'],
      1000,
      {},
    );
    await assert.rejects(malformed.promise);
    assert.equal(malformed.entry.exited, true);
    assert.match(await readFile(join(output, 'malformed.log'), 'utf8'), /invalid/);
    assert.ok(malformed.entry.error);
    const killed = launch(
      'watchdog',
      process.execPath,
      ['-e', 'setInterval(() => {}, 1000)'],
      100,
      {},
    );
    await assert.rejects(killed.promise, /watchdog/);
    assert.equal(killed.entry.exited, true);
    assert.equal(killed.entry.timedOut, true);
    assert.equal(killed.entry.signal, 'SIGKILL');
    assert.throws(() => process.kill(killed.entry.pid, 0), { code: 'ESRCH' });
    const missing = launch('missing', join(output, 'absent-executable'), [], 100, {});
    await assert.rejects(missing.promise);
    assert.equal(missing.entry.exited, true);
    assert.match(missing.entry.error, /ENOENT/);
  } finally {
    await rm(output, { recursive: true });
  }
});

test('actual final-case closure retains changed/missing inventories, raw attempts and child failures outside staging', async () => {
  const output = await mkdtemp(join(tmpdir(), 'sdk-evidence-'));
  try {
    const target = join(output, 'old'),
      data = join(output, 'data'),
      attempt = join(output, 'attempt.json');
    const { mkdir } = await import('node:fs/promises');
    await mkdir(target);
    await mkdir(data);
    await writeFile(join(target, 'app'), 'before');
    await writeFile(attempt, '{"installAttempted":true}');
    const row = {
      name: 'permission',
      before: await appInventory(target),
      config: { target, installReceipt: attempt },
    };
    await writeFile(join(output, 'permission-case.json'), JSON.stringify(row));
    await writeFile(join(target, 'app'), 'changed after failure');
    const processes = [
      {
        name: 'permission',
        pid: 123,
        exited: true,
        completed: true,
        timedOut: true,
        signal: 'SIGKILL',
        error: 'watchdog',
        stopError: 'signal diagnostic',
      },
    ];
    const code = source
      .split('// BEGIN FINAL CASE:')[1]
      .split('\n')
      .slice(1)
      .join('\n')
      .split('// END FINAL CASE')[0];
    const preserve = runInNewContext(`${code}; preserveCase`, {
      processes,
      childEnv: { XTRACE_DATA_DIR: data },
      appInventory,
      readFile,
      saveCase: (value) => writeFile(join(output, 'permission-case.json'), JSON.stringify(value)),
    });
    await preserve(row, true);
    const saved = JSON.parse(await readFile(join(output, 'permission-case.json')));
    assert.equal(saved.finalObservation.processExitConfirmed, true);
    assert.notEqual(saved.before.sha256, saved.finalObservation.target.sha256);
    assert.equal(saved.processes[0].stopError, 'signal diagnostic');
    assert.equal(saved.installAttemptReceipt, '{"installAttempted":true}');
    await rm(target, { recursive: true });
    await preserve(row, false);
    const missing = JSON.parse(await readFile(join(output, 'permission-case.json')));
    assert.equal(missing.finalObservation.processExitConfirmed, false);
    assert.match(missing.finalObservation.targetError, /ENOENT/);
    assert.equal(missing.before.sha256, saved.before.sha256);
  } finally {
    await rm(output, { recursive: true });
  }
});

const receipt = (name) => [{ path: `/${name}/latest.json` }, { path: `/${name}/archive` }];
const result = (name) => ({
  status: 'passed',
  case: name,
  pid: 1234,
  downloadTimeoutMs: 600000,
  verifiedDownload: false,
  installAttempted: false,
  error: 'actual error',
  errorDebug: 'Reqwest',
  elapsedMs: 600001,
  transferred: 4096,
  transferFinishedCallback: false,
});

test('acceptance rejects missing transfers, shortened timers, watchdogs and unexpected verified downloads', () => {
  for (const name of ['corrupt', 'version', 'timeout', 'interrupted']) {
    assert.doesNotThrow(() => acceptResult(name, result(name), receipt(name)));
    for (const change of [
      { status: 'failed' },
      { verifiedDownload: true },
      { installAttempted: true },
      { error: '' },
      { errorDebug: '' },
      { pid: 0 },
      { downloadTimeoutMs: 1000 },
    ])
      assert.throws(() => acceptResult(name, { ...result(name), ...change }, receipt(name)));
    assert.throws(() => acceptResult(name, result(name), []));
    assert.throws(() => acceptResult(name, result(name), [...receipt(name), ...receipt(name)]));
  }
  for (const elapsedMs of [1000, 598999, 630001])
    assert.throws(() =>
      acceptResult('timeout', { ...result('timeout'), elapsedMs }, receipt('timeout')),
    );
  for (const name of ['timeout', 'interrupted'])
    for (const change of [
      { transferred: 0 },
      { transferred: fixture.bytes },
      { transferFinishedCallback: true },
    ])
      assert.throws(() => acceptResult(name, { ...result(name), ...change }, receipt(name)));
});

test('permission result requires verified download plus independent Cancel and disappearance evidence', () => {
  const sdk = {
    ...result('permission'),
    verifiedDownload: true,
    verifiedBytes: fixture.bytes,
    transferred: fixture.bytes,
    installAttempted: true,
  };
  const cancel = {
    status: 'passed',
    cancelPressed: true,
    requester: { pid: 1234 },
    action: {},
    after: [],
  };
  assert.doesNotThrow(() => acceptResult('permission', sdk, receipt('permission'), cancel));
  for (const change of [
    { status: 'failed' },
    { cancelPressed: false },
    { requester: null },
    { requester: { pid: 9876 } },
    { action: null },
    { after: [{}] },
  ])
    assert.throws(() =>
      acceptResult('permission', sdk, receipt('permission'), { ...cancel, ...change }),
    );
  assert.throws(() => acceptResult('permission', sdk, receipt('permission')));
});

test(
  'compile actual Cancel predicate with synthetic rows only; no native observer or SDK execution',
  { skip: process.platform !== 'darwin' },
  async () => {
    const directory = await mkdtemp(join(tmpdir(), 'cancel-policy-'));
    try {
      const policy = observer
        .split('// BEGIN CANCEL POLICY:')[1]
        .split('\n')
        .slice(1)
        .join('\n')
        .split('// END CANCEL POLICY')[0];
      const swift = join(directory, 'main.swift'),
        binary = join(directory, 'policy');
      await writeFile(
        swift,
        `import Foundation\n${policy}\nlet input = try! JSONSerialization.jsonObject(with: FileHandle.standardInput.readDataToEndOfFile()) as! [String: Any]\nlet rows = input["rows"] as! [[String: String]]\nprint(cancelIndex(rows, input["name"] as! String) ?? -1)\n`,
      );
      const compiled = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', join(directory, 'cache'), swift, '-o', binary],
        { encoding: 'utf8', timeout: 120000 },
      );
      assert.equal(compiled.status, 0, compiled.stderr);
      const name = `XTraceUpdaterTest-${'a'.repeat(32)}`;
      const good = [
        { AXRole: 'AXWindow', AXSubrole: 'AXDialog' },
        { AXRole: 'AXStaticText', AXValue: `${name} wants to make changes.` },
        { AXRole: 'AXButton', AXTitle: 'Cancel', enabled: 'true' },
      ];
      const check = (rows) =>
        Number(
          spawnSync(binary, [], {
            input: JSON.stringify({ rows, name }),
            encoding: 'utf8',
            timeout: 5000,
          }).stdout.trim(),
        );
      assert.equal(check(good), 2);
      for (const rows of [
        [],
        good.slice(0, 2),
        [...good, good[2]],
        [{ AXRole: 'AXWindow' }, ...good.slice(1)],
        [good[0], { AXRole: 'AXTextField', AXValue: name }, good[2]],
        [good[0], { AXRole: 'AXStaticText', AXHelp: name }, good[2]],
        [good[0], { AXRole: 'AXStaticText', AXValue: `prefix${name}` }, good[2]],
        [good[0], { AXRole: 'AXStaticText', AXValue: `${name}-other` }, good[2]],
        [good[0], { AXRole: 'AXStaticText', AXValue: 'XTrace' }, good[2]],
        [good[0], good[1], { ...good[2], enabled: 'false' }],
        [good[0], good[1], { ...good[2], AXTitle: 'OK' }],
      ])
        assert.equal(check(rows), -1);
    } finally {
      await rm(directory, { recursive: true });
    }
  },
);

test(
  'actual refresh wait delivers common-set work in both directions without extending deadlines or spinning',
  { skip: process.platform !== 'darwin' },
  async () => {
    const directory = await mkdtemp(join(tmpdir(), 'observer-refresh-'));
    try {
      const wait = observer
        .split('// BEGIN REFRESH WAIT:')[1]
        .split('\n')
        .slice(1)
        .join('\n')
        .split('// END REFRESH WAIT')[0];
      const swift = join(directory, 'main.swift'),
        binary = join(directory, 'wait-controls');
      // Foundation only: in-memory state changes stand in for queued work, not
      // applications, AX windows or an actual authorization dialog.
      await writeFile(
        swift,
        `import Foundation
${wait}
var present = false
var evidence: [String: Any] = [:]
let appear = Timer(timeInterval: 0.01, repeats: false) { _ in present = true }
RunLoop.main.add(appear, forMode: .common)
Thread.sleep(forTimeInterval: 0.1)
evidence["oldWaitPresent"] = present
waitForRefresh(until: Date().addingTimeInterval(0.1))
evidence["refreshedPresent"] = present
appear.invalidate()
let disappear = Timer(timeInterval: 0.01, repeats: false) { _ in present = false }
RunLoop.main.add(disappear, forMode: .common)
Thread.sleep(forTimeInterval: 0.1)
evidence["oldWaitAbsent"] = !present
waitForRefresh(until: Date().addingTimeInterval(0.1))
evidence["refreshedAbsent"] = !present
disappear.invalidate()
let idleStart = Date(), idleDeadline = idleStart.addingTimeInterval(0.04)
var idleCalls = 0
while Date() < idleDeadline { waitForRefresh(until: idleDeadline); idleCalls += 1 }
evidence["idleElapsed"] = Date().timeIntervalSince(idleStart)
evidence["idleCalls"] = idleCalls
let expiredStart = Date()
waitForRefresh(until: expiredStart.addingTimeInterval(-1))
evidence["expiredElapsed"] = Date().timeIntervalSince(expiredStart)
var events = 0, eventCalls = 0
let repeated = Timer(timeInterval: 0.005, repeats: true) { _ in events += 1 }
RunLoop.main.add(repeated, forMode: .common)
let eventStart = Date(), eventDeadline = eventStart.addingTimeInterval(0.25)
while Date() < eventDeadline { waitForRefresh(until: eventDeadline); eventCalls += 1 }
repeated.invalidate()
evidence["eventElapsed"] = Date().timeIntervalSince(eventStart)
evidence["events"] = events
evidence["eventCalls"] = eventCalls
let bytes = try! JSONSerialization.data(withJSONObject: evidence, options: [.sortedKeys])
FileHandle.standardOutput.write(bytes)
`,
      );
      const compiled = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', join(directory, 'cache'), swift, '-o', binary],
        { encoding: 'utf8', timeout: 120000 },
      );
      assert.equal(compiled.status, 0, compiled.stderr);
      const run = spawnSync(binary, [], { encoding: 'utf8', timeout: 5000 });
      assert.equal(run.status, 0, `${run.error?.message || run.signal || ''}\n${run.stderr}`);
      const evidence = JSON.parse(run.stdout);
      console.log('Foundation refresh control evidence:', JSON.stringify(evidence));
      assert.equal(evidence.oldWaitPresent, false);
      assert.equal(evidence.refreshedPresent, true);
      assert.equal(evidence.oldWaitAbsent, false);
      assert.equal(evidence.refreshedAbsent, true);
      assert.ok(evidence.idleElapsed >= 0.03 && evidence.idleElapsed < 0.09);
      assert.ok(evidence.idleCalls >= 1 && evidence.idleCalls <= 2);
      assert.ok(evidence.expiredElapsed < 0.05);
      assert.ok(evidence.events >= 2);
      assert.ok(evidence.eventCalls >= 2 && evidence.eventCalls <= 4);
      assert.ok(evidence.eventElapsed >= 0.24 && evidence.eventElapsed < 0.34);
      const callSites = observer.split('// END REFRESH WAIT')[1];
      assert.equal((callSites.match(/waitForRefresh\(until: deadline\)/g) || []).length, 1);
      assert.equal((callSites.match(/waitForRefresh\(until: disappearedBy\)/g) || []).length, 1);
      assert.doesNotMatch(callSites, /Thread\.sleep/);
      assert.match(callSites, /let deadline = Date\(\)\.addingTimeInterval\(40\)/);
      assert.match(callSites, /let disappearedBy = Date\(\)\.addingTimeInterval\(5\)/);
    } finally {
      await rm(directory, { recursive: true });
    }
  },
);

test('workflow mode stays manually guarded and outside release packaging; SDK source boundaries remain explicit', () => {
  assert.match(job, /if: inputs.delivery_mode == 'updater-failures'/);
  assert.match(job, /needs: validate-mode/);
  assert.match(job, /test "\$WORKFLOW_SHA" = "\$CANDIDATE_SHA"/);
  assert.match(job, /test "\$SOURCE_SHA" = "\$CANDIDATE_SHA"/);
  assert.match(job, /test "\$WORKFLOW_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/);
  assert.match(job, /cargo \+1\.94\.1 build --locked -p xtrace-desktop --example updater_failures/);
  assert.match(job, /rustc \+1\.94\.1 --version/);
  assert.match(job, /xcrun swiftc -module-cache-path target\/hosted-updater-swift-cache\.noindex/);
  assert.doesNotMatch(source, /'compile-observer'/);
  const stepMaxima = [...job.matchAll(/^ {8}timeout-minutes: (\d+)$/gm)].map((match) =>
    Number(match[1]),
  );
  assert.ok(
    Number(job.match(/^ {4}timeout-minutes: (\d+)$/m)[1]) >
      stepMaxima.reduce((sum, value) => sum + value, 0),
  );
  assert.match(job, /timeout-minutes: 19/);
  assert.doesNotMatch(
    job,
    /secrets\.|environment:|persist-credentials: true|release-dmg|TAURI_CONFIG|contents: write/,
  );
  assert.match(rust, /update.timeout = Some\(Duration::from_secs\(600\)\)/);
  assert.match(rust, /Error::SignedVersionMismatch/);
  assert.match(rust, /Error::Minisign/);
  assert.match(rust, /update.install\(&bytes\)/);
  assert.doesNotMatch(rust, /app\.run\(|request_restart|std::env::set_var/);
  assert.doesNotMatch(
    observer,
    /kAXValueAttribute.*AXUIElementSet|CGEvent|kAXConfirm|do shell script|with administrator privileges/,
  );
  assert.match(observer, /CFEqual\(cancel, freshCancel\)/);
  assert.match(observer, /identity\(pid\) == requester/);
});
