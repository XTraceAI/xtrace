import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { baselinePaths, probeEnvironment, requireProbeHost } from './hosted-validation.mjs';

// The 0.1.9 candidate. Runtime still requires the matching public tag and the
// exact public DMG, archive, signature and feed bytes. The harness commit is
// separate from the released product source commit.
//
// The ten build values below were computed from the signed v0.1.9 files
// (docs/UPDATES.md says how). requireReleaseValues() refuses before any
// download, install or launch if any is missing or malformed. This object is
// the only place these build values are written.
export const release = Object.freeze({
  version: '0.1.9',
  tag: 'v0.1.9',
  source: '8bbc6c96d56780337cdc1644a0deebfde56514f1',
  identifier: 'ai.xtrace.app',
  bundle: '/Applications/XTrace.app',
  team: '3C8BCX5J3H',
  dmg: 'XTrace-0.1.9-macos-arm64.dmg',
  archive: 'XTrace-0.1.9-macos-arm64.app.tar.gz',
  // BEGIN SIGNED BUILD VALUES
  bytes: 13279128, // DMG size in bytes
  sha256: '1e85356568f08e6007f2b87ba93d46afca9a645cced527a879f5a641f9cdc8bd', // DMG SHA-256
  inventoryEntries: 102, // appInventory() of the DMG's XTrace.app
  inventorySha256: '7e4a76b78bd3d83a7c9474b26a7e1e2b0ba1e2d1dcc62e8f521698456c52420f',
  archiveBytes: 12597709, // updater archive size in bytes
  archiveSha256: '05f73f2f7af4b5b0b698b4603ad3b97cb1487a1b6fa3b28104bd9d43f6b1238e',
  signatureBytes: 448, // the archive's .sig file
  signatureSha256: '3064e4b99913b16a17118a7e7c124719b213fdabf2ce782c0fcb2a0bffdd4512',
  feedBytes: 958, // latest.json, byte-identical to fixtures/hosted-update-latest.json
  feedSha256: 'aa8515579c8b6c899edbd742df750d6a71028b41261a50dc4dfcf7e858f61825',
  // END SIGNED BUILD VALUES
});

const sizeNames = ['bytes', 'inventoryEntries', 'archiveBytes', 'signatureBytes', 'feedBytes'];
const digestNames = ['sha256', 'inventorySha256', 'archiveSha256', 'signatureSha256', 'feedSha256'];
export const releaseValueNames = Object.freeze([...sizeNames, ...digestNames]);

// Names of build values that are still unfilled or not a size/SHA-256.
export function missingReleaseValues(values = release) {
  return releaseValueNames.filter((name) =>
    sizeNames.includes(name)
      ? !Number.isSafeInteger(values[name]) || values[name] <= 0
      : typeof values[name] !== 'string' || !/^[0-9a-f]{64}$/.test(values[name]),
  );
}

export function requireReleaseValues(values = release) {
  const missing = missingReleaseValues(values);
  if (missing.length)
    throw new Error(
      `Signed ${values.version} build values are not filled in scripts/ci/hosted-release.mjs: ${missing.join(', ')}`,
    );
}

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

export function verifyDownload(bytes, pins = release) {
  if (bytes.length !== pins.bytes || sha256(bytes) !== pins.sha256)
    throw new Error('Downloaded DMG does not match the pinned public release');
}

// Inventory includes every literal byte, path, size and permission mode. Extended
// attributes (including quarantine) are reported separately; they are not code bytes.
// Paths start at `root`, the bundle folder's own name unless a caller names it:
// the same app contents give the same inventory under XTrace Desktop.app or XTrace.app.
export async function appInventory(bundle, root = basename(bundle)) {
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
  await visit(bundle, root);
  rows.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return { entries: rows.length, sha256: sha256(JSON.stringify(rows)) };
}

// The candidate's contents, wherever its folder is and whatever it is named.
export async function verifyAppInventory(bundle, pins = release) {
  const inventory = await appInventory(bundle, basename(pins.bundle));
  if (inventory.entries !== pins.inventoryEntries || inventory.sha256 !== pins.inventorySha256)
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
