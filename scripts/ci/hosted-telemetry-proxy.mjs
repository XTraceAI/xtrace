import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { stopSmokeChild } from './debug-bundle.mjs';
import { validateTelemetryEnvironment } from './hosted-telemetry.mjs';

export const telemetryCurlArguments = Object.freeze([
  '-q',
  '--silent',
  '--show-error',
  '--proto',
  '=https',
  '--tlsv1.2',
  '--max-time',
  '15',
  '--output',
  '/dev/null',
  '--write-out',
  '%{http_code}',
  '--header',
  'Content-Type: application/json',
  '--data-binary',
  '@-',
  'https://capture.invalid/batch/',
]);
const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
export async function startTelemetryProxy(directory, budget = { remaining: (ms) => ms }) {
  await mkdir(directory, { mode: 0o700 });
  const child = spawn(
    process.platform === 'darwin' ? '/opt/homebrew/bin/python3' : 'python3',
    ['-I', fileURLToPath(new URL('./hosted-telemetry-proxy.py', import.meta.url)), directory],
    { env: systemEnvironment, stdio: ['pipe', 'ignore', 'ignore'] },
  );
  child.on('error', () => {});
  try {
    const deadline = Date.now() + budget.remaining(5000);
    while (Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null || child.pid === undefined)
        throw new Error('Rejecting proxy did not start');
      try {
        const ready = JSON.parse(await readFile(join(directory, 'ready.json'), 'utf8'));
        if (
          ready.pid !== child.pid ||
          ready.host !== '127.0.0.1' ||
          ![ready.port, ready.deadPort].every(
            (port) => Number.isInteger(port) && port > 0 && port < 65536,
          ) ||
          ready.port === ready.deadPort
        )
          throw new Error('Rejecting proxy identity/ports differ from its owned child');
        return { child, directory, pid: ready.pid, port: ready.port, deadPort: ready.deadPort };
      } catch (error) {
        if (error.code !== 'ENOENT') throw error;
      }
      await delay(20);
    }
    throw new Error('Rejecting proxy readiness timed out');
  } catch (error) {
    await stopSmokeChild(child);
    throw error;
  }
}
export async function telemetryProxyState(proxy) {
  if (proxy.child.exitCode !== null || proxy.child.signalCode !== null)
    throw new Error('Rejecting proxy exited before app cleanup');
  const bytes = await readFile(join(proxy.directory, 'state.json'));
  if (bytes.length > 16384) throw new Error('Proxy observation exceeded its bound');
  const state = JSON.parse(bytes);
  if (
    state.pid !== proxy.pid ||
    state.stopped !== false ||
    !['probe', 'telemetry', 'github', 'unexpected', 'incomplete', 'afterRejectBytes'].every(
      (key) => Number.isSafeInteger(state[key]) && state[key] >= 0,
    ) ||
    state.unexpected ||
    state.incomplete ||
    state.afterRejectBytes
  )
    throw new Error('Rejecting proxy observed an unsupported or incomplete request');
  return state;
}
export async function qualifyTelemetryProxy(
  proxy,
  env,
  owned,
  home,
  environmentOff = false,
  budget = { remaining: (ms) => ms },
) {
  validateTelemetryEnvironment(env, owned, home, proxy.port, environmentOff);
  const before = await telemetryProxyState(proxy);
  const run = (launchEnv) =>
    spawnSync('/usr/bin/curl', telemetryCurlArguments, {
      env: launchEnv,
      input: '{"probe":true}',
      encoding: 'utf8',
      timeout: budget.remaining(17000),
      maxBuffer: 16384,
    });
  const live = run(env);
  if (live.error || live.signal || live.status !== 56 || live.stdout !== '000')
    throw new Error('System curl did not confirm explicit CONNECT rejection');
  const deadline = Date.now() + budget.remaining(2500);
  let after;
  do {
    after = await telemetryProxyState(proxy);
    if (after.probe > before.probe) break;
    await delay(20);
  } while (Date.now() < deadline);
  if (after.probe <= before.probe) throw new Error('Rejected curl did not reach the owned proxy');
  const deadEnv = {
    ...env,
    HTTPS_PROXY: `http://127.0.0.1:${proxy.deadPort}`,
    https_proxy: `http://127.0.0.1:${proxy.deadPort}`,
  };
  validateTelemetryEnvironment(deadEnv, owned, home, proxy.deadPort, environmentOff);
  const dead = run(deadEnv);
  if (
    dead.error ||
    dead.signal ||
    ![7, 28].includes(dead.status) ||
    dead.stdout !== '000' ||
    !dead.stderr.includes('127.0.0.1')
  )
    throw new Error('Unavailable proxy did not fail through the owned loopback endpoint');
  await telemetryProxyState(proxy);
  return {
    connectRejected: true,
    liveCurlStatus: live.status,
    deadProxyRejected: true,
    deadCurlStatus: dead.status,
    noTLSOrBodyAccepted: true,
    productionProviderContacted: false,
  };
}
export async function stopTelemetryProxy(proxy) {
  await stopSmokeChild(proxy.child, { graceMs: 5000, forceMs: 3000 });
  return {
    pid: proxy.pid,
    exitCode: proxy.child.exitCode,
    signal: proxy.child.signalCode,
    exitConfirmed: true,
  };
}

export function prepareProbeSource(helper, template) {
  const policy = helper.match(
    /\/\/ BEGIN TELEMETRY ENVIRONMENT POLICY:[^\n]*\n([\s\S]*?)\/\/ END TELEMETRY ENVIRONMENT POLICY/,
  );
  if (
    !policy ||
    !template.includes('// __TELEMETRY_ENVIRONMENT_POLICY__') ||
    !template.includes('__CURL_ARGUMENTS__')
  )
    throw new Error('Fixed probe source markers are missing');
  return template
    .replace('// __TELEMETRY_ENVIRONMENT_POLICY__', policy[1])
    .replace('__CURL_ARGUMENTS__', JSON.stringify(telemetryCurlArguments));
}
