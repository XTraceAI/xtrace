import assert from 'node:assert/strict';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { test } from 'node:test';
import { smokeEnvironment } from './debug-bundle.mjs';

test('native smoke overrides live paths and clears inherited fixture selection', () => {
  const inherited = {
    XTRACE_DATA_DIR: '/existing-data',
    XTRACE_FIXTURE: 'F1',
    XTRACE_PYTHON: '/some/python',
    PATH: '/tools',
  };
  const env = smokeEnvironment('/owned-test-directory', inherited);
  assert.equal(env.XTRACE_DATA_DIR, join('/owned-test-directory', 'data'));
  assert.equal(env.XTRACE_NATIVE_HOME, join('/owned-test-directory', 'home'));
  assert.equal('XTRACE_FIXTURE' in env, false);
  assert.equal('XTRACE_PYTHON' in env, false);
  assert.equal(env.PATH, inherited.PATH);
  assert.equal(env.GH_TOKEN, '');
  assert.equal(env.GITHUB_TOKEN, '');
  assert.equal(inherited.XTRACE_DATA_DIR, '/existing-data');
  assert.equal(inherited.XTRACE_FIXTURE, 'F1');
});

import { mkdtemp, mkdir, readFile, rm, writeFile, symlink, lstat, rename } from 'node:fs/promises';
import { readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import {
  checkSmokeBundle,
  isSmokeIdentifier,
  inspectorArguments,
  launchSmokeBundle,
  prepareSmokeBundle,
  unregisterSmokeBundle,
  stopSmokeChild,
} from './debug-bundle.mjs';

function fakeChild(kill) {
  const child = new EventEmitter();
  Object.assign(child, { pid: 123, exitCode: null, signalCode: null, kill });
  return child;
}

test('already-exited, signal-exited and failed-spawn children are never signalled', async () => {
  for (const status of [{ exitCode: 0 }, { signalCode: 'SIGTERM' }, { pid: undefined }]) {
    const child = Object.assign(
      fakeChild(() => assert.fail('Must not signal')),
      status,
    );
    await stopSmokeChild(child);
    assert.equal(child.listenerCount('exit'), 0);
  }
});

test('graceful termination waits for exit and clears its escalation timer', async () => {
  const signals = [];
  const child = fakeChild((signal) => {
    signals.push(signal);
    queueMicrotask(() => child.emit('exit', 0, null));
    return true;
  });
  await stopSmokeChild(child, { graceMs: 5, forceMs: 5 });
  await new Promise((resolve) => setTimeout(resolve, 15));
  assert.deepEqual(signals, ['SIGTERM']);
  assert.equal(child.listenerCount('exit'), 0);
});

test('forced termination waits for delayed exit before allowing copy cleanup', async () => {
  const signals = [];
  let notifyForced;
  const forced = new Promise((resolve) => {
    notifyForced = resolve;
  });
  const child = fakeChild((signal) => {
    signals.push(signal);
    if (signal === 'SIGKILL') notifyForced();
    return true;
  });
  let cleanupAllowed = false;
  const stopped = stopSmokeChild(child, { graceMs: 5, forceMs: 1000 }).then(() => {
    cleanupAllowed = true;
  });
  await forced;
  assert.equal(cleanupAllowed, false);
  child.signalCode = 'SIGKILL';
  child.emit('exit', null, 'SIGKILL');
  await stopped;
  assert.equal(cleanupAllowed, true);
  assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
  assert.equal(child.listenerCount('exit'), 0);
});

test('unconfirmed exit fails within bounds even when signals fail or throw', async () => {
  for (const failure of [false, new Error('synthetic failure')]) {
    const signals = [];
    const child = fakeChild((signal) => {
      signals.push(signal);
      if (failure instanceof Error) throw failure;
      return failure;
    });
    let cleanupAllowed = false;
    await assert.rejects(
      stopSmokeChild(child, { graceMs: 5, forceMs: 5 }).then(() => {
        cleanupAllowed = true;
      }),
      /exit was not confirmed/,
    );
    assert.equal(cleanupAllowed, false);
    assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
    assert.equal(child.listenerCount('exit'), 0);
  }
});

test('fixture smoke fixes F1 and disables provider-related background work without changing HOME', () => {
  const inherited = {
    XTRACE_FIXTURE: 'F20',
    XTRACE_PR_AUTO_CHECK: 'on',
    XTRACE_PYTHON: '/live/python',
    HOME: '/original/home',
    GH_TOKEN: 'synthetic',
    GITHUB_TOKEN: 'synthetic',
  };
  const env = smokeEnvironment('/owned/staging.noindex', inherited, { fixtureSmoke: true });
  assert.equal(env.XTRACE_FIXTURE, 'F1');
  assert.equal(env.XTRACE_PR_AUTO_CHECK, 'off');
  assert.equal(env.XTRACE_DATA_DIR, '/owned/staging.noindex/data');
  assert.equal(env.XTRACE_NATIVE_HOME, '/owned/staging.noindex/home');
  assert.equal(env.HOME, inherited.HOME);
  assert.equal('XTRACE_PYTHON' in env, false);
  assert.equal(env.GH_TOKEN, '');
  assert.equal(env.GITHUB_TOKEN, '');
  assert.equal(env.XTRACE_TELEMETRY, '0');
  assert.equal(inherited.XTRACE_FIXTURE, 'F20');
});

async function bundleFixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'smoke-id-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source', 'XTrace.app');
  const directory = join(root, 'copy');
  await mkdir(join(source, 'Contents/MacOS'), { recursive: true });
  await mkdir(directory);
  const plist = join(source, 'Contents/Info.plist');
  const identifier = `ai.xtrace.app.test.${randomUUID()}`;
  await writeFile(plist, identifier);
  await writeFile(join(source, 'Contents/MacOS/xtrace-desktop'), 'original executable');
  const calls = [];
  const run = (command, args) => {
    calls.push([command, ...args]);
    let stdout = '';
    if (args[1]?.startsWith('Set :CFBundleIdentifier '))
      writeFileSync(args[2], args[1].split(' ').at(-1));
    if (args[1] === 'Print :CFBundleIdentifier') stdout = readFileSync(args[2], 'utf8');
    if (args[1] === 'Print :CFBundleExecutable') stdout = 'xtrace-desktop';
    return { status: 0, stdout };
  };
  return { source, directory, plist, calls, run, identifier };
}

