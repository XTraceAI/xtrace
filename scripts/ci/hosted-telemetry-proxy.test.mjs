import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile, realpath } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { guardProcessDetails, guardFailureMetadata } from './hosted-guard-diagnostics.mjs';
import { telemetryEnvironment, validateTelemetryEnvironment } from './hosted-telemetry.mjs';
import {
  startTelemetryProxy,
  telemetryCurlArguments,
  pollTelemetry,
  qualifyTelemetryProxy,
  telemetryProxyState,
  stopTelemetryProxy,
} from './hosted-telemetry-proxy.mjs';

test(
  'owned rejecting proxy qualifies exact system curl and dead-proxy failure without forwarding',
  { skip: process.platform !== 'darwin', timeout: 30000 },
  async () => {
    const deadline = performance.now() + 30000;
    const root = await realpath(await mkdtemp(join(tmpdir(), 'telemetry-proxy-')));
    let proxy;
    try {
      proxy = await startTelemetryProxy(join(root, 'proxy.noindex'));
      const env = telemetryEnvironment(root, homedir(), proxy.port);
      const result = await qualifyTelemetryProxy(proxy, env, root, homedir());
      assert.equal(result.connectRejected, true);
      assert.equal(result.deadProxyRejected, true);
      assert.equal(result.noTLSOrBodyAccepted, true);
      const state = await telemetryProxyState(proxy);
      assert.ok(state.probe >= 1);
      assert.equal(state.telemetry, 0);
      assert.equal(state.github, 0);
      const before = state.probe;
      await assert.rejects(
        qualifyTelemetryProxy(proxy, { ...env, NO_PROXY: '*' }, root, homedir()),
      );
      assert.equal((await telemetryProxyState(proxy)).probe, before);
      const exit = await stopTelemetryProxy(proxy);
      assert.equal(exit.exitConfirmed, true);
      await assert.rejects(telemetryProxyState(proxy), /exited/);
      assert.equal(
        JSON.parse(await readFile(join(proxy.directory, 'state.json'), 'utf8')).stopped,
        true,
      );
    } catch (error) {
      try {
        const reserveMs =
          proxy && proxy.child.exitCode === null && proxy.child.signalCode === null ? 8000 : 0;
        console.log(
          JSON.stringify({ curlMetadata: guardFailureMetadata('curl', deadline, { reserveMs }) }),
        );
      } catch {
        /* Preserve the qualification error even when metadata fails. */
      }
      throw error;
    } finally {
      if (proxy) await stopTelemetryProxy(proxy);
      await rm(root, { recursive: true, force: true });
    }
  },
);

async function controlledQualification(source, live, dead, format = guardProcessDetails) {
  const start = source.indexOf('export async function qualifyTelemetryProxy(');
  const end = source.indexOf('export async function stopTelemetryProxy(', start);
  assert.ok(start >= 0 && end > start);
  let reads = 0;
  const results = [live, dead];
  const raw = results.map((result) => ({ ...result }));
  const calls = [];
  const run = (_program, args, options) => {
    calls.push({ args, options });
    return results.shift();
  };
  const qualify = new Function(
    'spawnSync',
    'telemetryCurlArguments',
    'validateTelemetryEnvironment',
    'telemetryProxyState',
    'pollTelemetry',
    'guardProcessDetails',
    `${source.slice(start, end).replace(/^export /, '')};return qualifyTelemetryProxy;`,
  )(
    run,
    telemetryCurlArguments,
    validateTelemetryEnvironment,
    async () => ({ probe: reads++ }),
    pollTelemetry,
    format,
  );
  let error;
  try {
    await qualify(
      { port: 12345, deadPort: 12346 },
      telemetryEnvironment('/owned', '/Users/runner', 12345),
      '/owned',
      '/Users/runner',
    );
  } catch (caught) {
    error = caught;
  }
  assert.deepEqual(live, raw[0]);
  assert.deepEqual(dead, raw[1]);
  assert.ok(
    calls.every(
      (call) =>
        call.options.timeout === 17000 &&
        call.options.maxBuffer === 16384 &&
        call.options.input === '{"probe":true}',
    ),
  );
  assert.ok(
    calls.every((call) => JSON.stringify(call.args) === JSON.stringify(telemetryCurlArguments)),
  );
  return error;
}

