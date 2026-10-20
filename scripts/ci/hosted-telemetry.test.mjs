import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { guardProcessDetails, guardFailureMetadata } from './hosted-guard-diagnostics.mjs';
import { executionBudget, firstLaunchEnvironment, release, sha256 } from './hosted-release.mjs';
import {
  telemetryEnvironment,
  validateTelemetryEnvironment,
  telemetryDataDirectory,
  verifyTelemetryDatabase,
  acceptTelemetryUI,
} from './hosted-telemetry.mjs';
import {
  cleanupTelemetry,
  telemetrySession,
  telemetryCleanupFacts,
  confirmProbeChildExit,
  confirmTelemetryLaunch,
  stopTelemetryOwners,
  stopTelemetryProxies,
  claimTelemetryProfiles,
  removeTelemetryProfiles,
} from './hosted-telemetry-run.mjs';
import {
  pollTelemetry,
  startTelemetryProxy,
  stopTelemetryProxy,
  prepareProbeSource,
} from './hosted-telemetry-proxy.mjs';
import { telemetryOwnedPaths, requireTelemetryHost, telemetryBudget } from './hosted-telemetry.mjs';

// BEGIN COMPILER DIAGNOSTICS: local assertions and owned temporary cleanup.
function recordCompiler(owner, label, timeout, elapsed, result) {
  owner.hasResult = true;
  owner.retain ||= Boolean(result.error || result.signal || result.status === null);
  let message = 'Compiler result diagnostics unavailable';
  try {
    message = JSON.stringify(guardProcessDetails(label, timeout, elapsed, result));
    console.log(message);
  } catch {
    /* Logging cannot change the status assertion. */
  }
  assert.equal(result.status, 0, message);
}
function diagnoseCompiler(owner, deadline) {
  if (!owner.hasResult || owner.metadataCollected) return;
  owner.metadataCollected = true;
  try {
    const metadata = guardFailureMetadata('swift', deadline);
    owner.retain ||= metadata.uncertainCompletion;
    console.log(JSON.stringify({ compilerMetadata: metadata }));
  } catch {
    owner.retain = true;
    try {
      console.log('Compiler metadata unavailable; descendant cleanup unconfirmed');
    } catch {
      /* Keep the primary error. */
    }
  }
}
async function cleanupCompiler(directory, owner) {
  if (owner.retain) {
    try {
      console.log(
        JSON.stringify({
          temporaryProofRetained: true,
          directory: basename(directory),
          compilerDescendants: 'unconfirmed',
          runnerTeardownVerified: false,
        }),
      );
    } catch {
      /* Keep the primary error. */
    }
  } else await rm(directory, { recursive: true, force: true });
}
// END COMPILER DIAGNOSTICS

const owned = '/owned/staging.noindex';
const home = '/Users/runner';
test('telemetry mode uses genuine default data and exact proxy variables; ordinary launch remains isolated', () => {
  const env = telemetryEnvironment(owned, home, 12345);
  assert.equal(env.HOME, home);
  assert.equal(env.XTRACE_NATIVE_HOME, join(owned, 'home'));
  assert.equal(env.XTRACE_DATA_DIR, undefined);
  assert.equal(env.XTRACE_TELEMETRY, undefined);
  assert.equal(env.HTTPS_PROXY, 'http://127.0.0.1:12345');
  assert.equal(env.https_proxy, env.HTTPS_PROXY);
  assert.equal(env.NO_PROXY, '');
  assert.equal(env.no_proxy, '');
  assert.deepEqual(validateTelemetryEnvironment(env, owned, home, 12345), env);
  const off = telemetryEnvironment(owned, home, 12345, true);
  assert.equal(off.XTRACE_TELEMETRY, '0');
  assert.deepEqual(validateTelemetryEnvironment(off, owned, home, 12345, true), off);
  assert.equal(
    telemetryDataDirectory(home),
    '/Users/runner/Library/Application Support/ai.xtrace.app',
  );
  const ordinary = firstLaunchEnvironment(owned);
  assert.equal(ordinary.XTRACE_DATA_DIR, join(owned, 'data'));
  assert.equal(ordinary.XTRACE_TELEMETRY, '0');
  for (const change of [
    { HOME: '/other' },
    { XTRACE_DATA_DIR: '/other' },
    { XTRACE_TELEMETRY: '0' },
    { NO_PROXY: '*' },
    { no_proxy: 'us.i.posthog.com' },
    { ALL_PROXY: 'http://other' },
    { all_proxy: '' },
    { HTTP_PROXY: env.HTTPS_PROXY },
    { https_proxy: 'http://localhost:12345' },
    { HTTPS_PROXY: 'https://127.0.0.1:12345' },
    { CURL_CA_BUNDLE: '/other' },
    { XTRACE_FIXTURE: 'F1' },
  ])
    assert.throws(() => validateTelemetryEnvironment({ ...env, ...change }, owned, home, 12345));
  const missing = { ...env };
  delete missing.no_proxy;
  assert.throws(() => validateTelemetryEnvironment(missing, owned, home, 12345));
  for (const port of [0, -1, 65536, 1.5, '12345'])
    assert.throws(() => telemetryEnvironment(owned, home, port));
});

