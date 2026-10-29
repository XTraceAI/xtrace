import { readFile, readdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { release, verifyAppInventory, verifyHostedDatabase } from './hosted-release.mjs';
import {
  previousRelease,
  productionFeed,
  verifyProductionFeed,
  verifyPreviousDownload,
  stopUpdateProcesses,
  requireBundleStep,
  updaterLeftovers,
  verifyPreviousInventory,
} from './hosted-update-checks.mjs';
const earlierBundle = previousRelease.bundle,
  candidateBundle = release.bundle;
export async function runUpdateLifecycle(
  {
    report,
    checked,
    helper,
    receipt,
    environmentFile,
    automationEnvironment,
    successorReceipt,
    reopenReceipt,
    env,
    databaseIdentity,
    dataKept,
    source,
    observe,
    markAttempt,
    prepare,
    appIdentity,
    validateSignature,
    quarantineAt,
  },
  effects = {},
) {
  const {
    readFile: read = readFile,
    readdir: entries = readdir,
    verifyPreviousInventory: previousInventory = verifyPreviousInventory,
    verifyAppInventory: inventory = verifyAppInventory,
    verifyHostedDatabase: database = verifyHostedDatabase,
  } = effects;
  await prepare();
  markAttempt('launch');
  await checked(
    'launch-services',
    helper,
    ['earlier-launch', receipt, environmentFile],
    55_000,
    automationEnvironment,
  );
  const ownership = JSON.parse(await read(receipt, 'utf8'));
  report.releasedAppLaunched = true;
  report.pid = ownership.pid;
  await checked(
    'process-environment',
    helper,
    ['earlier-inspect', receipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  await observe(
    'update-ready',
    ownership.pid,
    earlierBundle,
    185_000,
    `Observed usable ${previousRelease.version} Dashboard and target ${release.version} ready`,
  );
  // The earlier version made the database while showing Welcome. Every later
  // step must find this same file.
  report.databaseIdentity = await databaseIdentity();
  // Ready means verified download, not installation. Prove old bytes remain
  // immediately before asking the shipped UI to install and restart itself.
  report.beforePressInventory = await previousInventory(earlierBundle);
  await checked(
    'arm-update',
    helper,
    ['earlier-arm-update', receipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  markAttempt('press'); // AX success/timeout can race process exit; cleanup must resolve both.
  await observe(
    'update-press',
    ownership.pid,
    earlierBundle,
    15_000,
    `Pressed real Restart to update once for target ${release.version}`,
  );
  report.restartPressed = true;
  await checked(
    'adopt-update',
    helper,
    ['earlier-adopt-update', receipt, environmentFile],
    60_000,
    automationEnvironment,
  );
  const successor = JSON.parse(await read(successorReceipt, 'utf8'));
  report.originalExitConfirmed = true;
  report.successorPid = successor.pid;
  await checked(
    'successor-environment',
    helper,
    ['earlier-inspect', successorReceipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  await observe(
    'updated-dashboard',
    successor.pid,
    earlierBundle,
    95_000,
    `Observed usable native ${release.version} Dashboard after restart and retained completed Welcome`,
  );
  // (a) After the restart: the same fixed folder, now holding the
  // candidate's exact files, relaunched as a distinct process, data kept.
  report.restarted = {
    applications: requireBundleStep('restarted', await entries('/Applications')),
    inventory: await inventory(earlierBundle),
  };
  await appIdentity(earlierBundle, release.version, 'updated-plist');
  await validateSignature(earlierBundle, 'updated-app');
  await dataKept('After restart');
  report.restarted.dataKept = true;
  // A normal quit (the Quit Apple event, never forced) must finish before
  // inspecting the candidate or reopening it.
  await checked(
    'quit-updated',
    helper,
    ['earlier-quit', successorReceipt, environmentFile],
    25_000,
    automationEnvironment,
  );
  await checked(
    'all-exited',
    helper,
    ['earlier-absent', receipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  report.exitConfirmed = true;
  report.normalQuit = true;
  // (b) After the quit: only XTrace.app, identical files,
  // a valid signature, ticket and Gatekeeper assessment, data kept.
  report.afterQuit = {
    applications: requireBundleStep('quit', await entries('/Applications')),
    inventory: await inventory(candidateBundle),
  };
  if (JSON.stringify(report.afterQuit.inventory) !== JSON.stringify(report.restarted.inventory))
    throw new Error('App contents after normal quit differ from the updated app');
  await appIdentity(candidateBundle, release.version, 'after-quit-plist');
  await validateSignature(candidateBundle, 'after-quit-app');
  report.quarantine.afterQuit = await quarantineAt(candidateBundle, 'after-quit');
  await dataKept('After quit');
  report.databaseAfterUpdate = await database(source, env.XTRACE_DATA_DIR, checked);
  report.afterQuit.dataKept = true;
  // Reopen XTrace.app as a person would: Dashboard, no Welcome.
  markAttempt('reopen');
  await checked(
    'reopen',
    helper,
    ['launch', reopenReceipt, environmentFile],
    55_000,
    automationEnvironment,
  );
  const reopened = JSON.parse(await read(reopenReceipt, 'utf8'));
  report.reopenedPid = reopened.pid;
  await checked(
    'reopen-environment',
    helper,
    ['inspect', reopenReceipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  await observe(
    'reopened-dashboard',
    reopened.pid,
    candidateBundle,
    95_000,
    `Observed usable native ${release.version} Dashboard after reopening XTrace.app without Welcome`,
  );
  await checked(
    'quit-reopened',
    helper,
    ['quit', reopenReceipt, environmentFile],
    25_000,
    automationEnvironment,
  );
  await checked(
    'reopened-exited',
    helper,
    ['absent', reopenReceipt, environmentFile],
    10_000,
    automationEnvironment,
  );
  report.reopenExitConfirmed = true;
  report.reopened = {
    applications: requireBundleStep('reopened', await entries('/Applications')),
    inventory: await inventory(candidateBundle),
  };
  report.quarantine.installedAfterExit = await quarantineAt(
    candidateBundle,
    'installed-after-exit',
  );
  await dataKept('After reopening');
  report.sentinelRetained = true;
  report.database = await database(source, env.XTRACE_DATA_DIR, checked);
  report.nativeDatabaseVerified = true;
  // (c) No copy the updater set aside or extracted is left behind.
  const leftovers = [
    ...updaterLeftovers(await entries(env.TMPDIR)).map((name) => join(env.TMPDIR, name)),
    ...updaterLeftovers(await entries(tmpdir())).map((name) => join(tmpdir(), name)),
  ];
  if (leftovers.length) throw new Error(`Updater copies were left: ${leftovers.join(', ')}`);
  report.noLeftoverCopies = { applications: true, temporaryFolders: [env.TMPDIR, tmpdir()] };
}

export async function cleanupUpdate({
  report,
  launchAttempted,
  pressAttempted,
  reopenAttempted,
  receipt,
  successorReceipt,
  reopenReceipt,
  helper,
  environmentFile,
  automationEnvironment,
  checked,
  installed,
  staging,
  failure,
  exists,
  rm,
}) {
  try {
    if (launchAttempted && !report.exitConfirmed) {
      if (!(await exists(receipt)))
        throw new Error('Launch outcome has no ownership receipt; app retained');
      await stopUpdateProcesses(
        { pressAttempted, successorExists: await exists(successorReceipt) },
        {
          adopt: () =>
            checked(
              'cleanup-adopt',
              helper,
              ['earlier-adopt-update', receipt, environmentFile],
              60_000,
              automationEnvironment,
            ),
          stopOriginal: () =>
            checked(
              'cleanup-stop-original',
              helper,
              ['earlier-stop', receipt, environmentFile],
              20_000,
              automationEnvironment,
            ),
          stopSuccessor: () =>
            checked(
              'cleanup-stop-successor',
              helper,
              ['earlier-stop', successorReceipt, environmentFile],
              20_000,
              automationEnvironment,
            ),
          confirmAbsent: () =>
            checked(
              'cleanup-all-exited',
              helper,
              ['earlier-absent', receipt, environmentFile],
              10_000,
              automationEnvironment,
            ),
        },
      );
      report.exitConfirmed = true;
    }
    if (reopenAttempted && !report.reopenExitConfirmed) {
      if (!(await exists(reopenReceipt)))
        throw new Error('Reopen outcome has no ownership receipt; app retained');
      await checked(
        'cleanup-stop-reopened',
        helper,
        ['stop', reopenReceipt, environmentFile],
        20_000,
        automationEnvironment,
      );
      await checked(
        'cleanup-reopened-exited',
        helper,
        ['absent', reopenReceipt, environmentFile],
        10_000,
        automationEnvironment,
      );
      report.reopenExitConfirmed = true;
    }
    // Baseline proved this fixed installation path did not exist. Remove
    // each owned bundle once, only after all process exits are confirmed.
    if (installed)
      for (const bundle of new Set([earlierBundle, candidateBundle])) {
        if (!(await exists(bundle))) continue;
        await checked(
          'unregister-app',
          '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
          ['-u', bundle],
        );
        await rm(bundle, { recursive: true });
      }
    if (staging) await rm(staging, { recursive: true });
    report.cleanupConfirmed = true;
  } catch (error) {
    report.cleanupError = error.message;
    failure ||= error;
  }

  return failure;
}

export async function prepareUpdateInstall(
  {
    report,
    owned,
    output,
    feedPath,
    fetch,
    checked,
    exists,
    markInstalled,
    appIdentity,
    validateSignature,
    quarantineAt,
  },
  effects = {},
) {
  const {
    readFile: read = readFile,
    writeFile: write = writeFile,
    readdir: entries = readdir,
    verifyProductionFeed: feed = verifyProductionFeed,
    verifyPreviousDownload: download = verifyPreviousDownload,
    verifyPreviousInventory: inventory = verifyPreviousInventory,
  } = effects;
  await fetch('production-feed', productionFeed, feedPath, 65_536);
  report.productionFeed = feed(await read(feedPath));
  await write(join(output, 'production-feed.json'), await read(feedPath));
  const archive = join(owned, previousRelease.archive);
  await fetch(
    'old-download',
    `https://github.com/XTraceAI/xtrace/releases/download/${previousRelease.tag}/${previousRelease.archive}`,
    archive,
    previousRelease.bytes,
  );
  download(await read(archive));
  report.downloadVerified = true;
  report.quarantine = {
    downloadMethod: 'anonymous curl; metadata unchanged by harness',
    archive: await quarantineAt(archive, 'old-archive'),
    browserDownloadTested: false,
    securityPromptsAutomaticallyApproved: false,
  };
  // Only the exact reviewed hash above can reach extraction. Its 102-member
  // regular-file/directory map was checked before this pin was committed.
  await checked('old-extract', '/usr/bin/tar', ['-xzf', archive, '-C', owned], 60_000);
  const extracted = join(owned, basename(earlierBundle));
  report.extractedInventory = await inventory(extracted);
  if ((await exists(earlierBundle)) || (await exists(candidateBundle)))
    throw new Error('Installation path appeared after baseline');
  markInstalled();
  await checked('install', '/usr/bin/ditto', [extracted, earlierBundle], 60_000);
  report.installedInventory = await inventory(earlierBundle);
  requireBundleStep('installed', await entries('/Applications'));
  await appIdentity(earlierBundle, previousRelease.version, 'old-plist');
  await validateSignature(earlierBundle, 'old-app');
  report.quarantine.installedBeforeLaunch = await quarantineAt(
    earlierBundle,
    'installed-before-launch',
  );
}