test('actual qualification keeps the route decision while adding separate bounded details', async () => {
  const source = await readFile(new URL('./hosted-telemetry-proxy.mjs', import.meta.url), 'utf8');
  const live = { pid: 10, status: 56, signal: null, stdout: '000', stderr: 'CONNECT response 403' };
  const dead = {
    pid: 11,
    status: 7,
    signal: null,
    stdout: '000',
    stderr: 'Failed to connect to 127.0.0.1',
  };
  for (const [kind, change, accepted] of [
    ['dead', {}, true],
    ['dead', { status: 28 }, true],
    ['dead', { status: 28, stderr: 'Connection timed out after 15005 milliseconds' }, false],
    ['dead', { status: 6 }, false],
    ['dead', { stdout: '200' }, false],
    [
      'dead',
      { status: null, error: Object.assign(new Error('timeout'), { code: 'ETIMEDOUT' }) },
      false,
    ],
    ['dead', { status: null, signal: 'SIGTERM' }, false],
    ['live', { status: 7 }, false],
    ['live', { stdout: '200' }, false],
    ['live', { signal: 'SIGTERM' }, false],
    ['live', { error: Object.assign(new Error('launch'), { code: 'ENOENT' }) }, false],
  ]) {
    const error = await controlledQualification(
      source,
      kind === 'live' ? { ...live, ...change } : live,
      kind === 'dead' ? { ...dead, ...change } : dead,
    );
    assert.equal(!error, accepted);
    if (error) {
      assert.match(
        error.message,
        kind === 'live'
          ? /^System curl did not confirm explicit CONNECT rejection/
          : /^Unavailable proxy did not fail through the owned loopback endpoint/,
      );
      assert.ok(error.message.length < 17000);
    }
  }
  const ipless = { ...dead, status: 28, stderr: 'Connection timed out after 15005 milliseconds' };
  const error = await controlledQualification(source, live, ipless);
  assert.ok(error.message.includes('http://127.0.0.1:12346'));
  assert.equal(ipless.stderr.includes('127.0.0.1'), false);
  assert.match(error.message, /Connection timed out/);
  const unavailable = await controlledQualification(source, live, ipless, () => {
    throw new Error('formatter failed');
  });
  assert.equal(
    unavailable.message,
    'Unavailable proxy did not fail through the owned loopback endpoint',
  );
});

test('selected process details are bounded, nonmutating and exclude extra values and paths', () => {
  const error = Object.assign(new Error('compile /Users/private user/source.swift:12:3: failed'), {
    code: 'ETIMEDOUT',
    errno: -1,
    syscall: 'spawnSync /Users/private/source',
    extra: { body: 'do-not-log', argv: ['do-not-log'], environment: { TOKEN: 'do-not-log' } },
  });
  Object.freeze(error.extra);
  Object.freeze(error);
  const raw = Object.freeze({
    pid: 22,
    status: null,
    signal: 'SIGTERM',
    error,
    stdout: '\u0001'.repeat(10000),
    stderr: 'x'.repeat(10000),
    extra: 'do-not-log',
  });
  const result = guardProcessDetails('compile', 20000, 20003, raw);
  const encoded = JSON.stringify(result);
  assert.ok(encoded.length < 16384);
  assert.equal(result.stdout.truncated, true);
  assert.equal(result.stderr.truncated, true);
  assert.equal(result.error.code.text, 'ETIMEDOUT');
  assert.equal(result.status, null);
  assert.equal(result.pid, 22);
  assert.ok(!encoded.includes('do-not-log'));
  assert.ok(!encoded.includes('/Users/'));
  assert.ok(!encoded.includes('private user'));
  assert.equal(raw.stderr.length, 10000);
  assert.equal(raw.error, error);
});

