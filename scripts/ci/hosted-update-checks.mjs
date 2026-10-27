import { basename } from 'node:path';
import { release, sha256, appInventory } from './hosted-release.mjs';
import { requireProbeHost } from './hosted-validation.mjs';

// The published 0.1.8 baseline, installed as "XTrace.app". Its public archive
// and all 101 paths/bytes/modes were checked independently against its DMG.
export const previousRelease = Object.freeze({
  identifier: release.identifier,
  bundle: '/Applications/XTrace.app',
  team: release.team,
  inventoryEntries: 101,
  version: '0.1.8',
  tag: 'v0.1.8',
  source: '75d49d54f6717d32d72eeb7a933eebb13f5be148',
  archive: 'XTrace-0.1.8-macos-arm64.app.tar.gz',
  bytes: 11843476,
  sha256: 'ba52098989d15ab6aab7600609b030460c2840d0757dff06e88beab5f5412c2e',
  inventorySha256: '68c08e83836c518fcc27f1571bb28114b3ca3435c01a2a9b53cd0312bf4df524',
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
// both releases use XTrace.app. Each phase independently requires that one
// folder, with no backup or extracted updater copies beside it.
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
