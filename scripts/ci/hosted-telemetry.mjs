import { isAbsolute, join } from 'node:path';
import {
  firstLaunchEnvironment,
  requireFirstLaunchHost,
  verifyHostedDatabase,
} from './hosted-release.mjs';

export const telemetryBudget = Object.freeze({ workMs: 18 * 60_000, cleanupMs: 3 * 60_000 });
export const telemetryLimits = Object.freeze({
  launchConfiguration: 'fresh default app data with a non-forwarding rejecting loopback proxy',
  successfulSignedHTTPPostValidated: false,
  realPostHogDeliveryValidated: false,
  beforeFirstPixelOrderingValidated: false,
  payloadEvidence: 'frozen-source tests; CONNECT rejection exposes no event body',
});
export function requireTelemetryHost(input) {
  if (input.mode !== 'telemetry') throw new Error('Expected explicit telemetry mode');
  requireFirstLaunchHost({ ...input, mode: 'first-launch' });
}
export function telemetryDataDirectory(home) {
  if (!isAbsolute(home) || home === '/') throw new Error('Expected the genuine hosted user home');
  return join(home, 'Library/Application Support/ai.xtrace.app');
}
export function telemetryOwnedPaths(home) {
  return ['ai.xtrace.app', 'ai.xtrace.app.test.telemetry-proxy'].flatMap((id) => [
    join(home, 'Library/Application Support', id),
    ...[
      `WebKit/${id}`,
      `Preferences/${id}.plist`,
      `Caches/${id}`,
      `Saved Application State/${id}.savedState`,
      `HTTPStorages/${id}`,
    ].map((path) => join(home, 'Library', path)),
  ]);
}
export function telemetryEnvironment(owned, home, port, environmentOff = false) {
  if (!Number.isInteger(port) || port < 1 || port > 65535)
    throw new Error('Invalid owned proxy port');
  telemetryDataDirectory(home);
  const env = firstLaunchEnvironment(owned);
  delete env.XTRACE_DATA_DIR;
  delete env.XTRACE_TELEMETRY;
  return {
    ...env,
    HOME: home,
    HTTPS_PROXY: `http://127.0.0.1:${port}`,
    https_proxy: `http://127.0.0.1:${port}`,
    NO_PROXY: '',
    no_proxy: '',
    ...(environmentOff ? { XTRACE_TELEMETRY: '0' } : {}),
  };
}
export function validateTelemetryEnvironment(actual, owned, home, port, environmentOff = false) {
  const expected = telemetryEnvironment(owned, home, port, environmentOff);
  if (
    Object.keys(actual).length !== Object.keys(expected).length ||
    Object.entries(actual).some(
      ([key, value]) => !Object.hasOwn(expected, key) || expected[key] !== value,
    )
  )
    throw new Error('Telemetry launch environment differs from its exact contained configuration');
  return expected;
}
export async function verifyTelemetryDatabase(source, directory, checked, phase, enabled) {
  const result = await verifyHostedDatabase(source, directory, (label, program, args) =>
    checked(`${phase}-${label}`, program, [
      ...args,
      enabled ? '--telemetry=on' : '--telemetry=off',
    ]),
  );
  if (
    result.telemetry?.preferenceEnabled !== enabled ||
    result.telemetry?.disclosureShown !== true ||
    result.telemetry?.ledger?.sent !== 0
  )
    throw new Error('Database did not confirm the required saved telemetry state');
  return result;
}

// What each driver phase establishes; the Swift driver reports these words
// (telemetryReason in hosted-ax.swift), and the tests compare the two.
export const telemetryUIReasons = Object.freeze({
  'telemetry-notice':
    'Welcome notice text and one enabled Turn off are in the accessibility tree; visibility was not checked',
  'telemetry-dashboard-notice':
    'Pressed Open Dashboard once; the Dashboard and the corner-card notice with one enabled Turn off are in the accessibility tree',
  'telemetry-turn-off':
    'Pressed Turn off once in the corner card; the card then said telemetry is off and no Turn off remained',
  'telemetry-settings-off': 'Opened Settings; the telemetry switch is off and enabled',
  'telemetry-settings-on': 'Opened Settings; the telemetry switch is on and enabled',
  'telemetry-settings-enable': 'Pressed the enabled telemetry switch once; it is now on',
  'telemetry-settings-disable': 'Pressed the enabled telemetry switch once; it is now off',
  'telemetry-environment-off':
    'Opened Settings; the telemetry switch is off and disabled, with the environment-off explanation',
});

export function acceptTelemetryUI(evidence, state, mode) {
  if (
    !Object.hasOwn(telemetryUIReasons, mode) ||
    evidence.code !== 0 ||
    evidence.reason !== telemetryUIReasons[mode] ||
    !Array.isArray(evidence.nodes) ||
    typeof evidence.telemetry?.expectedUpdaterWarning !== 'boolean' ||
    (evidence.telemetry.expectedUpdaterWarning && !(state.github > 0))
  )
    throw new Error('Telemetry UI evidence lacks required same-phase proxy confirmation');
  return {
    mode,
    established: telemetryUIReasons[mode],
    expectedUpdaterWarning: evidence.telemetry.expectedUpdaterWarning,
    blockedUpdaterAuthority: evidence.telemetry.expectedUpdaterWarning ? 'github.com:443' : null,
  };
}