test('fixed metadata commands share one clipped deadline and preserve bounded failures', () => {
  let clock = 1000;
  const calls = [];
  const result = guardFailureMetadata('swift', 10000, {
    now: () => clock,
    run: (program, args, options) => {
      calls.push({ program, args, options });
      clock += Math.min(600, options.timeout);
      return {
        pid: 20,
        status: 0,
        signal: null,
        stdout:
          args[0] === '-p'
            ? '/Users/private user/Xcode.app/Contents/Developer\n'
            : 'x'.repeat(10000),
        stderr: '',
      };
    },
  });
  assert.deepEqual(
    calls.map((call) => call.options.timeout),
    [1500, 900, 300],
  );
  assert.equal(clock, 2500);
  assert.ok(calls.every((call) => call.options.maxBuffer === 4096));
  assert.equal(result.uncertainCompletion, false);
  assert.ok(result.records[0].stdout.truncated);
  assert.ok(!JSON.stringify(result).includes('/Users/'));
  assert.ok(!JSON.stringify(result).includes('private user'));
  assert.deepEqual(
    calls.map(({ program, args }) => [program, args]),
    [
      ['/usr/bin/xcrun', ['swiftc', '--version']],
      ['/usr/bin/xcrun', ['--show-sdk-version']],
      ['/usr/bin/xcode-select', ['-p']],
    ],
  );
  let spawned = 0;
  for (const deadline of [clock, clock - 1, clock + 8000, NaN]) {
    const skipped = guardFailureMetadata('curl', deadline, {
      reserveMs: 8000,
      now: () => clock,
      run: () => {
        spawned++;
      },
    });
    assert.equal(skipped.records.length, 0);
    assert.equal(skipped.skipped.length, 1);
  }
  assert.equal(spawned, 0);
  const clipped = guardFailureMetadata('curl', clock + 8500, {
    reserveMs: 8000,
    now: () => clock,
    run: (_program, _args, options) => {
      assert.equal(options.timeout, 500);
      clock += 500;
      return {
        pid: 21,
        status: null,
        signal: 'SIGTERM',
        error: Object.assign(new Error('timed out'), { code: 'ETIMEDOUT' }),
        stdout: '',
        stderr: '',
      };
    },
  });
  assert.equal(clipped.uncertainCompletion, true);
  assert.equal(clipped.records[0].error.code.text, 'ETIMEDOUT');
  assert.equal(
    guardFailureMetadata('unsupported', 999999, {
      run: () => {
        throw new Error('not allowed');
      },
    }).records.length,
    0,
  );
});

test('both actual compiler assertion/cleanup paths preserve failure and uncertain proof', async () => {
  for (const filename of ['hosted-stop.test.mjs', 'hosted-telemetry.test.mjs']) {
    const source = await readFile(new URL(`./${filename}`, import.meta.url), 'utf8');
    const block = source
      .split('// BEGIN COMPILER DIAGNOSTICS:')[1]
      .split('\n')
      .slice(1)
      .join('\n')
      .split('// END COMPILER DIAGNOSTICS')[0];
    for (const [result, uncertain] of [
      [{ status: 0 }, false],
      [{ status: 1 }, false],
      [{ status: null, error: { code: 'ETIMEDOUT' } }, true],
      [{ status: null, signal: 'SIGTERM' }, true],
    ]) {
      for (const metadataMode of ['normal', 'throw', 'uncertain', 'malformed']) {
        let metadataCalls = 0;
        const removed = [],
          logs = [];
        const owner = { retain: false };
        const functions = new Function(
          'assert',
          'guardProcessDetails',
          'guardFailureMetadata',
          'console',
          'basename',
          'rm',
          `${block};return {recordCompiler,diagnoseCompiler,cleanupCompiler};`,
        )(
          assert,
          guardProcessDetails,
          () => {
            metadataCalls++;
            if (metadataMode === 'throw') throw new Error('secondary metadata failure');
            if (metadataMode === 'malformed') return undefined;
            return { uncertainCompletion: metadataMode === 'uncertain' };
          },
          { log: (value) => logs.push(value) },
          (value) => value.split('/').at(-1),
          async (path) => removed.push(path),
        );
        let primary;
        try {
          functions.recordCompiler(owner, 'controlled-compile', 20000, 20001, result);
        } catch (error) {
          primary = error;
          functions.diagnoseCompiler(owner, 99999);
          functions.diagnoseCompiler(owner, 99999);
        }
        await functions.cleanupCompiler('/owned/checks.noindex', owner);
        assert.equal(Boolean(primary), result.status !== 0);
        if (primary) assert.equal(primary.actual, result.status);
        assert.equal(metadataCalls, result.status === 0 ? 0 : 1);
        const retained = uncertain || (result.status !== 0 && metadataMode !== 'normal');
        assert.equal(owner.retain, retained);
        assert.equal(removed.length, retained ? 0 : 1);
        if (retained)
          assert.ok(logs.some((line) => line.includes('"compilerDescendants":"unconfirmed"')));
      }
    }
  }
});

