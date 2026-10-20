import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile, realpath } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { telemetryEnvironment } from './hosted-telemetry.mjs';
import {
  startTelemetryProxy,
  qualifyTelemetryProxy,
  telemetryProxyState,
  stopTelemetryProxy,
} from './hosted-telemetry-proxy.mjs';

test(
  'owned rejecting proxy qualifies exact system curl and dead-proxy failure without forwarding',
  { skip: process.platform !== 'darwin', timeout: 30000 },
  async () => {
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
    } finally {
      if (proxy) await stopTelemetryProxy(proxy);
      await rm(root, { recursive: true, force: true });
    }
  },
);
