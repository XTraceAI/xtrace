import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { firstLaunchEnvironment, sha256 } from './hosted-release.mjs';
import {
  telemetryEnvironment,
  validateTelemetryEnvironment,
  telemetryDataDirectory,
  verifyTelemetryDatabase,
} from './hosted-telemetry.mjs';
import { prepareProbeSource } from './hosted-telemetry-proxy.mjs';
import { telemetryOwnedPaths } from './hosted-telemetry.mjs';

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

test(
  'actual Swift telemetry identity/environment policies and prepared probe typecheck',
  { skip: process.platform !== 'darwin', timeout: 150000 },
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'telemetry-swift-'));
    try {
      const owned = join(root, 'checks.noindex');
      await mkdir(owned);
      const cache = join(owned, 'cache');
      const launcher = await readFile(new URL('./hosted-launch.swift', import.meta.url), 'utf8');
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
      for (const [path, timeout] of [
        [fileURLToPath(new URL('./hosted-launch.swift', import.meta.url)), 60000],
        [probePath, 30000],
      ]) {
        const result = spawnSync(
          '/usr/bin/xcrun',
          ['swiftc', '-typecheck', '-module-cache-path', cache, path],
          { encoding: 'utf8', timeout },
        );
        assert.equal(result.status, 0, result.stderr);
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
        controls;
      const path = join(owned, 'main.swift'),
        binary = join(owned, 'controls');
      await writeFile(path, source);
      const compile = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', cache, path, '-o', binary],
        { encoding: 'utf8', timeout: 20000 },
      );
      assert.equal(compile.status, 0, compile.stderr);
      const ran = spawnSync(binary, [], { encoding: 'utf8', timeout: 5000 });
      assert.equal(ran.status, 0, ran.stderr);
      assert.match(ran.stdout, /controls passed/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  },
);

test('foundation owns only the default product and fixed harmless probe profiles', () => {
  const paths = telemetryOwnedPaths('/Users/runner');
  assert.equal(paths.length, 12);
  assert.equal(new Set(paths).size, 12);
  assert.ok(
    paths.every(
      (path) => path.startsWith('/Users/runner/Library/') && !path.includes('ai.xtrace.desktop'),
    ),
  );
});
