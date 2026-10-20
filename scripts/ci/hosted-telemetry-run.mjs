import { lstat, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { appInventory, release, verifyAppInventory } from './hosted-release.mjs';
import {
  acceptTelemetryUI,
  telemetryDataDirectory,
  telemetryEnvironment,
  telemetryOwnedPaths,
  validateTelemetryEnvironment,
  verifyTelemetryDatabase,
} from './hosted-telemetry.mjs';
import {
  qualifyTelemetryProxy,
  startTelemetryProxy,
  stopTelemetryProxy,
  prepareProbeSource,
  pollTelemetry,
  telemetryProxyState,
} from './hosted-telemetry-proxy.mjs';

const registry =
  '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister';
const phases = ['notice', 'saved-off', 'environment-off', 'saved-on'];
export function telemetrySession() {
  return { proxies: [], owners: [], phases: [], profileClaims: [], cleanupFailed: false };
}
async function readJSON(path) {
  return JSON.parse(await readFile(path, 'utf8'));
}
async function waitFor(check, budget, timeout, description) {
  const value = await pollTelemetry(check, budget, timeout, 40);
  if (!value) throw new Error(`${description} was not confirmed within its deadline`);
  return value;
}
export async function claimTelemetryProfiles(
  session,
  kind,
  home,
  exists = async (path) => {
    try {
      await lstat(path);
      return true;
    } catch (error) {
      if (error.code === 'ENOENT') return false;
      throw error;
    }
  },
) {
  if (!['probe', 'telemetry'].includes(kind)) throw new Error('Unknown profile owner');
  if (session.profileClaims.some((claim) => claim.kind === kind)) return;
  const paths = telemetryOwnedPaths(home).slice(
    kind === 'probe' ? 6 : 0,
    kind === 'probe' ? 12 : 6,
  );
  for (const path of paths)
    if (await exists(path)) throw new Error('Unclaimed profile appeared before its owned launch');
  session.profileClaims.push({ kind, paths, removed: false });
}
export async function removeTelemetryProfiles(session, remove = rm) {
  for (const claim of session.profileClaims) {
    for (const path of claim.paths) await remove(path, { recursive: true, force: true });
    claim.removed = true;
  }
}
async function prepareProbe({ source, owned, checked, compilerEnvironment }) {
  const bundle = join(owned, 'Telemetry Proxy Probe.app');
  await mkdir(join(bundle, 'Contents/MacOS'), { recursive: true });
  const helper = await readFile(join(source, 'hosted-launch.swift'), 'utf8');
  const template = await readFile(join(source, 'hosted-telemetry-probe.swift'), 'utf8');
  const swift = join(owned, 'telemetry-probe.swift');
  await writeFile(swift, prepareProbeSource(helper, template), { flag: 'wx' });
  await writeFile(
    join(bundle, 'Contents/Info.plist'),
    `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>ai.xtrace.app.test.telemetry-proxy</string><key>CFBundleExecutable</key><string>TelemetryProxyProbe</string><key>CFBundleName</key><string>Telemetry Proxy Probe</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>`,
    { flag: 'wx' },
  );
  await checked(
    'telemetry-probe-compile',
    '/usr/bin/xcrun',
    [
      'swiftc',
      '-module-cache-path',
      join(owned, 'compiler-cache'),
      swift,
      '-o',
      join(bundle, 'Contents/MacOS/TelemetryProxyProbe'),
    ],
    120_000,
    compilerEnvironment,
  );
  await checked('telemetry-probe-sign', '/usr/bin/codesign', ['--sign', '-', bundle]);
  await checked('telemetry-probe-verify', '/usr/bin/codesign', [
    '--verify',
    '--deep',
    '--strict',
    bundle,
  ]);
  return { bundle, inventory: await appInventory(bundle) };
}
async function sameProbe(probe) {
  if (JSON.stringify(await appInventory(probe.bundle)) !== JSON.stringify(probe.inventory))
    throw new Error('Fixed LaunchServices probe changed after preparation');
}
async function stopOwner(owner, checked, helper, automationEnvironment, label) {
  if (owner.exited) return;
  await lstat(owner.receipt); // No inferred owner or broad process termination if launch failed.
  await checked(
    `${label}-stop`,
    helper,
    [`${owner.kind}-stop`, owner.receipt, owner.environmentFile],
    20_000,
    automationEnvironment,
  );
  await checked(
    `${label}-absent`,
    helper,
    [`${owner.kind}-absent`, owner.receipt, owner.environmentFile],
    10_000,
    automationEnvironment,
  );
  owner.exited = true;
}
export async function confirmTelemetryLaunch(owner) {
  const receipt = await readJSON(owner.receipt);
  if (
    !Number.isSafeInteger(receipt.pid) ||
    receipt.pid <= 0 ||
    !Number.isSafeInteger(receipt.startTime) ||
    receipt.startTime <= 0 ||
    receipt.bundle !== release.bundle ||
    receipt.executable !== join(release.bundle, 'Contents/MacOS/xtrace-desktop') ||
    receipt.identifier !== release.identifier
  )
    throw new Error('Telemetry launch receipt did not confirm the fixed product identity');
  owner.launchConfirmed = true;
  return receipt;
}
export async function confirmProbeChildExit(owner, read = readFile) {
  try {
    const receiptBytes = await read(owner.receipt);
    const resultBytes = await read(`${owner.receipt}.result.json`);
    if (receiptBytes.length > 16384 || resultBytes.length > 16384)
      throw new Error('Probe ownership/result exceeded its bound');
    const receipt = JSON.parse(receiptBytes),
      result = JSON.parse(resultBytes);
    if (
      !Number.isSafeInteger(receipt.pid) ||
      receipt.pid <= 0 ||
      result.pid !== receipt.pid ||
      typeof result.passed !== 'boolean' ||
      result.curlExitConfirmed !== true
    )
      throw new Error('Probe result did not confirm its owned curl child exit');
    owner.childExitConfirmed = true;
  } catch (error) {
    owner.childExitConfirmed = false;
    owner.childExitError = error.message;
    throw error;
  }
}
export async function stopTelemetryOwners(session, context) {
  for (const owner of [...session.owners].reverse()) {
    await stopOwner(
      owner,
      context.checked,
      context.helper,
      context.automationEnvironment,
      `cleanup-${owner.kind}-${owner.phase}`,
    );
    if (owner.kind === 'probe') await confirmProbeChildExit(owner);
  }
}
export async function stopTelemetryProxies(
  session,
  output,
  {
    stop = stopTelemetryProxy,
    read = readFile,
    write = writeFile,
    budget = { remaining: (ms) => ms },
  } = {},
) {
  const errors = [];
  for (const proxy of session.proxies) {
    if (proxy.exit?.exitConfirmed !== true) {
      try {
        proxy.exit = await stop(proxy, budget.remaining(8000));
      } catch (error) {
        proxy.exit = { exitConfirmed: false };
        proxy.shutdownError ||= error.message;
      }
    }
    if (output && proxy.exit?.exitConfirmed === true) {
      try {
        const bytes = await read(join(proxy.directory, 'state.json'));
        if (bytes.length > 16384) throw new Error('Final proxy observations exceeded their bound');
        await write(join(output, `proxy-${proxy.phase}.json`), bytes);
      } catch (error) {
        proxy.evidenceError ||= error.message;
      }
    }
    for (const error of [proxy.shutdownError, proxy.evidenceError]) if (error) errors.push(error);
  }
  if (errors.length) throw new Error(errors.join('; '));
}
export function telemetryCleanupFacts(session) {
  const stagingState = !session.stagingOwned
    ? 'not-owned'
    : session.stagingRemoved
      ? 'removed'
      : session.stagingRemovalAttempted
        ? 'unknown'
        : 'retained';
  return {
    appExitUnknown: session.owners.some((owner) => owner.kind === 'telemetry' && !owner.exited),
    probeParentExitUnknown: session.owners.some((owner) => owner.kind === 'probe' && !owner.exited),
    probeChildExitUnknown: session.owners.some(
      (owner) => owner.kind === 'probe' && owner.childExitConfirmed !== true,
    ),
    appAndProfilesRemoved: session.appAndProfilesRemoved === true,
    stagingState,
    stagingRetained: stagingState === 'retained',
    profileClaims: session.profileClaims,
    proxyExitUnknown: session.proxies.some((proxy) => proxy.exit?.exitConfirmed !== true),
    hostedMachineDisposalVerified: false,
    networkSandboxValidated: false,
    owners: session.owners.map(
      ({
        kind,
        phase,
        exited,
        environmentVerified,
        childExitConfirmed,
        childExitError,
        launchConfirmed,
      }) => ({
        kind,
        phase,
        exitConfirmed: exited === true,
        environmentVerified: environmentVerified === true,
        ...(kind === 'probe'
          ? { childExitConfirmed: childExitConfirmed === true, childExitError }
          : { launchConfirmed: launchConfirmed === true }),
      }),
    ),
    proxies: session.proxies.map((proxy) => ({
      phase: proxy.phase,
      ...proxy.exit,
      startupError: proxy.startupError,
      shutdownError: proxy.shutdownError,
      evidenceError: proxy.evidenceError,
    })),
    cleanupFailed: session.cleanupFailed,
  };
}
export async function runTelemetryPhases(context, session) {
  const { source, owned, output, home, checked, helper, driver, automationEnvironment, budget } =
    context;
  session.probe = await prepareProbe(context);
  let previousDatabase;
  for (const phase of phases) {
    budget.remaining(1);
    const proxy = await startTelemetryProxy(join(owned, `proxy-${phase}`), budget, {
      register: (proxy) => {
        proxy.phase = phase;
        session.proxies.push(proxy);
      },
    });
    const environmentOff = phase === 'environment-off';
    const env = telemetryEnvironment(owned, home, proxy.port, environmentOff);
    const environmentFile = join(owned, `environment-${phase}.json`);
    await writeFile(environmentFile, JSON.stringify(env), { flag: 'wx', mode: 0o600 });
    const phaseReport = { phase, proxyPid: proxy.pid, proxyPort: proxy.port };
    session.phases.push(phaseReport);
    budget.remaining(40_000);
    phaseReport.systemCurl = await qualifyTelemetryProxy(
      proxy,
      env,
      owned,
      home,
      environmentOff,
      budget,
    );
    await sameProbe(session.probe);
    const probeBefore = await telemetryProxyState(proxy);
    const probeOwner = {
      kind: 'probe',
      phase,
      receipt: join(owned, `probe-receipt-${phase}.json`),
      environmentFile,
      exited: false,
    };
    await claimTelemetryProfiles(session, 'probe', home);
    session.probeRegistered = true;
    session.owners.push(probeOwner); // Record attempted ownership before LaunchServices can start anything.
    await checked(
      `${phase}-probe-launch`,
      helper,
      ['probe-launch', probeOwner.receipt, environmentFile],
      55_000,
      automationEnvironment,
    );
    const probeReceipt = await readJSON(probeOwner.receipt);
    await writeFile(
      join(output, `probe-receipt-${phase}.json`),
      JSON.stringify(probeReceipt, null, 2),
      { flag: 'wx' },
    );
    await checked(
      `${phase}-probe-environment`,
      helper,
      ['probe-inspect', probeOwner.receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    probeOwner.environmentVerified = true;
    const result = await waitFor(
      async () => {
        try {
          return await readJSON(`${probeOwner.receipt}.result.json`);
        } catch (error) {
          if (error.code === 'ENOENT') return null;
          throw error;
        }
      },
      budget,
      25_000,
      'LaunchServices probe result',
    );
    phaseReport.launchServicesProbe = result;
    await confirmProbeChildExit(probeOwner);
    if (
      result.passed !== true ||
      result.pid !== probeReceipt.pid ||
      result.environmentMatches !== true ||
      result.genuineHome !== true ||
      result.curlExitConfirmed !== true ||
      result.curlStatus !== 56 ||
      result.httpCode !== '000'
    )
      throw new Error('LaunchServices did not qualify the actual inherited proxy environment');
    await waitFor(
      async () => (await telemetryProxyState(proxy)).probe > probeBefore.probe,
      budget,
      3000,
      'LaunchServices CONNECT rejection',
    );
    await stopOwner(probeOwner, checked, helper, automationEnvironment, `${phase}-probe`);
    await sameProbe(session.probe);
    await checked(`${phase}-probe-unregister`, registry, ['-u', session.probe.bundle]);
    session.probeRegistered = false;
    validateTelemetryEnvironment(
      await readJSON(environmentFile),
      owned,
      home,
      proxy.port,
      environmentOff,
    );
    await telemetryProxyState(proxy);
    await verifyAppInventory(release.bundle);
    const owner = {
      kind: 'telemetry',
      phase,
      receipt: join(output, `launch-receipt-${phase}.json`),
      environmentFile,
      exited: false,
    };
    await claimTelemetryProfiles(session, 'telemetry', home);
    session.owners.push(owner);
    await checked(
      `${phase}-launch`,
      helper,
      ['telemetry-launch', owner.receipt, environmentFile],
      55_000,
      automationEnvironment,
    );
    const receipt = await confirmTelemetryLaunch(owner);
    phaseReport.pid = receipt.pid;
    await checked(
      `${phase}-environment`,
      helper,
      ['telemetry-inspect', owner.receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    owner.environmentVerified = true;
    phaseReport.environmentVerified = true;
    const ui = async (mode) => {
      const text = await checked(
        `${phase}-${mode}`,
        driver,
        [String(receipt.pid), release.bundle, release.identifier, mode],
        100_000,
        automationEnvironment,
      );
      const evidence = JSON.parse(text);
      await writeFile(
        join(output, `${phase}-${mode}.json`),
        `${JSON.stringify(evidence, null, 2)}\n`,
        { flag: 'wx' },
      );
      const state = await waitFor(
        async () => {
          const current = await telemetryProxyState(proxy);
          return !evidence.telemetry?.expectedUpdaterWarning || current.github > 0 ? current : null;
        },
        budget,
        3000,
        'Same-phase blocked updater request',
      );
      (phaseReport.ui ||= []).push(acceptTelemetryUI(evidence, state, mode));
    };
    const attempt = () =>
      waitFor(
        async () => (await telemetryProxyState(proxy)).telemetry > 0,
        budget,
        30_000,
        'Blocked default telemetry attempt',
      );
    if (phase === 'notice') {
      await ui('telemetry-notice');
      await attempt();
      await ui('telemetry-turn-off');
      await ui('telemetry-dashboard');
    } else {
      if (phase === 'saved-on') await attempt();
      await ui(
        phase === 'environment-off'
          ? 'telemetry-environment-off'
          : phase === 'saved-on'
            ? 'telemetry-settings-on'
            : 'telemetry-settings-off',
      );
      if (phase === 'saved-off') await ui('telemetry-settings-enable');
      if (phase === 'saved-on') await ui('telemetry-settings-disable');
    }
    await stopOwner(owner, checked, helper, automationEnvironment, phase);
    phaseReport.exitConfirmed = true;
    phaseReport.afterExitInventory = await verifyAppInventory(release.bundle);
    phaseReport.proxy = await telemetryProxyState(proxy);
    phaseReport.database = await verifyTelemetryDatabase(
      source,
      telemetryDataDirectory(home),
      checked,
      phase,
      phase === 'saved-off' || environmentOff,
    );
    if (
      environmentOff &&
      (phaseReport.proxy.telemetry !== 0 ||
        phaseReport.database.telemetry.ledger.lastId !== previousDatabase.telemetry.ledger.lastId ||
        phaseReport.database.telemetry.ledger.rows !== previousDatabase.telemetry.ledger.rows)
    )
      throw new Error('Environment-off restart changed telemetry ledger or attempted delivery');
    previousDatabase = phaseReport.database;
  }
  return session.phases;
}

// The proxy remains alive through owned app/profile cleanup. If app exit cannot
// be proved, retain all files, mark failure first, then stop the owned proxy.
export async function cleanupTelemetry(session, { stopOwners, removeOwned, stopProxies }) {
  let primary;
  try {
    await stopOwners();
    await removeOwned();
  } catch (error) {
    session.cleanupFailed = true;
    primary = error;
  }
  try {
    await stopProxies();
  } catch (error) {
    session.cleanupFailed = true;
    if (primary) {
      primary.proxyCleanupError = error.message;
    } else primary = error;
  }
  if (primary) throw primary;
}
export async function unregisterTelemetryProbe(session, checked) {
  if (session.probeRegistered) {
    await checked('cleanup-probe-unregister', registry, ['-u', session.probe.bundle]);
    session.probeRegistered = false;
  }
}
export async function telemetryProxiesHealthy(session) {
  for (const proxy of session.proxies)
    if (proxy.exit?.exitConfirmed !== true) await telemetryProxyState(proxy);
}
