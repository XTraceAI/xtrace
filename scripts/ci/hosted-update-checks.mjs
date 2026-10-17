import { release, sha256, appInventory } from './hosted-release.mjs';
import { requireProbeHost } from './hosted-validation.mjs';

// Retained canonical public archives and all 79 paths/bytes/modes were checked
// independently. Runtime needs only these fixed digests, not a copy of the maps.
export const previousRelease = Object.freeze({
  identifier: release.identifier,
  bundle: release.bundle,
  team: release.team,
  inventoryEntries: release.inventoryEntries,
  version: '0.1.5',
  tag: 'v0.1.5',
  source: 'b9e82894ea6fa34c183f82b8b400693c200b9683',
  archive: 'XTrace-Desktop-0.1.5-macos-arm64.app.tar.gz',
  bytes: 11255927,
  sha256: '45f1762cf36081e7554ef087ecea5cff089869a71257aff2a2ec083f47a87f5d',
  inventorySha256: 'a76724e86f6b2f9a1e9a85d148310f4871844e4fc2f7df880e4ed33c3e82e203',
});
export const updateBudget = Object.freeze({ workMs: 12 * 60_000, cleanupMs: 2 * 60_000 });
export const productionFeed =
  'https://github.com/XTraceAI/xtrace-desktop/releases/latest/download/latest.json';
export const sentinel =
  'Owned hosted update check: retain this synthetic file across 0.1.5 to 0.1.6.\n';

export function requireUpdateHost(input) {
  if (input.mode !== 'update' || input.allowReleaseIdentity !== 'true')
    throw new Error('Update requires an explicit hosted real-identity exception');
  requireProbeHost({ ...input, mode: 'probe' });
}

export function verifyPreviousDownload(bytes) {
  if (bytes.length !== previousRelease.bytes || sha256(bytes) !== previousRelease.sha256)
    throw new Error('Old archive does not match the pinned public 0.1.5 release');
}

export async function verifyPreviousInventory(bundle) {
  const inventory = await appInventory(bundle);
  if (
    inventory.entries !== previousRelease.inventoryEntries ||
    inventory.sha256 !== previousRelease.inventorySha256
  )
    throw new Error('Old app paths, bytes or modes differ from the signed 0.1.5 release');
  return inventory;
}

export function verifyProductionFeed(bytes) {
  if (
    bytes.length !== 985 ||
    sha256(bytes) !== 'b8dcc5a2b8349c1af5e85b520648182c46972cf59f08999d6eb043e11b5753b6'
  )
    throw new Error('Production feed prerequisite unmet: canonical 0.1.6 is not available');
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
