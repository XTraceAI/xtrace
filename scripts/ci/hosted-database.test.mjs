import { updateControl } from './fixtures/hosted-update-controls.mjs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { verifyHostedDatabase } from './hosted-release.mjs';

const python = '/opt/homebrew/bin/python3';
const helper = fileURLToPath(new URL('./hosted-database.py', import.meta.url));
const env = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const accepted = {
  status: 'passed',
  mode: 'ro',
  mainDatabaseUnchanged: true,
  integrity: ['ok'],
  sessions: 0,
  records: 0,
};

test('shared database wrapper uses one fixed isolated reader and propagates a missing tool without fallback', async () => {
  const calls = [];
  const result = await verifyHostedDatabase('/source', '/owned', async (...args) => {
    calls.push(args);
    return JSON.stringify(accepted);
  });
  assert.deepEqual(result, accepted);
  assert.deepEqual(calls, [
    ['database', python, ['-I', '/source/hosted-database.py', '/owned/xtrace.db']],
  ]);
  let attempts = 0;
  await assert.rejects(
    verifyHostedDatabase('/source', '/owned', async () => {
      attempts++;
      throw Object.assign(new Error('spawn ENOENT'), { code: 'ENOENT' });
    }),
    /ENOENT/,
  );
  assert.equal(attempts, 1);
  for (const changed of [
    { status: 'failed' },
    { mode: 'rw' },
    { mainDatabaseUnchanged: false },
    { sessions: 1 },
    { records: 1 },
    { integrity: ['ok', 'error'] },
    { integrity: [] },
  ]) {
    await assert.rejects(
      verifyHostedDatabase('/source', '/owned', async () =>
        JSON.stringify({ ...accepted, ...changed }),
      ),
    );
  }
  await assert.rejects(verifyHostedDatabase('/source', '/owned', async () => 'not json'));
});

test('update database acceptance follows normal exit and failed exit prevents database reads', async () => {
  const success = updateControl({ defaultDatabase: true });
  await success.run();
  const firstRead = success.calls.indexOf('database');
  assert.ok(firstRead > success.calls.indexOf('earlier-absent'));
  assert.ok(success.calls.lastIndexOf('database') > success.calls.indexOf('absent'));
  assert.equal(success.report.database.mode, 'ro');
  for (const failAt of ['earlier-quit', 'earlier-absent', 'quit', 'absent']) {
    const control = updateControl({ failAt, defaultDatabase: true });
    await assert.rejects(control.run(), new RegExp(failAt));
    assert.equal(
      control.calls.filter((name) => name === 'database').length,
      ['quit', 'absent'].includes(failAt) ? 1 : 0,
    );
  }
});

// The first-launch caller remains guarded as a fixed hosted entrypoint.
test('first-launch database acceptance retains confirmed-exit and isolated-reader wiring', async () => {
  const source = await readFile(new URL('./hosted-first-launch.mjs', import.meta.url), 'utf8');
  assert.ok(
    source.indexOf('report.exitConfirmed = true') < source.indexOf('await verifyHostedDatabase('),
  );
  assert.doesNotMatch(source, /PRAGMA integrity_check|\/usr\/bin\/sqlite3/);
});

let available = true;
try {
  await access(python);
} catch {
  available = false;
}
test(
  'actual isolated readonly checker keeps main bytes and rejects corrupt, missing, nonzero and pending-WAL data',
  { skip: !available, timeout: 30000 },
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'hosted-database-check-'));
    try {
      for (const [name, extra, unclean, expected] of [
        ['empty', '', false, true],
        ['sessions', 'INSERT INTO sessions VALUES(1);', false, false],
        ['records', 'INSERT INTO records VALUES(1);', false, false],
        ['pending-wal', 'INSERT INTO records VALUES(1);', true, false],
        ['missing-table', 'DROP TABLE records;', false, false],
        ['corrupt', '', false, false],
      ]) {
        const directory = join(root, name);
        await mkdir(directory);
        const database = join(directory, 'xtrace #?.db');
        const sql =
          'PRAGMA journal_mode=WAL; CREATE TABLE sessions(id); CREATE TABLE records(id);' + extra;
        const made = spawnSync(
          python,
          [
            '-I',
            '-c',
            'import os,sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.executescript(sys.argv[2]);' +
              (unclean ? 'os._exit(0)' : 'c.close()'),
            database,
            sql,
          ],
          { env, encoding: 'utf8', timeout: 5000 },
        );
        assert.equal(made.status, 0, made.stderr);
        if (!unclean) assert.deepEqual(await readdir(directory), ['xtrace #?.db']);
        if (name === 'corrupt') {
          const bytes = await readFile(database);
          Buffer.from('broken database!').copy(bytes);
          await writeFile(database, bytes);
        }
        const before = hash(await readFile(database));
        const checked = spawnSync(python, ['-I', helper, database], {
          env,
          encoding: 'utf8',
          timeout: 10000,
        });
        const result = JSON.parse(checked.stdout);
        assert.equal(checked.status, expected ? 0 : 1, checked.stderr);
        assert.equal(result.status, expected ? 'passed' : 'failed');
        assert.equal(result.mode, 'ro');
        assert.equal(result.mainDatabaseUnchanged, true);
        assert.equal(result.mainSha256Before, before);
        assert.equal(hash(await readFile(database)), before);
        assert.ok(result.python && result.sqlite);
        assert.ok(result.filesBefore.database.sha256 && result.filesAfter.database.sha256);
        if (expected) assert.deepEqual(result.integrity, ['ok']);
        if (name === 'pending-wal') {
          assert.ok(result.filesBefore.wal.size > 0);
          assert.equal(result.records, 1, 'committed WAL rows must not be ignored');
        }
        if (name === 'missing-table') assert.match(result.error, /no such table/);
        if (name === 'corrupt') assert.match(result.error, /not a database/);
      }
      const missing = spawnSync(python, ['-I', helper, join(root, 'missing.db')], {
        env,
        encoding: 'utf8',
      });
      assert.equal(missing.status, 1);
      assert.match(JSON.parse(missing.stdout).error, /Missing or unsafe database/);
      const link = join(root, 'link.db');
      await symlink(join(root, 'empty/xtrace #?.db'), link);
      const linked = spawnSync(python, ['-I', helper, link], { env, encoding: 'utf8' });
      assert.equal(linked.status, 1);
      assert.match(JSON.parse(linked.stdout).error, /unsafe database/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  },
);