test('real proxy failure handler preserves its original error and outstanding stop reserve', async () => {
  const source = await readFile(
    new URL('./hosted-telemetry-proxy.test.mjs', import.meta.url),
    'utf8',
  );
  const start = source.indexOf('    } catch (error) {');
  const end = source.indexOf('    } finally {', start);
  const actual = new (Object.getPrototypeOf(async function () {}).constructor)(
    'guardFailureMetadata',
    'console',
    'deadline',
    'proxy',
    'error',
    source.slice(start + '    } catch (error) {'.length, end),
  );
  for (const exitCode of [null, 0]) {
    for (const broken of [false, true]) {
      const original = new Error(
        'Unavailable proxy did not fail through the owned loopback endpoint',
      );
      let calls = 0;
      await assert.rejects(
        actual(
          (kind, deadline, options) => {
            calls++;
            assert.equal(kind, 'curl');
            assert.equal(deadline, 30000);
            assert.equal(options.reserveMs, exitCode === null ? 8000 : 0);
            if (broken) throw new Error('metadata failed');
            return {};
          },
          {
            log: () => {
              if (broken) throw new Error('logging failed');
            },
          },
          30000,
          { child: { exitCode, signalCode: null } },
          original,
        ),
        (error) => error === original,
      );
      assert.equal(calls, 1);
      assert.equal(
        original.message,
        'Unavailable proxy did not fail through the owned loopback endpoint',
      );
    }
  }
});

test('developer selection retains known installation kind without revealing paths', () => {
  for (const [path, kind] of [
    ['/Library/Developer/CommandLineTools\n', 'CommandLineTools'],
    ['/Applications/Xcode_16.4.app/Contents/Developer\n', 'Xcode'],
    ['/Users/private user/custom.app/Contents/Developer\n', 'other'],
  ]) {
    const metadata = guardFailureMetadata('swift', 10000, {
      now: () => 0,
      run: () => ({ status: 0, stdout: path, stderr: '' }),
    });
    assert.equal(metadata.records[2].stdout.text, `<developer-selection:${kind}>`);
    assert.ok(!JSON.stringify(metadata).includes('/Users/'));
    assert.ok(!JSON.stringify(metadata).includes('private user'));
  }
});

test('path-bearing diagnostic lines are removed whole without leaking spaced-name fragments', () => {
  const reviewer =
    'xcrun: error: invalid active developer path (/Users/private user/Xcode.app), missing xcrun at: /Users/private user/Xcode.app/usr/bin/xcrun';
  for (const unsafe of [
    reviewer,
    'error: unable to open output file /Users/private user/controls: Permission denied',
    'error: unable to open output file "/Users/private user/controls": Permission denied',
    "error: cannot read '/Users/private user/controls'",
    '(/Users/private user/Build (old)/controls) and /Users/private user/other',
    '/Users/private user/one.swift:12:3: warning: example',
    '/Users/private user/one\n/Users/private user/two',
  ]) {
    const stderr = `safe first line\n${unsafe}\nsafe final line`;
    const raw = Object.freeze({
      pid: 22,
      status: 1,
      signal: null,
      stdout: stderr,
      stderr,
      error: Object.freeze({ code: 'ETIMEDOUT', message: stderr }),
    });
    const record = guardProcessDetails('synthetic', 1500, 1499, raw);
    const metadata = guardFailureMetadata('swift', 10000, { now: () => 0, run: () => raw });
    const encoded = JSON.stringify({ record, metadata });
    assert.ok(!encoded.includes('private'));
    assert.ok(!encoded.includes(' user'));
    assert.ok(!encoded.includes('Xcode.app'));
    assert.ok(!encoded.includes('controls'));
    assert.ok(!encoded.includes('Build (old)'));
    assert.equal(
      record.stderr.text,
      `safe first line\n${unsafe
        .split('\n')
        .map(() => '<path-bearing line redacted>')
        .join('\n')}\nsafe final line`,
    );
    assert.equal(record.status, 1);
    assert.equal(record.elapsedMs, 1499);
    assert.equal(record.error.code.text, 'ETIMEDOUT');
    assert.equal(raw.stderr, stderr);
    assert.equal(raw.error.message, stderr);
    assert.ok(metadata.records[0].stderr.text.startsWith('safe first line\n'));
    assert.ok(metadata.records[0].stderr.text.endsWith('\nsafe final line'));
  }
  const safe = 'curl 8.7.1 libcurl/8.7.1\nConnection timed out after 15005 milliseconds';
  assert.equal(
    guardProcessDetails('safe', 1500, 1, { status: 28, stderr: safe }).stderr.text,
    safe,
  );
  const latePath = 'x'.repeat(2000) + ' /Users/private user/source';
  const bounded = guardProcessDetails('late-path', 1500, 1, { status: 1, stderr: latePath });
  assert.equal(bounded.stderr.text, '<path-bearing line redacted>');
});