test('explicit fixture mode reaches the launched child and reported evidence', async (t) => {
  const f = await bundleFixture(t);
  const calls = [];
  const launched = await launchSmokeBundle(f.source, f.directory, {
    identifier: f.identifier,
    run: f.run,
    fixtureSmoke: true,
    spawnProcess: (command, args, options) => {
      calls.push({ command, args, options });
      return 'synthetic child';
    },
  });
  assert.equal(launched.smokeMode, 'fixture-f1');
  assert.equal(launched.child, 'synthetic child');
  assert.equal(calls.length, 1);
  assert.equal(calls[0].options.env.XTRACE_FIXTURE, 'F1');
  assert.equal(calls[0].options.env.XTRACE_PR_AUTO_CHECK, 'off');
  assert.equal(calls[0].command, join(launched.prepared.bundle, 'Contents/MacOS/xtrace-desktop'));
});

test('unregistering a smoke copy cannot name the original app or a production identity', () => {
  const directory = '/owned/staging.noindex';
  const bundle = join(directory, 'XTrace.app');
  const prepared = { bundle, identifier: 'ai.xtrace.app.test.abc123' };
  const calls = [];
  unregisterSmokeBundle(prepared, directory, (command, args) => {
    calls.push({ command, args });
    return { status: 0 };
  });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].args, ['-u', bundle]);
  for (const invalid of [
    { ...prepared, bundle: '/Applications/XTrace.app' },
    { ...prepared, bundle: '/Applications/XTrace Desktop.app' },
    { ...prepared, identifier: 'ai.xtrace.app' },
  ])
    assert.throws(() =>
      unregisterSmokeBundle(invalid, directory, () => assert.fail('Must not unregister')),
    );
  assert.throws(
    () => unregisterSmokeBundle(prepared, directory, () => ({ status: 1 })),
    /Could not unregister/,
  );
});

test('smoke copies, assigns a unique test ID, signs and verifies without changing the built app', async (t) => {
  const f = await bundleFixture(t);
  const prepared = await prepareSmokeBundle(f.source, f.directory, {
    run: f.run,
    identifier: f.identifier,
  });
  assert.notEqual(prepared.bundle, f.source);
  assert.equal(isSmokeIdentifier(prepared.identifier), true);
  assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  assert.equal(
    await readFile(join(prepared.bundle, 'Contents/Info.plist'), 'utf8'),
    prepared.identifier,
  );
  assert.equal(
    await readFile(join(prepared.bundle, 'Contents/MacOS/xtrace-desktop'), 'utf8'),
    'original executable',
  );
  assert.deepEqual(f.calls.slice(1, 3), [
    ['codesign', '--force', '--deep', '--sign', '-', prepared.bundle],
    ['codesign', '--verify', '--deep', '--strict', prepared.bundle],
  ]);
  const second = join(f.directory, 'second');
  await mkdir(second);
  const next = await prepareSmokeBundle(f.source, second, { run: f.run, identifier: f.identifier });
  assert.equal(prepared.identifier, next.identifier); // Both copies match their compiled source.
  assert.deepEqual(inspectorArguments(42, prepared), ['42', prepared.bundle, prepared.identifier]);
});

