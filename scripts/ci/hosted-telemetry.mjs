import { isAbsolute, join } from 'node:path';
import { firstLaunchEnvironment, verifyHostedDatabase } from './hosted-release.mjs';

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