test('telemetry DB wrapper retains ordinary integrity/empty-history checks and requires saved facts', async () => {
  const accepted = {
    status: 'passed',
    mode: 'ro',
    mainDatabaseUnchanged: true,
    integrity: ['ok'],
    sessions: 0,
    records: 0,
    telemetry: { preferenceEnabled: false, disclosureShown: true, ledger: { sent: 0 } },
  };
  let args;
  assert.deepEqual(
    await verifyTelemetryDatabase(
      '/source',
      '/owned',
      async (...call) => {
        args = call;
        return JSON.stringify(accepted);
      },
      'notice',
      false,
    ),
    accepted,
  );
  assert.deepEqual(args, [
    'notice-database',
    '/opt/homebrew/bin/python3',
    ['-I', '/source/hosted-database.py', '/owned/xtrace.db', '--telemetry=off'],
  ]);
  for (const changed of [
    { records: 1 },
    { mainDatabaseUnchanged: false },
    { telemetry: undefined },
    { telemetry: { preferenceEnabled: true, disclosureShown: true, ledger: { sent: 0 } } },
    { telemetry: { preferenceEnabled: false, disclosureShown: false, ledger: { sent: 0 } } },
    { telemetry: { preferenceEnabled: false, disclosureShown: true, ledger: { sent: 1 } } },
  ])
    await assert.rejects(
      verifyTelemetryDatabase(
        '/source',
        '/owned',
        async () => JSON.stringify({ ...accepted, ...changed }),
        'notice',
        false,
      ),
    );
});