test('smoke refuses production IDs and preparation failures before launching any process', async (t) => {
  for (const identifier of [
    'ai.xtrace.app',
    'ai.xtrace.desktop',
    'ai.xtrace.app.test',
    'ai.xtrace.app.test.bad/id',
  ]) {
    assert.equal(isSmokeIdentifier(identifier), false);
    assert.throws(() => inspectorArguments(42, { bundle: '/copy/app', identifier }));
    const f = await bundleFixture(t);
    await assert.rejects(
      launchSmokeBundle(f.source, f.directory, {
        identifier,
        run: f.run,
        spawnProcess: () => assert.fail('Must not spawn'),
      }),
    );
    assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  }
  for (const failAt of [0, 1, 2, 3, 4]) {
    const f = await bundleFixture(t);
    let step = 0;
    await assert.rejects(
      launchSmokeBundle(f.source, f.directory, {
        identifier: f.identifier,
        run: (command, args) => (step++ === failAt ? { status: 1 } : f.run(command, args)),
        spawnProcess: () => assert.fail('Must not spawn'),
      }),
    );
    assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  }
});

test('smoke refuses a source whose identity differs before any copy or spawn', async (t) => {
  const f = await bundleFixture(t);
  await assert.rejects(
    launchSmokeBundle(f.source, f.directory, {
      identifier: f.identifier,
      run: (command, args) =>
        args[1] === 'Print :CFBundleIdentifier'
          ? { status: 0, stdout: 'ai.xtrace.app' }
          : f.run(command, args),
      spawnProcess: () => assert.fail('Must not spawn'),
    }),
    /expected test identity/,
  );
});

test('relative plist and executable links become independent files in the smoke copy', async (t) => {
  const f = await bundleFixture(t);
  const realPlist = join(f.source, 'Contents/real.plist');
  const executable = join(f.source, 'Contents/MacOS/xtrace-desktop');
  const realExecutable = join(f.source, 'Contents/MacOS/real-executable');
  await rename(f.plist, realPlist);
  await symlink('real.plist', f.plist);
  await rename(executable, realExecutable);
  await symlink('real-executable', executable);
  const prepared = await prepareSmokeBundle(f.source, f.directory, {
    run: f.run,
    identifier: f.identifier,
  });
  assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  assert.equal(await readFile(realPlist, 'utf8'), f.identifier);
  assert.equal((await lstat(f.plist)).isSymbolicLink(), true);
  assert.equal((await lstat(executable)).isSymbolicLink(), true);
  assert.equal((await lstat(join(prepared.bundle, 'Contents/Info.plist'))).isSymbolicLink(), false);
  assert.equal(
    (await lstat(join(prepared.bundle, 'Contents/MacOS/xtrace-desktop'))).isSymbolicLink(),
    false,
  );
  assert.equal(await readFile(realExecutable, 'utf8'), 'original executable');
});

