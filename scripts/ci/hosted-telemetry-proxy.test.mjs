import { compilerControls, reportProxyFailure } from './hosted-compiler-controls.mjs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { mkdtemp, rm, readFile, realpath } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { guardProcessDetails, guardFailureMetadata } from './hosted-guard-diagnostics.mjs';
import { telemetryEnvironment } from './hosted-telemetry.mjs';
import {
  startTelemetryProxy,
  telemetryCurlArguments,
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
      assert.equal(result.deadCurlStatus, 7);
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
      reportProxyFailure(error, proxy, deadline);
    } finally {
      if (proxy) await stopTelemetryProxy(proxy);
      await rm(root, { recursive: true, force: true });
    }
  },
);

async function controlledQualification(live, dead, format = guardProcessDetails) {
  let reads = 0;
  const results = [live, dead];
  const raw = results.map((result) => ({ ...result }));
  const calls = [];
  const run = (_program, args, options) => {
    calls.push({ args, options });
    return results.shift();
  };
  let error;
  try {
    await qualifyTelemetryProxy(
      { port: 12345, deadPort: 12346 },
      telemetryEnvironment('/owned', '/Users/runner', 12345),
      '/owned',
      '/Users/runner',
      false,
      undefined,
      {
        spawnSync: run,
        telemetryProxyState: async () => ({ probe: reads++ }),
        guardProcessDetails: format,
      },
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
  const error = await controlledQualification(live, ipless);
  assert.ok(error.message.includes('http://127.0.0.1:12346'));
  assert.equal(ipless.stderr.includes('127.0.0.1'), false);
  assert.match(error.message, /Connection timed out/);
  const unavailable = await controlledQualification(live, ipless, () => {
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

test('shared compiler assertion/cleanup paths preserve failure and uncertain proof', async () => {
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
      const functions = compilerControls({
        guardFailureMetadata: () => {
          metadataCalls++;
          if (metadataMode === 'throw') throw new Error('secondary metadata failure');
          if (metadataMode === 'malformed') return undefined;
          return { uncertainCompletion: metadataMode === 'uncertain' };
        },
        console: { log: (value) => logs.push(value) },
        rm: async (path) => removed.push(path),
      });
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
});

test('real proxy failure handler preserves its original error and outstanding stop reserve', async () => {
  for (const exitCode of [null, 0]) {
    for (const broken of [false, true]) {
      const original = new Error(
        'Unavailable proxy did not fail through the owned loopback endpoint',
      );
      let calls = 0;
      await assert.rejects(
        Promise.resolve().then(() =>
          reportProxyFailure(original, { child: { exitCode, signalCode: null } }, 30000, {
            guardFailureMetadata: (kind, deadline, options) => {
              calls++;
              assert.equal(kind, 'curl');
              assert.equal(deadline, 30000);
              assert.equal(options.reserveMs, exitCode === null ? 8000 : 0);
              if (broken) throw new Error('metadata failed');
              return {};
            },
            console: {
              log: () => {
                if (broken) throw new Error('logging failed');
              },
            },
          }),
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

test(
  'actual worker reserves an exclusive unavailable port and closes sockets on setup failure',
  { timeout: 15000 },
  () => {
    const source = fileURLToPath(new URL('./hosted-telemetry-proxy.py', import.meta.url));
    const control = String.raw`
import errno,json,runpy,socket,sys,tempfile
from pathlib import Path
worker=runpy.run_path(sys.argv[1])
original=socket.socket
rows=[]
for mode in ['normal','connect-error','accept-error','mismatched-peer','competing-bind','wrong-bind-error','main-normal','main-setup-error','main-publish-error']:
    created=[];listened=[];next_role=iter(['listener','target','parking','collision'] if mode.startswith('main-') else ['target','parking','collision'])
    class Tracked:
        def __init__(self,base,role):self.base=base;self.role=role;created.append(self)
        def __getattr__(self,name):return getattr(self.base,name)
        def __enter__(self):return self
        def __exit__(self,*args):self.base.close()
        def listen(self,backlog):
            listened.append(self.role)
            assert self.role in ['listener','parking']
            return self.base.listen(backlog)
        def connect(self,endpoint):
            assert self.role=='target' and endpoint[0]=='127.0.0.1'
            if mode in ['connect-error','main-setup-error']:raise OSError(errno.ECONNREFUSED,'controlled setup failure')
            return self.base.connect(endpoint)
        def accept(self):
            if mode=='accept-error':raise OSError(errno.ECONNABORTED,'controlled accept failure')
            return self.base.accept()
        def getpeername(self):
            if mode=='mismatched-peer' and self.role=='target':return ('127.0.0.1',1)
            return self.base.getpeername()
        def bind(self,endpoint):
            assert endpoint[0]=='127.0.0.1'
            if self.role=='collision' and mode=='competing-bind':return
            if self.role=='collision' and mode=='wrong-bind-error':raise OSError(errno.EACCES,'controlled unexpected error')
            return self.base.bind(endpoint)
    def factory(*args,**kwargs):return Tracked(original(*args,**kwargs),'peer' if 'fileno' in kwargs else next(next_role))
    socket.socket=factory
    failed=False
    try:
        if mode.startswith('main-'):
            with tempfile.TemporaryDirectory(suffix='.noindex') as directory:
                root=Path(directory).resolve();old_argv=sys.argv;sys.argv=['worker',str(root)]
                publish=worker['main'].__globals__['publish']
                if mode=='main-publish-error':
                    def broken(path,value):
                        if path.name=='ready.json':raise OSError('controlled publish failure')
                        publish(path,value)
                    worker['main'].__globals__['publish']=broken
                try:worker['main']()
                except OSError:failed=True
                finally:sys.argv=old_argv;worker['main'].__globals__['publish']=publish
                assert (root/'ready.json').exists()==(mode=='main-normal')
                if mode=='main-normal':assert json.loads((root/'state.json').read_text())['stopped'] is True
        else:
            try:target,peer=worker['reserve_unavailable_port']()
            except (OSError,ValueError):failed=True
            if mode=='normal':
                assert not failed and listened==['parking']
                assert target.getsockname()==peer.getpeername() and target.getpeername()==peer.getsockname()
                assert all(sock.fileno()==-1 for sock in created if sock.role in ['parking','collision'])
                with original() as collision:
                    try:collision.bind(target.getsockname());raise AssertionError('target port became unowned')
                    except OSError as error:assert error.errno==errno.EADDRINUSE
                with original() as client:
                    client.settimeout(0.5)
                    try:client.connect(target.getsockname());raise AssertionError('target accepted a connection')
                    except OSError as error:assert error.errno==errno.ECONNREFUSED
                for sock in [target,peer]:
                    sock.setblocking(False)
                    try:sock.recv(1);raise AssertionError('reservation exchanged data or closed early')
                    except BlockingIOError:pass
                target.close();peer.close()
        assert failed==(mode not in ['normal','main-normal'])
        assert all(sock.fileno()==-1 for sock in created),mode
        rows.append(dict(mode=mode,failedAsExpected=failed,created=len(created),allSocketsClosed=True,targetNeverListened='target' not in listened))
    finally:
        socket.socket=original
        for sock in created:sock.close()
print(json.dumps(rows))
`;
    const began = performance.now();
    const result = spawnSync(
      process.platform === 'darwin' ? '/opt/homebrew/bin/python3' : 'python3',
      ['-I', '-c', control, source],
      { encoding: 'utf8', timeout: 12000, killSignal: 'SIGKILL', maxBuffer: 16384 },
    );
    assert.equal(
      result.status,
      0,
      JSON.stringify(
        guardProcessDetails('reservation-controls', 12000, performance.now() - began, result),
      ),
    );
    const rows = JSON.parse(result.stdout);
    assert.equal(rows.length, 9);
    assert.ok(rows.every((row) => row.allSocketsClosed && row.targetNeverListened));
    console.log(JSON.stringify({ reservationControls: rows }));
  },
);
