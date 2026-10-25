import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { stopSmokeChild } from './debug-bundle.mjs';
import { guardProcessDetails } from './hosted-guard-diagnostics.mjs';
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
export async function pollTelemetry(
  check,
  budget,
  timeout,
  interval = 20,
  { now = () => performance.now(), sleep = delay } = {},
) {
  const deadline = now() + budget.remaining(timeout);
  for (;;) {
    const value = await check();
    if (value) return value;
    const left = deadline - now();
    if (left <= 0) return null;
    const pause = Math.min(left, budget.remaining(interval));
    if (pause <= 0) throw new Error('Telemetry wait budget exhausted');
    await sleep(pause);
  }
}
export async function startTelemetryProxy(
  directory,
  budget = { remaining: (ms) => ms },
  {
    register = () => {},
    launch = spawn,
    stop = stopTelemetryProxy,
    read = readFile,
    now = () => performance.now(),
    sleep = delay,
  } = {},
) {
  await mkdir(directory, { mode: 0o700 });
  const available = budget.remaining(13000);
  if (available <= 8000)
    throw new Error('Proxy readiness and emergency stop do not fit the work budget');
  const child = launch(
    process.platform === 'darwin' ? '/opt/homebrew/bin/python3' : 'python3',
    ['-I', fileURLToPath(new URL('./hosted-telemetry-proxy.py', import.meta.url)), directory],
    { env: systemEnvironment, stdio: ['pipe', 'ignore', 'ignore'] },
  );
  child.on('error', () => {});
  const proxy = { child, directory, pid: child.pid, ready: false };
  register(proxy);
  const readiness = {
    remaining(ms) {
      const left = budget.remaining(ms + 8000) - 8000;
      if (left <= 0) throw new Error('Proxy readiness reserve reached');
      return Math.min(ms, left);
    },
  };
  try {
    const ready = await pollTelemetry(
      async () => {
        if (child.exitCode !== null || child.signalCode !== null || child.pid === undefined)
          throw new Error('Rejecting proxy did not start');
        try {
          const value = JSON.parse(await read(join(directory, 'ready.json'), 'utf8'));
          if (
            value.pid !== child.pid ||
            value.host !== '127.0.0.1' ||
            ![value.port, value.deadPort].every(
              (port) => Number.isInteger(port) && port > 0 && port < 65536,
            ) ||
            value.port === value.deadPort
          )
            throw new Error('Rejecting proxy identity/ports differ from its owned child');
          return value;
        } catch (error) {
          if (error.code !== 'ENOENT') throw error;
          return null;
        }
      },
      readiness,
      available - 8000,
      20,
      { now, sleep },
    );
    if (!ready) throw new Error('Rejecting proxy readiness timed out');
    Object.assign(proxy, { port: ready.port, deadPort: ready.deadPort, ready: true });
    return proxy;
  } catch (error) {
    proxy.startupError = error.message;
    try {
      proxy.exit = await stop(proxy, budget.remaining(8000));
    } catch (stopError) {
      proxy.shutdownError = stopError.message;
      error.proxyCleanupError = stopError.message;
    }
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
  {
    spawnSync: runCurl = spawnSync,
    telemetryProxyState: readState = telemetryProxyState,
    guardProcessDetails: formatDetails = guardProcessDetails,
  } = {},
) {
  validateTelemetryEnvironment(env, owned, home, proxy.port, environmentOff);
  const before = await readState(proxy);
  let elapsedMs, timeoutMs;
  const run = (launchEnv) => {
    timeoutMs = budget.remaining(17000);
    const began = performance.now();
    const result = runCurl('/usr/bin/curl', telemetryCurlArguments, {
      env: launchEnv,
      input: '{"probe":true}',
      encoding: 'utf8',
      timeout: timeoutMs,
      maxBuffer: 16384,
    });
    elapsedMs = performance.now() - began;
    return result;
  };
  const failure = (message, label, result, configuredProxy) => {
    const error = new Error(message);
    try {
      error.message += `; ${JSON.stringify({ configuredProxy, ...formatDetails(label, timeoutMs, elapsedMs, result) })}`;
    } catch {
      /* Diagnostic failure must not replace the guard error. */
    }
    return error;
  };
  const live = run(env);
  if (live.error || live.signal || live.status !== 56 || live.stdout !== '000')
    throw failure(
      'System curl did not confirm explicit CONNECT rejection',
      'live-curl',
      live,
      env.HTTPS_PROXY,
    );
  const after = await pollTelemetry(
    async () => {
      const state = await readState(proxy);
      return state.probe > before.probe ? state : null;
    },
    budget,
    2500,
  );
  if (!after) throw new Error('Rejected curl did not reach the owned proxy');
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
    throw failure(
      'Unavailable proxy did not fail through the owned loopback endpoint',
      'dead-curl',
      dead,
      deadEnv.HTTPS_PROXY,
    );
  await readState(proxy);
  return {
    connectRejected: true,
    liveCurlStatus: live.status,
    deadProxyRejected: true,
    deadCurlStatus: dead.status,
    noTLSOrBodyAccepted: true,
    productionProviderContacted: false,
  };
}
export async function stopTelemetryProxy(proxy, maxMs = 8000) {
  if (!Number.isFinite(maxMs) || maxMs < 2) throw new Error('No bounded proxy stop time remains');
  const graceMs = Math.min(5000, Math.floor(maxMs) - 1);
  await stopSmokeChild(proxy.child, { graceMs, forceMs: Math.floor(maxMs) - graceMs });
  return {
    pid: proxy.pid,
    exitCode: proxy.child.exitCode,
    signal: proxy.child.signalCode,
    exitConfirmed: true,
  };
}

export function prepareProbeSource(policy, template) {
  if (
    !policy.trim() ||
    !template.includes('// __TELEMETRY_ENVIRONMENT_POLICY__') ||
    !template.includes('__CURL_ARGUMENTS__')
  )
    throw new Error('Fixed probe source inputs are missing');
  return template
    .replace('// __TELEMETRY_ENVIRONMENT_POLICY__', () => policy)
    .replace('__CURL_ARGUMENTS__', JSON.stringify(telemetryCurlArguments));
}
