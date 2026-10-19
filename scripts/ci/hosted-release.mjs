import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { baselinePaths, probeEnvironment, requireProbeHost } from './hosted-validation.mjs';

// Immutable 0.1.7 candidate inputs, independently checked before publication.
// Runtime still requires the matching public tag and exact public DMG bytes.
// The harness commit is separate from the released product source commit.
export const release = Object.freeze({
  version: '0.1.7',
  tag: 'v0.1.7',
  source: 'e743e900fb40d32bdbe90709bd54b7d9bde32540',
  identifier: 'ai.xtrace.app',
  bundle: '/Applications/XTrace Desktop.app',
  team: '3C8BCX5J3H',
  dmg: 'XTrace-Desktop-0.1.7-macos-arm64.dmg',
  bytes: 11748749,
  sha256: '2ec76b267a79ac20fd216306bd33c3b04715199f1c4ba086bc2a12664d0c1fe9',
  inventoryEntries: 79,
  inventorySha256: '250d82fc255fd691aac182207dde84f39abeeef1d49ca603dd1965f54fd891bd',
});

export function requireFirstLaunchHost(input) {
  if (input.mode !== 'first-launch' || input.allowReleaseIdentity !== 'true')
    throw new Error('First launch requires an explicit hosted real-identity exception');
  requireProbeHost({ ...input, mode: 'probe' });
}

export const firstLaunchBudget = Object.freeze({ workMs: 8 * 60_000, cleanupMs: 2 * 60_000 });
// The isolated environment suppresses both telemetry and its default notice.
// Keep these limits in every report, including successful launch/update reports.
export const signedLaunchLimits = Object.freeze({
  launchConfiguration: 'synthetic data with telemetry disabled',
  defaultTelemetryNoticeValidated: false,
  defaultTelemetryBehaviorValidated: false,
});
export function executionBudget(now = () => performance.now(), limits = firstLaunchBudget) {
  let deadline = now() + limits.workMs;
  let cleaning = false;
  return {
    remaining(requestedMs) {
      const remaining = Math.floor(deadline - now());
      if (remaining <= 0)
        throw new Error(`${cleaning ? 'Cleanup' : 'First-launch work'} time budget exhausted`);
      return Math.min(requestedMs, remaining);
    },
    beginCleanup() {
      if (!cleaning) {
        cleaning = true;
        deadline = now() + limits.cleanupMs;
      }
    },
  };
}

export function firstLaunchEnvironment(directory) {
  // LaunchServices may add this key; the process inspector requires it to stay empty.
  return {
    ...probeEnvironment(directory),
    GH_CONFIG_DIR: join(directory, 'gh'),
    SSH_AUTH_SOCK: '',
  };
}

export function firstLaunchBaselinePaths(home) {
  return [
    ...baselinePaths(home),
    ...['.codex', '.claude', '.claude.json', '.cursor', '.config/gh'].map((p) => join(home, p)),
    ...['ai.xtrace.app', 'ai.xtrace.desktop'].flatMap((id) => [
      join(home, 'Library/WebKit', id),
      join(home, 'Library/Preferences', `${id}.plist`),
      join(home, 'Library/Caches', id),
      join(home, 'Library/Saved Application State', `${id}.savedState`),
      join(home, 'Library/HTTPStorages', id),
    ]),
    join(home, 'Library/Application Support/Cursor'),
    join(home, 'Library/Application Support/Code/User/globalStorage'),
  ];
}

export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

export function verifyDownload(bytes) {
  if (bytes.length !== release.bytes || sha256(bytes) !== release.sha256)
    throw new Error('Downloaded DMG does not match the pinned public release');
}

// Inventory includes every literal byte, path, size and permission mode. Extended
// attributes (including quarantine) are reported separately; they are not code bytes.
export async function appInventory(bundle) {
  const rows = [];
  const visit = async (path, relative) => {
    const info = await lstat(path);
    if (info.isSymbolicLink() || (!info.isDirectory() && !info.isFile()))
      throw new Error('App inventory contains a link or special file');
    rows.push([
      relative,
      info.isDirectory(),
      info.isDirectory() ? 0 : info.size,
      info.mode & 0o7777,
      info.isDirectory() ? null : sha256(await readFile(path)),
    ]);
    if (info.isDirectory())
      for (const name of await readdir(path)) await visit(join(path, name), `${relative}/${name}`);
  };
  await visit(bundle, basename(bundle));
  rows.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return { entries: rows.length, sha256: sha256(JSON.stringify(rows)) };
}

export async function verifyAppInventory(bundle) {
  const inventory = await appInventory(bundle);
  if (
    inventory.entries !== release.inventoryEntries ||
    inventory.sha256 !== release.inventorySha256
  )
    throw new Error('App file bytes, paths or modes differ from the pinned signed release');
  return inventory;
}

export async function verifyTag(tag, readAnnotation) {
  if (tag?.ref !== `refs/tags/${release.tag}` || !/^[0-9a-f]{40}$/.test(tag.object?.sha))
    throw new Error('Public release tag has an unexpected ref or object identity');
  let target = tag.object;
  if (target.type === 'tag') {
    // The published tag has one annotation layer. Never follow an API-supplied
    // URL or recursively peel an unexpected annotation chain.
    if (typeof readAnnotation !== 'function') throw new Error('Tag annotation lookup is required');
    const annotation = await readAnnotation(
      `https://api.github.com/repos/XTraceAI/xtrace-desktop/git/tags/${target.sha}`,
    );
    if (annotation?.sha !== target.sha || annotation.tag !== release.tag)
      throw new Error('Tag annotation identity or name does not match its ref');
    target = annotation.object;
  }
  if (target?.type !== 'commit' || target.sha !== release.source)
    throw new Error('Public release tag no longer identifies the pinned source commit');
  return release.source;
}

// One strict verifier for both released-app modes, always after confirmed exit.
export async function verifyHostedDatabase(source, directory, checked) {
  const result = JSON.parse(
    await checked('database', '/opt/homebrew/bin/python3', [
      '-I',
      join(source, 'hosted-database.py'),
      join(directory, 'xtrace.db'),
    ]),
  );
  if (
    result.status !== 'passed' ||
    result.mode !== 'ro' ||
    result.mainDatabaseUnchanged !== true ||
    JSON.stringify(result.integrity) !== '["ok"]' ||
    result.sessions !== 0 ||
    result.records !== 0
  )
    throw new Error('Database checker did not confirm intact unchanged empty data');
  return result;
}