test('compiled native inspector accepts test IDs and immediately rejects production or malformed IDs', async (t) => {
  if (process.platform !== 'darwin') return t.skip('Native inspector requires macOS');
  const root = await mkdtemp(join(tmpdir(), 'smoke-inspector-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const exe = join(root, 'inspect');
  const compile = spawnSync('swiftc', ['scripts/ci/debug-bundle.swift', '-o', exe], {
    encoding: 'utf8',
  });
  assert.equal(compile.status, 0, compile.stderr);
  for (const identifier of ['ai.xtrace.app', 'ai.xtrace.desktop', 'ai.xtrace.app.test', 'bad']) {
    assert.equal(spawnSync(exe, ['2147483647', '/no/test/app', identifier]).status, 2);
  }
  // An accepted ID enters the inspection loop rather than returning its
  // validation error. No process with this impossible PID is ever launched.
  const accepted = spawnSync(exe, ['2147483647', '/no/test/app', 'ai.xtrace.app.test.abc123'], {
    timeout: 250,
  });
  assert.equal(accepted.status, null);
  assert.equal(accepted.signal, 'SIGTERM');
  // The released app changed identity; every launcher path now supplies the
  // compiled test ID rather than relying on the old implicit production ID.
  assert.equal(spawnSync(exe, ['2147483647', '/no/test/app']).status, 2);
});

test('a copied identity changed during signing is refused before spawn', async (t) => {
  const f = await bundleFixture(t);
  await assert.rejects(
    launchSmokeBundle(f.source, f.directory, {
      identifier: f.identifier,
      run: (command, args) =>
        args[1] === 'Print :CFBundleIdentifier' && args[2] !== f.plist
          ? { status: 0, stdout: 'ai.xtrace.app' }
          : f.run(command, args),
      spawnProcess: () => assert.fail('Must not spawn'),
    }),
    /identity mismatch/,
  );
});

for (const mode of ['debug', 'release', 'fixture']) {
  test(`${mode} smoke unregisters only its staged copy after confirmed exit`, async (t) => {
    const f = await bundleFixture(t);
    const events = [];
    let copy;
    let child;
    let spawned;
    const result = checkSmokeBundle(f.source, f.identifier, {
      fixtureSmoke: mode === 'fixture',
      temporaryRoot: f.directory,
      run: (command, args) => {
        if (command.endsWith('/inspect')) events.push('inspect');
        if (command.endsWith('/lsregister')) {
          assert.deepEqual(args, ['-u', copy]);
          assert.deepEqual(events, ['inspect', 'SIGTERM', 'exit']);
          assert.equal(readFileSync(join(copy, 'Contents/Info.plist'), 'utf8'), f.identifier);
          events.push('unregister');
        }
        return f.run(command, args);
      },
      spawnProcess: (command, args, options) => {
        copy = command.slice(0, command.indexOf('/Contents/MacOS/'));
        assert.match(copy, /\/staging\.noindex\/XTrace\.app$/);
        assert.equal(options.env.XTRACE_FIXTURE, mode === 'fixture' ? 'F1' : undefined);
        assert.equal(options.env.XTRACE_DATA_DIR, join(copy, '..', 'data'));
        assert.equal(options.env.XTRACE_NATIVE_HOME, join(copy, '..', 'home'));
        child = fakeChild((signal) => {
          events.push(signal);
          queueMicrotask(() => {
            events.push('exit');
            child.exitCode = 0;
            child.emit('exit', 0, null);
          });
        });
        spawned = setImmediate(() => child.emit('spawn'));
        return child;
      },
    });
    t.after(() => clearImmediate(spawned));
    await result;
    assert.deepEqual(events, ['inspect', 'SIGTERM', 'exit', 'unregister']);
    await assert.rejects(lstat(copy), { code: 'ENOENT' });
    assert.equal(await readFile(f.plist, 'utf8'), f.identifier);
  });

  test(`${mode} smoke retains its copy and skips unregister when child exit is unconfirmed`, async (t) => {
    const f = await bundleFixture(t);
    const signals = [];
    let copy;
    let spawned;
    await assert.rejects(
      checkSmokeBundle(f.source, f.identifier, {
        fixtureSmoke: mode === 'fixture',
        temporaryRoot: f.directory,
        stopOptions: { graceMs: 5, forceMs: 5 },
        run: (command, args) => {
          assert.ok(!command.endsWith('/lsregister'), 'Must not unregister a running app');
          return f.run(command, args);
        },
        spawnProcess: (command) => {
          copy = command.slice(0, command.indexOf('/Contents/MacOS/'));
          const child = fakeChild((signal) => {
            signals.push(signal);
            return false;
          });
          spawned = setImmediate(() => child.emit('spawn'));
          return child;
        },
      }),
      /exit was not confirmed/,
    );
    t.after(() => clearImmediate(spawned));
    assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
    assert.equal(await readFile(join(copy, 'Contents/Info.plist'), 'utf8'), f.identifier);
  });
}

test('a failed inspection still stops the child before unregistering and removing its copy', async (t) => {
  const f = await bundleFixture(t);
  const events = [];
  let copy;
  await assert.rejects(
    checkSmokeBundle(f.source, f.identifier, {
      temporaryRoot: f.directory,
      run: (command, args) => {
        if (command.endsWith('/inspect')) return { status: 1 };
        if (command.endsWith('/lsregister')) {
          assert.deepEqual(events, ['exit']);
          events.push('unregister');
        }
        return f.run(command, args);
      },
      spawnProcess: (command) => {
        copy = command.slice(0, command.indexOf('/Contents/MacOS/'));
        const child = fakeChild(() => {
          queueMicrotask(() => {
            events.push('exit');
            child.emit('exit', 0, null);
          });
        });
        setImmediate(() => child.emit('spawn'));
        return child;
      },
    }),
  );
  assert.deepEqual(events, ['exit', 'unregister']);
  await assert.rejects(lstat(copy), { code: 'ENOENT' });
});
