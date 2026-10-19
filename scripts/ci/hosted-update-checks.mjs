import { release, sha256, appInventory } from './hosted-release.mjs';
import { requireProbeHost } from './hosted-validation.mjs';

// The public 0.1.6 archive and its own 79 paths/bytes/modes were checked
// independently. Keep its inventory count separate from the candidate target.
export const previousRelease = Object.freeze({
  identifier: release.identifier,
  bundle: release.bundle,
  team: release.team,
  inventoryEntries: 79,
  version: '0.1.6',
  tag: 'v0.1.6',
  source: 'bab5673759993dc8970a846a8e37625def989d2b',
  archive: 'XTrace-Desktop-0.1.6-macos-arm64.app.tar.gz',
  bytes: 10997217,
  sha256: '8a3b9708bb1342ae73fdd646672597e795ca66b87706669bb72bcbe2e330bc8c',
  inventorySha256: '70a1cbcff0208976fca09e8557e08d93bb75b19dcc0262666cf555e85d59cdaa',
});
export const updateBudget = Object.freeze({ workMs: 12 * 60_000, cleanupMs: 2 * 60_000 });
export const productionFeed =
  'https://github.com/XTraceAI/xtrace-desktop/releases/latest/download/latest.json';
export const sentinel =
  'Owned hosted update check: retain this synthetic file across 0.1.6 to 0.1.7.\n';

export function requireUpdateHost(input) {
  if (input.mode !== 'update' || input.allowReleaseIdentity !== 'true')
    throw new Error('Update requires an explicit hosted real-identity exception');
  requireProbeHost({ ...input, mode: 'probe' });
}

export function verifyPreviousDownload(bytes) {
  if (bytes.length !== previousRelease.bytes || sha256(bytes) !== previousRelease.sha256)
    throw new Error('Old archive does not match the pinned public 0.1.6 release');
}

export async function verifyPreviousInventory(bundle) {
  const inventory = await appInventory(bundle);
  if (
    inventory.entries !== previousRelease.inventoryEntries ||
    inventory.sha256 !== previousRelease.inventorySha256
  )
    throw new Error('Old app paths, bytes or modes differ from the signed 0.1.6 release');
  return inventory;
}

// Exact prospective feed bytes; the current production endpoint must match
// before any app is extracted or launched. This check never promotes the feed.
export function verifyProductionFeed(bytes) {
  if (
    bytes.length !== 1751 ||
    sha256(bytes) !== 'c2bf25a08febff2ddeda03cfaceb04d75d553d6c15ecb9f0aaa85ac07180c02d'
  )
    throw new Error('Production feed prerequisite unmet: canonical 0.1.7 is not available');
  return { bytes: bytes.length, sha256: sha256(bytes), version: JSON.parse(bytes).version };
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
