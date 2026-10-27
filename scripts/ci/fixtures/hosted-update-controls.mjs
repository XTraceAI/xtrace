import assert from 'node:assert/strict';
import { previousRelease } from '../hosted-update-checks.mjs';
import { runUpdateLifecycle } from '../hosted-update-lifecycle.mjs';

export function updateControl({ failAt, prepare = async () => {}, defaultDatabase = false } = {}) {
  const calls = [];
  const report = { quarantine: {} };
  let inventoryReads = 0;
  const inventoryPhases = ['restarted', 'afterQuit', 'reopened'];
  const record = (name) => {
    calls.push(name);
    if (name === failAt) throw new Error(name);
  };
  const options = {
    report,
    helper: 'helper',
    receipt: 'original',
    successorReceipt: 'successor',
    reopenReceipt: 'reopen',
    environmentFile: 'environment',
    automationEnvironment: {},
    source: '/source',
    env: { XTRACE_DATA_DIR: '/owned/data', TMPDIR: '/owned/tmp' },
    prepare,
    checked: async (name, executable, args) => {
      if (executable === 'helper') {
        if (['stop', 'earlier-stop'].includes(args[0]))
          throw new Error('Successful update must use normal quit');
        record(args[0]);
      } else {
        record(name);
      }
      return JSON.stringify({
        status: 'passed',
        mode: 'ro',
        mainDatabaseUnchanged: true,
        integrity: ['ok'],
        sessions: 0,
        records: 0,
      });
    },
    observe: async (mode) => record(mode),
    databaseIdentity: async () => {
      record('databaseIdentity');
      return { ino: 1 };
    },
    dataKept: async (step) => record(`data:${step}`),
    markAttempt: (kind) => record(`attempt:${kind}`),
    appIdentity: async (bundle, _version, phase) => {
      assert.equal(bundle, previousRelease.bundle);
      record(`identity:${phase}`);
    },
    validateSignature: async (_bundle, name) => record(`signature:${name}`),
    quarantineAt: async (_bundle, name) => record(`quarantine:${name}`),
  };
  const effects = {
    readFile: async (path) => {
      record(`read:${path}`);
      return JSON.stringify({ pid: { original: 1, successor: 2, reopen: 3 }[path] });
    },
    readdir: async (path) => {
      record(`entries:${path}`);
      return path === '/Applications' ? ['XTrace.app'] : [];
    },
    verifyPreviousInventory: async () => {
      record('previousInventory');
      return { old: true };
    },
    verifyAppInventory: async (bundle) => {
      assert.equal(bundle, previousRelease.bundle);
      assert.ok(inventoryReads < inventoryPhases.length);
      record(`inventory:${inventoryPhases[inventoryReads++]}`);
      return { candidate: true };
    },
    ...(!defaultDatabase
      ? {
          verifyHostedDatabase: async () => {
            record('database');
            return { valid: true };
          },
        }
      : {}),
  };
  return { calls, report, options, effects, run: () => runUpdateLifecycle(options, effects) };
}