test('actual readonly telemetry checks accept saved on/off and reject invalid facts without modifying bytes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'telemetry-db-'));
  const python = process.platform === 'darwin' ? '/opt/homebrew/bin/python3' : 'python3';
  const checker = fileURLToPath(new URL('./hosted-database.py', import.meta.url));
  try {
    for (const [name, enabled, disclosure, status, rows, expected] of [
      ['off', 'false', 'true', 'failed', 2, true],
      ['on', 'true', 'true', 'pending', 1, true],
      ['missing', null, 'true', 'failed', 0, false],
      ['bad-bool', '1', 'true', 'failed', 0, false],
      ['undisclosed', 'false', 'false', 'failed', 0, false],
      ['sent', 'false', 'true', 'sent', 1, false],
      ['bad-status', 'false', 'true', 'unknown', 1, false],
      ['over-cap', 'false', 'true', 'failed', 201, false],
    ]) {
      const db = join(root, name + '.db');
      const made = spawnSync(
        python,
        [
          '-I',
          '-c',
          `import sqlite3,sys,json
c=sqlite3.connect(sys.argv[1]); c.executescript("CREATE TABLE sessions(id);CREATE TABLE records(id);CREATE TABLE settings(key TEXT PRIMARY KEY,value_json TEXT);CREATE TABLE telemetry_events(id INTEGER PRIMARY KEY,event,status);")
e,d,s,n=json.loads(sys.argv[2])
if e is not None:c.execute("INSERT INTO settings VALUES('telemetry.enabled',?)",(e,))
c.execute("INSERT INTO settings VALUES('telemetry.disclosure_shown',?)",(d,))
c.executemany("INSERT INTO telemetry_events VALUES(?,'app_opened',?)",[(i+1,s) for i in range(n)])
c.commit();c.close()`,
          db,
          JSON.stringify([enabled, disclosure, status, rows]),
        ],
        { encoding: 'utf8', timeout: 5000 },
      );
      assert.equal(made.status, 0, made.stderr);
      const before = sha256(await readFile(db));
      const result = spawnSync(
        python,
        ['-I', checker, db, enabled === 'true' ? '--telemetry=on' : '--telemetry=off'],
        { encoding: 'utf8', timeout: 5000 },
      );
      const report = JSON.parse(result.stdout);
      assert.equal(result.status, expected ? 0 : 1, result.stderr);
      assert.equal(report.mainDatabaseUnchanged, true);
      assert.equal(sha256(await readFile(db)), before);
      if (expected)
        assert.deepEqual(
          [
            report.telemetry.preferenceEnabled,
            report.telemetry.disclosureShown,
            report.telemetry.ledger.sent,
          ],
          [enabled === 'true', true, 0],
        );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('updater warning requires rejection from the proxy belonging to this phase', () => {
  const evidence = {
    code: 0,
    reason: 'Telemetry UI phase telemetry-notice observed',
    nodes: [],
    telemetry: { expectedUpdaterWarning: true },
  };
  assert.throws(() => acceptTelemetryUI(evidence, { github: 0 }, 'telemetry-notice'));
  assert.equal(
    acceptTelemetryUI(evidence, { github: 1 }, 'telemetry-notice').blockedUpdaterAuthority,
    'github.com:443',
  );
  assert.throws(() => acceptTelemetryUI(evidence, { github: 1 }, 'telemetry-settings-on'));
  assert.equal(
    acceptTelemetryUI(
      { ...evidence, telemetry: { expectedUpdaterWarning: false } },
      { github: 0 },
      'telemetry-notice',
    ).blockedUpdaterAuthority,
    null,
  );
});

test(
  'actual Swift telemetry identity/environment/alert policies and prepared probe typecheck',
  { skip: process.platform !== 'darwin', timeout: 150000 },
  async () => {
    const deadline = performance.now() + 150000;
    const compiler = { retain: false };
    const root = await mkdtemp(join(tmpdir(), 'telemetry-swift-'));
    try {
      const owned = join(root, 'checks.noindex');
      await mkdir(owned);
      const cache = join(owned, 'cache');
      const launcher = await readFile(new URL('./hosted-launch.swift', import.meta.url), 'utf8');
      const driver = await readFile(new URL('./hosted-ax.swift', import.meta.url), 'utf8');
      const block = (text, name) =>
        text
          .split(`// BEGIN ${name}:`)[1]
          .split('\n')
          .slice(1)
          .join('\n')
          .split(`// END ${name}`)[0];
      const envPolicy = block(launcher, 'TELEMETRY ENVIRONMENT POLICY');
      const probe = prepareProbeSource(
        launcher,
        await readFile(new URL('./hosted-telemetry-probe.swift', import.meta.url), 'utf8'),
      );
      const probePath = join(owned, 'probe.swift');
      await writeFile(probePath, probe);
      for (const [path, timeout, label] of [
        [
          fileURLToPath(new URL('./hosted-launch.swift', import.meta.url)),
          60000,
          'telemetry-launcher-typecheck',
        ],
        [
          fileURLToPath(new URL('./hosted-ax.swift', import.meta.url)),
          30000,
          'telemetry-ax-typecheck',
        ],
        [probePath, 30000, 'telemetry-probe-typecheck'],
      ]) {
        const began = performance.now();
        const result = spawnSync(
          '/usr/bin/xcrun',
          ['swiftc', '-typecheck', '-module-cache-path', cache, path],
          { encoding: 'utf8', timeout },
        );
        recordCompiler(compiler, label, timeout, performance.now() - began, result);
      }
      const receipt = launcher.slice(
        launcher.indexOf('struct Receipt:'),
        launcher.indexOf('// BEGIN TRANSITION POLICY'),
      );
      const controls = await readFile(
        new URL('./fixtures/hosted-telemetry-controls.swift', import.meta.url),
        'utf8',
      );
      const source =
        'import AppKit\nimport ApplicationServices\nimport Darwin\n' +
        receipt +
        block(launcher, 'LAUNCH CONTEXT POLICY') +
        envPolicy +
        block(driver, 'UPDATE UI POLICY') +
        block(driver, 'TELEMETRY UI POLICY') +
        controls;
      const path = join(owned, 'main.swift'),
        binary = join(owned, 'controls');
      await writeFile(path, source);
      const compileBegan = performance.now();
      const compile = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', cache, path, '-o', binary],
        { encoding: 'utf8', timeout: 20000 },
      );
      recordCompiler(
        compiler,
        'telemetry-controls-compile',
        20000,
        performance.now() - compileBegan,
        compile,
      );
      const ran = spawnSync(binary, [], { encoding: 'utf8', timeout: 5000 });
      assert.equal(ran.status, 0, ran.stderr);
      assert.match(ran.stdout, /controls passed/);
    } catch (error) {
      diagnoseCompiler(compiler, deadline);
      throw error;
    } finally {
      await cleanupCompiler(root, compiler);
    }
  },
);

test('cleanup retains all files on unknown exit, marks failure before proxy stop and preserves both errors', async () => {
  const session = telemetrySession();
  session.owners.push({ kind: 'telemetry', phase: 'notice', exited: false });
  const calls = [];
  await assert.rejects(
    cleanupTelemetry(session, {
      stopOwners: async () => {
        calls.push('stop-app');
        throw new Error('unknown app exit');
      },
      removeOwned: async () => calls.push('remove'),
      stopProxies: async () => {
        calls.push('stop-proxy');
        assert.equal(session.cleanupFailed, true);
        throw new Error('proxy exit failed');
      },
    }),
    (error) =>
      error.message === 'unknown app exit' && error.proxyCleanupError === 'proxy exit failed',
  );
  assert.deepEqual(calls, ['stop-app', 'stop-proxy']);
  assert.equal(telemetryCleanupFacts(session).appExitUnknown, true);
  assert.equal(telemetryCleanupFacts(session).owners[0].environmentVerified, false);
  const healthy = telemetrySession();
  const order = [];
  await cleanupTelemetry(healthy, {
    stopOwners: async () => order.push('stop-app'),
    removeOwned: async () => order.push('remove'),
    stopProxies: async () => order.push('stop-proxy'),
  });
  assert.deepEqual(order, ['stop-app', 'remove', 'stop-proxy']);
  assert.equal(healthy.cleanupFailed, false);
  await assert.rejects(
    cleanupTelemetry(healthy, {
      stopOwners: async () => {},
      removeOwned: async () => {},
      stopProxies: async () => {
        throw new Error('proxy');
      },
    }),
    /proxy/,
  );
  assert.equal(healthy.cleanupFailed, true);
});
test('telemetry owns only absent default/test profiles and rejects local execution before launch', async () => {
  const paths = telemetryOwnedPaths('/Users/runner');
  assert.equal(paths.length, 12);
  assert.equal(new Set(paths).size, paths.length);
  assert.ok(
    paths.every(
      (path) => path.startsWith('/Users/runner/Library/') && !path.includes('ai.xtrace.desktop'),
    ),
  );
  assert.throws(() => requireTelemetryHost({ mode: 'first-launch' }), /explicit/);
  assert.equal(telemetryBudget.workMs + telemetryBudget.cleanupMs, 21 * 60000);
  const root = await mkdtemp(join(tmpdir(), 'telemetry-denied-'));
  try {
    const run = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('./hosted-first-launch.mjs', import.meta.url)), 'telemetry'],
      { cwd: root, env: { PATH: '/usr/bin:/bin' }, encoding: 'utf8', timeout: 10000 },
    );
    assert.equal(run.status, 1);
    const report = JSON.parse(
      await readFile(join(root, 'artifacts/hosted-telemetry/report.json'), 'utf8'),
    );
    assert.equal(report.status, 'failed');
    assert.equal(report.releasedAppLaunched, false);
    assert.equal(report.freshBaseline, undefined);
    assert.equal(report.telemetryCleanup.owners.length, 0);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('probe cleanup distinguishes stopped parent from receipt-bound curl reaping', async () => {
  const root = await mkdtemp(join(tmpdir(), 'telemetry-cleanup-'));
  try {
    for (const [name, result, clean] of [
      ['success', { pid: 17, passed: true, curlExitConfirmed: true }, true],
      ['qualification-failed', { pid: 17, passed: false, curlExitConfirmed: true }, true],
      ['child-unknown', { pid: 17, passed: false, curlExitConfirmed: false }, false],
      ['wrong-owner', { pid: 18, passed: false, curlExitConfirmed: true }, false],
      ['invalid', { pid: 17, curlExitConfirmed: true }, false],
      ['missing', null, false],
    ]) {
      const receipt = join(root, `${name}.json`);
      await writeFile(receipt, JSON.stringify({ pid: 17 }));
      if (result) await writeFile(`${receipt}.result.json`, JSON.stringify(result));
      const owner = {
        kind: 'probe',
        phase: name,
        receipt,
        environmentFile: '/owned/environment.json',
        exited: false,
      };
      const session = telemetrySession();
      session.owners.push(owner);
      const calls = [];
      const work = cleanupTelemetry(session, {
        stopOwners: () =>
          stopTelemetryOwners(session, {
            helper: '/owned/helper',
            automationEnvironment: {},
            checked: async (label) => {
              calls.push(label);
            },
          }),
        removeOwned: async () => {
          calls.push('database');
          calls.push('remove');
        },
        stopProxies: async () => {
          calls.push('proxy');
        },
      });
      if (clean) await work;
      else await assert.rejects(work);
      assert.equal(owner.exited, true);
      assert.equal(owner.childExitConfirmed, clean);
      assert.equal(calls.filter((value) => value.endsWith('-stop')).length, 1);
      assert.equal(calls.includes('database'), clean);
      assert.equal(calls.includes('remove'), clean);
      assert.equal(calls.at(-1), 'proxy');
      const facts = telemetryCleanupFacts(session);
      assert.equal(facts.probeParentExitUnknown, false);
      assert.equal(facts.probeChildExitUnknown, !clean);
      assert.equal(facts.appAndProfilesRemoved, false);
    }
    const session = telemetrySession();
    session.owners.push({
      kind: 'probe',
      phase: 'missing-receipt',
      receipt: join(root, 'absent'),
      exited: false,
    });
    const calls = [];
    await assert.rejects(
      cleanupTelemetry(session, {
        stopOwners: () =>
          stopTelemetryOwners(session, { checked: async () => calls.push('signal') }),
        removeOwned: async () => calls.push('remove'),
        stopProxies: async () => calls.push('proxy'),
      }),
    );
    assert.deepEqual(calls, ['proxy']);
    await assert.rejects(
      confirmProbeChildExit({ receipt: '/owned' }, async () => Buffer.alloc(16385)),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('proxy exit remains confirmed when final evidence fails, with prior errors retained', async () => {
  for (const failure of ['missing', 'oversized', 'unwritable']) {
    const proxy = { phase: 'notice', directory: '/owned' };
    const session = { proxies: [proxy] };
    let stops = 0;
    const dependencies = {
      stop: async () => {
        stops++;
        return { exitConfirmed: true, exitCode: 0 };
      },
      read: async () => {
        if (failure === 'missing') throw new Error('missing evidence');
        return Buffer.alloc(failure === 'oversized' ? 16385 : 1);
      },
      write: async () => {
        throw new Error('unwritable evidence');
      },
    };
    await assert.rejects(stopTelemetryProxies(session, '/output', dependencies));
    assert.equal(proxy.exit.exitConfirmed, true);
    assert.ok(proxy.evidenceError);
    const first = proxy.evidenceError;
    await assert.rejects(
      stopTelemetryProxies(session, '/output', {
        ...dependencies,
        read: async () => Buffer.from('{}'),
        write: async () => {},
      }),
    );
    assert.equal(stops, 1);
    assert.equal(proxy.evidenceError, first);
    assert.equal(proxy.exit.exitConfirmed, true);
  }
  const proxy = { phase: 'notice', directory: '/owned' };
  await assert.rejects(
    stopTelemetryProxies({ proxies: [proxy] }, '/output', {
      stop: async () => {
        throw new Error('stop failed');
      },
      read: async () => {
        throw new Error('must not read');
      },
    }),
    /stop failed/,
  );
  assert.equal(proxy.exit.exitConfirmed, false);
  assert.equal(proxy.shutdownError, 'stop failed');
  assert.equal(proxy.evidenceError, undefined);
});

test('late profile claims preserve unexpected bytes and never claim the other kind', async () => {
  const root = await mkdtemp(join(tmpdir(), 'telemetry-claims-'));
  try {
    for (const kind of ['probe', 'telemetry']) {
      const home = join(root, kind);
      const session = telemetrySession();
      const paths = telemetryOwnedPaths(home);
      const sentinel = paths[kind === 'probe' ? 6 : 0];
      // Initial absent baseline, then acquisition/probe preparation creates a sentinel.
      await assert.rejects(readFile(join(sentinel, 'sentinel')));
      await mkdir(sentinel, { recursive: true });
      await writeFile(join(sentinel, 'sentinel'), 'not owned');
      let launched = false;
      await assert.rejects(
        (async () => {
          await claimTelemetryProfiles(session, kind, home);
          launched = true;
        })(),
      );
      await removeTelemetryProfiles(session);
      assert.equal(launched, false);
      assert.equal(session.profileClaims.length, 0);
      assert.equal(await readFile(join(sentinel, 'sentinel'), 'utf8'), 'not owned');
      const other = kind === 'probe' ? 'telemetry' : 'probe';
      await claimTelemetryProfiles(session, other, home);
      const claimed = session.profileClaims[0].paths[0];
      await mkdir(claimed, { recursive: true });
      await writeFile(join(claimed, 'owned'), 'owned');
      await claimTelemetryProfiles(session, other, home, async () => {
        throw new Error('Restart must reuse its claim');
      });
      await removeTelemetryProfiles(session);
      await assert.rejects(readFile(join(claimed, 'owned')));
      assert.equal(await readFile(join(sentinel, 'sentinel'), 'utf8'), 'not owned');
      assert.equal(session.profileClaims[0].removed, true);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('polling clips time and checks the deadline observation exactly once', async () => {
  for (const success of [true, false]) {
    let clock = 0,
      final = 0;
    const sleeps = [];
    const budget = executionBudget(() => clock, { workMs: 25, cleanupMs: 50 });
    const result = await pollTelemetry(
      async () => {
        if (clock === 25) final++;
        return success && clock === 25;
      },
      budget,
      100,
      20,
      {
        now: () => clock,
        sleep: async (ms) => {
          assert.ok(ms > 0);
          sleeps.push(ms);
          clock += ms;
        },
      },
    );
    assert.equal(Boolean(result), success);
    assert.deepEqual(sleeps, [20, 5]);
    assert.equal(final, 1);
    assert.throws(() => budget.remaining(1));
    budget.beginCleanup();
    clock += 20;
    budget.beginCleanup();
    assert.equal(budget.remaining(50), 30);
  }
});

test('proxy readiness hands off its exact child before failure and reserves its stop time', async () => {
  const root = await mkdtemp(join(tmpdir(), 'telemetry-start-'));
  try {
    let spawned = 0;
    await assert.rejects(
      startTelemetryProxy(
        join(root, 'too-short'),
        { remaining: () => 8000 },
        {
          launch: () => {
            spawned++;
          },
        },
      ),
      /do not fit/,
    );
    assert.equal(spawned, 0);
    for (const localStopFails of [false, true]) {
      const session = telemetrySession();
      let clock = 0,
        readinessEnd = 0;
      const child = { pid: 77, exitCode: null, signalCode: null, on() {} };
      const budget = executionBudget(() => clock, { workMs: 13000, cleanupMs: 180000 });
      await assert.rejects(
        startTelemetryProxy(join(root, String(localStopFails)), budget, {
          register: (proxy) => session.proxies.push(proxy),
          launch: () => child,
          read: async () => {
            throw Object.assign(new Error('missing'), { code: 'ENOENT' });
          },
          now: () => clock,
          sleep: async (ms) => {
            clock += ms;
          },
          stop: async (proxy, ms) => {
            assert.equal(proxy.child, child);
            assert.equal(session.proxies[0], proxy);
            assert.equal(ms, 8000);
            readinessEnd = clock;
            if (localStopFails) throw new Error('local stop uncertain');
            return { exitConfirmed: true };
          },
        }),
      );
      assert.equal(readinessEnd, 5000);
      assert.equal(session.proxies.length, 1);
      assert.equal(session.proxies[0].child, child);
      budget.beginCleanup();
      const calls = [];
      const cleanup = cleanupTelemetry(session, {
        stopOwners: async () => {},
        removeOwned: async () => {
          if (session.proxies[0].exit?.exitConfirmed !== true)
            throw new Error('retain startup proof');
          calls.push('remove');
          session.appAndProfilesRemoved = true;
        },
        stopProxies: () =>
          stopTelemetryProxies(session, undefined, {
            budget,
            stop: async (proxy, ms) => {
              assert.equal(proxy.child, child);
              assert.ok(ms > 0);
              calls.push('global-stop');
              throw new Error('global stop uncertain');
            },
          }),
      });
      if (localStopFails) {
        await assert.rejects(cleanup, /retain startup proof/);
        assert.deepEqual(calls, ['global-stop']);
        assert.equal(telemetryCleanupFacts(session).proxyExitUnknown, true);
        assert.equal(session.appAndProfilesRemoved, undefined);
      } else {
        await cleanup;
        assert.deepEqual(calls, ['remove']);
        assert.equal(telemetryCleanupFacts(session).proxyExitUnknown, false);
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('caller preserves proxy cleanup time and multiple stops share one remaining reserve', async () => {
  let clock = 0;
  const budget = executionBudget(() => clock, { workMs: 18000, cleanupMs: 180000 });
  budget.beginCleanup();
  const caller = await readFile(new URL('./hosted-first-launch.mjs', import.meta.url), 'utf8');
  const body = caller
    .split('  const run = (command, args, timeout = 30_000, env = systemEnvironment) => {')[1]
    .split('\n  };')[0];
  const invoke = new Function(
    'budget',
    'proxyStopReserve',
    'spawnSync',
    'command',
    'args',
    'timeout',
    'env',
    body,
  );
  clock = 180000 - 16050;
  let timeout;
  invoke(
    budget,
    16000,
    (_command, _args, options) => {
      timeout = options.timeout;
    },
    'owned',
    [],
    100,
    {},
  );
  assert.equal(timeout, 50);
  clock += 50;
  assert.throws(
    () =>
      invoke(
        budget,
        16000,
        () => {
          throw new Error('Must not spawn');
        },
        'owned',
        [],
        100,
        {},
      ),
    /reserved/,
  );
  const limits = [];
  await stopTelemetryProxies({ proxies: [{}, {}] }, undefined, {
    budget,
    stop: async (_proxy, ms) => {
      limits.push(ms);
      clock += ms;
      return { exitConfirmed: true };
    },
  });
  assert.deepEqual(limits, [8000, 8000]);
  assert.throws(() => budget.remaining(1));
  await assert.rejects(stopTelemetryProxy({ child: {} }, 0), /No bounded/);
});

test('actual final caller distinguishes attempted/confirmed launch and uncertain staging removal', async () => {
  const source = await readFile(new URL('./hosted-first-launch.mjs', import.meta.url), 'utf8');
  const start = source.lastIndexOf('  try {\n    if (session) {\n      await cleanupTelemetry');
  const end = source.indexOf("  await writeFile(join(output, 'report.json')", start);
  assert.ok(start > 0 && end > start);
  const actual = new (Object.getPrototypeOf(async function () {}).constructor)(
    'input',
    `
    const {session,cleanupTelemetry,telemetryCleanupFacts,stopTelemetryOwners,stopTelemetryProxies,removeOwned,rm,staging}=input;
    const report={status:'failed',releasedAppLaunched:false};let failure=input.failure;
    const telemetryContext={},output='/owned/output',budget={};let proxyStopReserve=0;
    ${source.slice(start, end)}
    return {report,failure};`,
  );
  const root = await mkdtemp(join(tmpdir(), 'telemetry-report-'));
  try {
    for (const [name, receipt, workError] of [
      ['pre-spawn-deadline', undefined, 'Work budget exhausted'],
      ['missing-receipt', undefined, 'Launch failed'],
      ['malformed', '{', 'Malformed receipt'],
      [
        'wrong-identity',
        {
          pid: 17,
          startTime: 1,
          bundle: '/wrong',
          executable: '/wrong',
          identifier: release.identifier,
        },
        'Wrong identity',
      ],
      [
        'confirmed-ui-failure',
        {
          pid: 17,
          startTime: 1,
          bundle: release.bundle,
          executable: join(release.bundle, 'Contents/MacOS/xtrace-desktop'),
          identifier: release.identifier,
        },
        'UI failed',
      ],
    ]) {
      const session = telemetrySession();
      session.stagingOwned = true;
      const owner = { kind: 'telemetry', phase: 'notice', receipt: join(root, `${name}.json`) };
      session.owners.push(owner);
      if (receipt !== undefined)
        await writeFile(
          owner.receipt,
          typeof receipt === 'string' ? receipt : JSON.stringify(receipt),
        );
      const confirmed = name === 'confirmed-ui-failure';
      if (confirmed) await confirmTelemetryLaunch(owner);
      else await assert.rejects(confirmTelemetryLaunch(owner));
      const result = await actual({
        session,
        cleanupTelemetry,
        telemetryCleanupFacts,
        staging: '/owned/staging',
        failure: new Error(workError),
        stopTelemetryOwners: async () => {
          throw new Error('Exit remains unknown');
        },
        stopTelemetryProxies: async () => {},
        removeOwned: async () => {
          throw new Error('Must retain files');
        },
        rm: async () => {
          throw new Error('Must retain staging');
        },
      });
      assert.equal(result.report.releasedAppLaunched, confirmed ? true : null);
      assert.equal(result.report.releasedAppLaunchAttempted, true);
      assert.equal(result.report.releasedAppLaunchOutcomeUnknown, !confirmed);
      assert.equal(result.report.error, workError);
      assert.equal(result.report.cleanupError, 'Exit remains unknown');
      assert.equal(result.report.telemetryCleanup.cleanupFailed, true);
      assert.equal(result.report.telemetryCleanup.stagingState, 'retained');
    }
    for (const removalFails of [false, true]) {
      const session = telemetrySession();
      session.stagingOwned = true;
      const staging = join(root, String(removalFails));
      await mkdir(staging);
      await writeFile(join(staging, 'proof'), 'proof');
      const result = await actual({
        session,
        cleanupTelemetry,
        telemetryCleanupFacts,
        staging,
        failure: new Error('Earlier work failure'),
        stopTelemetryOwners: async () => {},
        stopTelemetryProxies: async () => {},
        removeOwned: async () => {
          session.appAndProfilesRemoved = true;
        },
        rm: async (path) => {
          await rm(join(path, 'proof'));
          if (removalFails) throw new Error('Partial staging removal failed');
          await rm(path, { recursive: true });
        },
      });
      assert.equal(result.report.error, 'Earlier work failure');
      assert.equal(result.report.releasedAppLaunched, false);
      assert.equal(result.report.telemetryCleanup.cleanupFailed, removalFails);
      assert.equal(
        result.report.telemetryCleanup.stagingState,
        removalFails ? 'unknown' : 'removed',
      );
      assert.equal(result.report.telemetryCleanup.stagingRetained, false);
      assert.equal(result.report.cleanupConfirmed, removalFails ? undefined : true);
      if (removalFails) assert.equal(result.report.cleanupError, 'Partial staging removal failed');
      await assert.rejects(readFile(join(staging, 'proof')));
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('actual caller product exit needs a confirmed launch and every product attempt absent', async () => {
  const source = await readFile(new URL('./hosted-first-launch.mjs', import.meta.url), 'utf8');
  const start = source.lastIndexOf('  try {\n    if (session) {\n      await cleanupTelemetry');
  const end = source.indexOf("  await writeFile(join(output, 'report.json')", start);
  const actual = new (Object.getPrototypeOf(async function () {}).constructor)(
    'input',
    `
    const {session,cleanupTelemetry,telemetryCleanupFacts,stopTelemetryOwners,stopTelemetryProxies,removeOwned,rm}=input;
    const report={status:'failed',releasedAppLaunched:false};let failure=input.failure;
    const telemetryContext={},output='/owned/output',budget={},staging='/owned/staging';let proxyStopReserve=0;
    ${source.slice(start, end)}
    return {report,failure};`,
  );
  const product = (confirmed, exited) => ({
    kind: 'telemetry',
    launchConfirmed: confirmed,
    exited,
  });
  const probe = (exited) => ({ kind: 'probe', exited, childExitConfirmed: exited });
  const observed = [];
  for (const [name, owners, cleanupFailure, expected] of [
    ['zero', [], null, false],
    ['probe-only', [probe(true)], null, false],
    ['unconfirmed-unresolved', [product(false, false)], 'stop', false],
    ['unconfirmed-absent', [product(false, true)], null, false],
    ['confirmed-unresolved', [product(true, false)], 'stop', false],
    ['confirmed-absent', [product(true, true)], null, true],
    ['mixed-unresolved', [product(true, true), product(false, false)], 'stop', false],
    ['mixed-all-absent', [product(true, true), product(false, true)], null, true],
    ['probe-failure-after-product-exit', [product(true, true), probe(false)], 'stop', true],
    ['staging-failure-after-product-exit', [product(true, true)], 'staging', true],
  ]) {
    const session = telemetrySession();
    session.stagingOwned = true;
    session.owners = owners;
    const result = await actual({
      session,
      cleanupTelemetry,
      telemetryCleanupFacts,
      failure: new Error('Earlier work failure'),
      stopTelemetryOwners: async () => {
        if (cleanupFailure === 'stop') throw new Error('Stop uncertain');
      },
      stopTelemetryProxies: async () => {},
      removeOwned: async () => {},
      rm: async () => {
        if (cleanupFailure === 'staging') throw new Error('Partial removal');
      },
    });
    const [launched, productUnknown, probeUnknown, launchUnknown] = {
      zero: [false, false, false, false],
      'probe-only': [false, false, false, false],
      'unconfirmed-unresolved': [null, true, false, true],
      'unconfirmed-absent': [null, false, false, true],
      'confirmed-unresolved': [true, true, false, false],
      'confirmed-absent': [true, false, false, false],
      'mixed-unresolved': [true, true, false, true],
      'mixed-all-absent': [true, false, false, true],
      'probe-failure-after-product-exit': [true, false, true, false],
      'staging-failure-after-product-exit': [true, false, false, false],
    }[name];
    assert.equal(result.report.releasedAppLaunched, launched, name);
    assert.equal(result.report.releasedAppLaunchAttempted, launched !== false, name);
    assert.equal(result.report.releasedAppLaunchOutcomeUnknown, launchUnknown, name);
    assert.equal(result.report.telemetryCleanup.appExitUnknown, productUnknown, name);
    assert.equal(result.report.telemetryCleanup.probeParentExitUnknown, probeUnknown, name);
    assert.equal(result.report.telemetryCleanup.probeChildExitUnknown, probeUnknown, name);
    observed.push({ name, actual: result.report.exitConfirmed, expected });
    assert.equal(result.report.error, 'Earlier work failure');
    assert.equal(result.report.telemetryCleanup.cleanupFailed, Boolean(cleanupFailure));
    assert.equal(
      result.report.telemetryCleanup.stagingState,
      cleanupFailure === 'stop' ? 'retained' : cleanupFailure === 'staging' ? 'unknown' : 'removed',
    );
  }
  assert.deepEqual(
    observed.map(({ name, actual }) => ({ name, value: actual })),
    observed.map(({ name, expected }) => ({ name, value: expected })),
  );
});

test('actual saved-off branch checks and retains its proxy snapshot before enabling', async () => {
  const source = await readFile(new URL('./hosted-telemetry-run.mjs', import.meta.url), 'utf8');
  const start = source.indexOf("    if (phase === 'notice') {");
  const end = source.indexOf('    await stopOwner(owner,', start);
  assert.ok(start > 0 && end > start);
  const actual = new (Object.getPrototypeOf(async function () {}).constructor)(
    'phase',
    'ui',
    'attempt',
    'telemetryProxyState',
    'proxy',
    'phaseReport',
    source.slice(start, end),
  );
  for (const [name, count, error] of [
    ['zero', 0, null],
    ['positive', 1, null],
    ['read-failure', null, 'Proxy observation failed'],
  ]) {
    const calls = [],
      report = {},
      proxy = { phase: 'saved-off' };
    const snapshot = { telemetry: count, github: 1, probe: 2 };
    const run = actual(
      'saved-off',
      async (mode) => calls.push(mode),
      async () => {
        throw new Error('No attempt wait in saved-off');
      },
      async (selected) => {
        assert.equal(selected, proxy);
        calls.push('snapshot');
        if (error) throw new Error(error);
        return snapshot;
      },
      proxy,
      report,
    );
    if (name === 'zero') await run;
    else
      await assert.rejects(
        run,
        error ? /Proxy observation failed/ : /Saved-off startup attempted telemetry/,
      );
    assert.deepEqual(
      calls,
      name === 'zero'
        ? ['telemetry-settings-off', 'snapshot', 'telemetry-settings-enable']
        : ['telemetry-settings-off', 'snapshot'],
    );
    assert.equal(report.beforeEnable, error ? undefined : snapshot);
  }
});
