import { basename } from 'node:path';
import { release, sha256, appInventory } from './hosted-release.mjs';
import { requireProbeHost } from './hosted-validation.mjs';

// The published 0.1.7 that users run today, installed as "XTrace Desktop.app".
// Its public archive (11,064,049 bytes) and the 79 paths/bytes/modes that
// /usr/bin/tar extracts from it were checked independently; the same 79-entry
// inventory is the one the 0.1.6 to 0.1.7 hosted update installed.
export const previousRelease = Object.freeze({
  identifier: release.identifier,
  bundle: '/Applications/XTrace Desktop.app',
  team: release.team,
  inventoryEntries: 79,
  version: '0.1.7',
  tag: 'v0.1.7',
  source: 'e743e900fb40d32bdbe90709bd54b7d9bde32540',
  archive: 'XTrace-Desktop-0.1.7-macos-arm64.app.tar.gz',
  bytes: 11064049,
  sha256: 'cef0e375a22290b6064c5871920ab8710107cc9d802e4ff3227c9b1dea7a151f',
  inventorySha256: '250d82fc255fd691aac182207dde84f39abeeef1d49ca603dd1965f54fd891bd',
});
export const updateBudget = Object.freeze({ workMs: 14 * 60_000, cleanupMs: 2 * 60_000 });
export const productionFeed =
  'https://github.com/XTraceAI/xtrace-desktop/releases/latest/download/latest.json';
export const sentinel = `Owned hosted update check: retain this synthetic file across ${previousRelease.version} to ${release.version}.\n`;

export function requireUpdateHost(input) {
  if (input.mode !== 'update' || input.allowReleaseIdentity !== 'true')
    throw new Error('Update requires an explicit hosted real-identity exception');
  requireProbeHost({ ...input, mode: 'probe' });
}

export function verifyPreviousDownload(bytes) {
  if (bytes.length !== previousRelease.bytes || sha256(bytes) !== previousRelease.sha256)
    throw new Error(
      `Old archive does not match the pinned public ${previousRelease.version} release`,
    );
}

export async function verifyPreviousInventory(bundle) {
  const inventory = await appInventory(bundle, basename(previousRelease.bundle));
  if (
    inventory.entries !== previousRelease.inventoryEntries ||
    inventory.sha256 !== previousRelease.inventorySha256
  )
    throw new Error(
      `Old app paths, bytes or modes differ from the signed ${previousRelease.version} release`,
    );
  return inventory;
}

// Exact candidate feed bytes; the current production endpoint must match
// before any app is extracted or launched. This check never promotes the feed.
export function verifyProductionFeed(bytes, pins = release) {
  if (bytes.length !== pins.feedBytes || sha256(bytes) !== pins.feedSha256)
    throw new Error(
      `Production feed prerequisite unmet: canonical ${pins.version} is not available`,
    );
  // Defense in depth: the pinned bytes must also name this version and only
  // the public archive of its own tag.
  const feed = JSON.parse(bytes);
  if (
    feed.version !== pins.version ||
    JSON.stringify(Object.keys(feed.platforms || {})) !== '["darwin-aarch64"]' ||
    feed.platforms['darwin-aarch64'].url !==
      `https://github.com/XTraceAI/xtrace-desktop/releases/download/${pins.tag}/${pins.archive}`
  )
    throw new Error('Pinned feed names a different version, platform or archive');
  return { bytes: bytes.length, sha256: sha256(bytes), version: feed.version };
}

// The XTrace bundle folders in /Applications, plus any updater copy that was
// left there. Baseline guarantees there were none before this run.
export function xtraceEntries(names) {
  return names.filter((name) => /xtrace|^tauri_/i.test(name)).sort();
}

// The one rule for which folder holds the app after each step of the update:
// after Restart to update the updater has written the candidate into the
// earlier folder and nothing is renamed; after a normal quit only the
// candidate's folder name is left.
export const bundleSteps = Object.freeze({
  installed: [basename(previousRelease.bundle)],
  restarted: [basename(previousRelease.bundle)],
  quit: [basename(release.bundle)],
  reopened: [basename(release.bundle)],
});
export function requireBundleStep(step, names) {
  const found = xtraceEntries(names);
  const expected = bundleSteps[step];
  if (!expected || JSON.stringify(found) !== JSON.stringify(expected))
    throw new Error(
      `${step}: /Applications holds ${found.join(', ') || 'no XTrace app'}; expected only ${expected?.join(', ')}`,
    );
  return found;
}

// The updater moves the current app aside into, and extracts the new one
// from, temporary folders with these names. Both must be gone afterwards.
export function updaterLeftovers(names) {
  return names.filter((name) => /^tauri_(current|updated)_app/.test(name)).sort();
}

export function requireAppIdentity(plist, version) {
  if (
    plist.CFBundleIdentifier !== release.identifier ||
    plist.CFBundleShortVersionString !== version ||
    plist.CFBundleVersion !== version ||
    plist.CFBundleExecutable !== 'xtrace-desktop'
  )
    throw new Error(`App identity/version is not ${release.identifier} ${version}`);
}

// The data folder's database must be the one file the earlier version made,
// never a replacement: same inode and creation time.
export function requireSameDatabase(before, after, step) {
  if (
    !before ||
    !after ||
    before.ino !== after.ino ||
    before.dev !== after.dev ||
    before.birthtimeMs !== after.birthtimeMs
  )
    throw new Error(`${step}: the synthetic database was replaced or is missing`);
}

// A failed/timed-out AX call may still have started installation. Always stop
// the known original if successor adoption fails, but keep the uncertainty:
// throwing prevents removal of a bundle that a late successor could still use.
export async function stopUpdateProcesses({ pressAttempted, successorExists }, actions) {
  if (pressAttempted && !successorExists) {
    try {
      await actions.adopt();
    } catch (adoptionError) {
      try {
        await actions.stopOriginal();
      } catch (stopError) {
        throw new Error(
          `${adoptionError.message}; original stop also failed: ${stopError.message}`,
          { cause: stopError },
        );
      }
      throw new Error(
        `${adoptionError.message}; original stopped, transition remains uncertain; app retained`,
        { cause: adoptionError },
      );
    }
  }
  if (pressAttempted) await actions.stopSuccessor();
  else await actions.stopOriginal();
  await actions.confirmAbsent();
}
