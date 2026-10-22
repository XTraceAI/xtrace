import { spawnSync } from 'node:child_process';
import { lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { requireFreshBaseline } from './hosted-validation.mjs';
import { signatureProblems, notarizedByGatekeeper } from './release-dmg.mjs';
import {
  release,
  signedLaunchLimits,
  firstLaunchBaselinePaths,
  firstLaunchEnvironment,
  requireReleaseValues,
  verifyAppInventory,
  verifyHostedDatabase,
  executionBudget,
} from './hosted-release.mjs';
import {
  previousRelease,
  updateBudget,
  productionFeed,
  sentinel,
  requireUpdateHost,
  requireBundleStep,
  requireAppIdentity,
  requireSameDatabase,
  updaterLeftovers,
  verifyPreviousDownload,
  verifyPreviousInventory,
  verifyProductionFeed,
  stopUpdateProcesses,
} from './hosted-update-checks.mjs';

const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
// The published earlier app is installed, and updated in place, here; a
// normal quit of the updated app renames it to the candidate's folder.
const earlierBundle = previousRelease.bundle;
const renamedBundle = release.bundle;
async function exists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error.code === 'ENOENT') return false;
    throw error;
  }
}

async function main() {
  const output = resolve('artifacts/hosted-update');
  await mkdir(output, { recursive: true });
  const report = {
    status: 'failed',
    mode: 'update',
    release,
    previousRelease,
    validationLimits: signedLaunchLimits,
    releasedAppLaunched: false,
    commandBudget: updateBudget,
  };
  const budget = executionBudget(undefined, updateBudget);
  let staging,
    installed = false,
    launchAttempted = false,
    pressAttempted = false,
    reopenAttempted = false,
    failure;
  let helper, receipt, successorReceipt, reopenReceipt, environmentFile, automationEnvironment;
  const run = (command, args, timeout = 30_000, env = systemEnvironment) =>
    spawnSync(command, args, {
      encoding: 'utf8',
      env,
      timeout: budget.remaining(timeout),
      killSignal: 'SIGKILL',
      maxBuffer: 4 * 1024 * 1024,
    });
  const checked = async (name, command, args, timeout, env) => {
    const result = run(command, args, timeout, env);
    await writeFile(join(output, `${name}.log`), `${result.stdout || ''}\n${result.stderr || ''}`);
    if (result.error || result.signal || result.status !== 0)
      throw new Error(
        `${name} failed (${result.error?.code || result.signal || result.status}); see its log`,
      );
    return `${result.stdout || ''}${result.stderr || ''}`.trim();
  };
  const validateSignature = async (path, name) => {
    await checked(`${name}-strict`, '/usr/bin/codesign', ['--verify', '--deep', '--strict', path]);
    const identity = await checked(`${name}-identity`, '/usr/bin/codesign', [
      '-dv',
      '--verbose=4',
      path,
    ]);
    if (
      signatureProblems(identity, release.team).length ||
      !identity.includes('Authority=Developer ID Application: XTrace Inc. (3C8BCX5J3H)')
    )
      throw new Error(`${name} does not have the required Developer ID signature`);
    await checked(`${name}-staple`, '/usr/bin/xcrun', ['stapler', 'validate', path], 60_000);
    const assessment = await checked(
      `${name}-gatekeeper`,
      '/usr/sbin/spctl',
      ['--assess', '--type', 'execute', '--verbose=4', path],
      60_000,
    );
    if (!notarizedByGatekeeper(assessment))
      throw new Error(`${name} was not accepted as notarized by Gatekeeper`);
  };
  const quarantineAt = async (path, name) => {
    const names = await checked(`${name}-attributes`, '/usr/bin/xattr', [path]);
    return names.split('\n').includes('com.apple.quarantine')
      ? checked(`${name}-quarantine`, '/usr/bin/xattr', ['-p', 'com.apple.quarantine', path])
      : null;
  };
  const appIdentity = async (bundle, version, name) =>
    requireAppIdentity(
      JSON.parse(
        await checked(name, '/usr/bin/plutil', [
          '-convert',
          'json',
          '-o',
          '-',
          join(bundle, 'Contents/Info.plist'),
        ]),
      ),
      version,
    );
  try {
    if (process.argv.length !== 2) throw new Error('Update runner accepts no positional overrides');
    const head = run('/usr/bin/git', ['rev-parse', 'HEAD']).stdout?.trim();
    const osVersion =
      process.platform === 'darwin'
        ? run('/usr/bin/sw_vers', ['-productVersion']).stdout?.trim()
        : '';
    requireUpdateHost({
      mode: 'update',
      allowReleaseIdentity: process.env.ALLOW_RELEASE_IDENTITY,
      platform: process.platform,
      arch: process.arch,
      osVersion,
      env: process.env,
      head,
    });
    requireReleaseValues();
    Object.assign(report, {
      head,
      osVersion,
      architecture: process.arch,
      imageOS: process.env.ImageOS,
      imageVersion: process.env.ImageVersion,
    });
    const consoleUser = await checked('console', '/usr/bin/stat', ['-f', '%Su', '/dev/console']);
    if (!consoleUser || ['root', 'loginwindow'].includes(consoleUser))
      throw new Error('No interactive console user');
    const baseline = firstLaunchBaselinePaths(homedir());
    const present = await Promise.all(baseline.map(exists));
    requireFreshBaseline(
      baseline.filter((_, at) => present[at]),
      await checked('process-baseline', '/bin/ps', ['-axo', 'pid=,comm=']),
    );
    const keychain = run('/usr/bin/security', [
      'find-generic-password',
      '-s',
      'Claude Code-credentials',
    ]);
    if (keychain.error || keychain.signal || keychain.status !== 44)
      throw new Error(
        'Claude account Keychain absence was not confirmed; no credential values were requested or logged',
      );
    report.freshBaseline = true;
    staging = await mkdtemp(join(output, 'run-'));
    const owned = join(staging, 'staging.noindex');
    await mkdir(owned);
    const env = firstLaunchEnvironment(owned);
    for (const key of [
      'TMPDIR',
      'XTRACE_DATA_DIR',
      'XTRACE_NATIVE_HOME',
      'CLAUDE_CONFIG_DIR',
      'GH_CONFIG_DIR',
    ])
      await mkdir(env[key]);
    environmentFile = join(owned, 'environment.json');
    await writeFile(environmentFile, JSON.stringify(env));
    receipt = join(output, 'launch-receipt.json');
    successorReceipt = receipt + '.successor.json';
    reopenReceipt = join(output, 'reopen-receipt.json');
    if ((await exists(receipt)) || (await exists(reopenReceipt)))
      throw new Error('A prior launch receipt exists');
    automationEnvironment = {
      ...systemEnvironment,
      ALLOW_RELEASE_IDENTITY: 'true',
      GITHUB_ACTIONS: 'true',
      RUNNER_ENVIRONMENT: 'github-hosted',
      GITHUB_EVENT_NAME: 'workflow_dispatch',
    };
    const source = dirname(fileURLToPath(import.meta.url));
    helper = join(owned, 'launch');
    const driver = join(owned, 'ax');
    for (const [name, binary] of [
      ['hosted-launch', helper],
      ['hosted-ax', driver],
    ])
      await checked(
        `${name}-compile`,
        '/usr/bin/xcrun',
        [
          'swiftc',
          '-module-cache-path',
          join(owned, 'compiler-cache'),
          join(source, `${name}.swift`),
          '-o',
          binary,
        ],
        120_000,
        env,
      );
    // Exact bytes authenticate the retained public releases; no mutable tag/API
    // response can substitute another archive, feed, or installed app.
    const feedPath = join(owned, 'latest.json');
    const fetch = async (name, url, destination, maxBytes) =>
      checked(
        name,
        '/usr/bin/curl',
        [
          '--disable',
          '--noproxy',
          '*',
          '--fail',
          '--location',
          '--silent',
          '--show-error',
          '--proto',
          '=https',
          '--proto-redir',
          '=https',
          '--max-time',
          '90',
          '--max-filesize',
          String(maxBytes),
          '--output',
          destination,
          url,
        ],
        95_000,
      );
    await fetch('production-feed', productionFeed, feedPath, 65_536);
    report.productionFeed = verifyProductionFeed(await readFile(feedPath));
    await writeFile(join(output, 'production-feed.json'), await readFile(feedPath));
    const archive = join(owned, previousRelease.archive);
    await fetch(
      'old-download',
      `https://github.com/XTraceAI/xtrace-desktop/releases/download/${previousRelease.tag}/${previousRelease.archive}`,
      archive,
      previousRelease.bytes,
    );
    verifyPreviousDownload(await readFile(archive));
    report.downloadVerified = true;
    report.quarantine = {
      downloadMethod: 'anonymous curl; metadata unchanged by harness',
      archive: await quarantineAt(archive, 'old-archive'),
      browserDownloadTested: false,
      securityPromptsAutomaticallyApproved: false,
    };
    // Only the exact reviewed hash above can reach extraction. Its 79-member
    // regular-file/directory map was checked before this pin was committed.
    await checked('old-extract', '/usr/bin/tar', ['-xzf', archive, '-C', owned], 60_000);
    const extracted = join(owned, basename(earlierBundle));
    report.extractedInventory = await verifyPreviousInventory(extracted);
    if ((await exists(earlierBundle)) || (await exists(renamedBundle)))
      throw new Error('Installation path appeared after baseline');
    installed = true;
    await checked('install', '/usr/bin/ditto', [extracted, earlierBundle], 60_000);
    report.installedInventory = await verifyPreviousInventory(earlierBundle);
    requireBundleStep('installed', await readdir('/Applications'));
    await appIdentity(earlierBundle, previousRelease.version, 'old-plist');
    await validateSignature(earlierBundle, 'old-app');
    report.quarantine.installedBeforeLaunch = await quarantineAt(
      earlierBundle,
      'installed-before-launch',
    );
    const sentinelPath = join(env.XTRACE_DATA_DIR, 'update-sentinel.txt');
    const databasePath = join(env.XTRACE_DATA_DIR, 'xtrace.db');
    await writeFile(sentinelPath, sentinel, { flag: 'wx' });
    if (await exists(databasePath)) throw new Error('Synthetic database must start absent');
    const databaseIdentity = async () => {
      const { ino, dev, birthtimeMs } = await lstat(databasePath);
      return { ino, dev, birthtimeMs };
    };
    const dataKept = async (step) => {
      if ((await readFile(sentinelPath, 'utf8')) !== sentinel)
        throw new Error(`${step}: synthetic sentinel changed`);
      requireSameDatabase(report.databaseIdentity, await databaseIdentity(), step);
      for (const id of ['ai.xtrace.app', 'ai.xtrace.desktop'])
        if (await exists(join(homedir(), 'Library/Application Support', id, 'xtrace.db')))
          throw new Error('App wrote a database outside the owned data directory');
    };
    launchAttempted = true;
    await checked(
      'launch-services',
      helper,
      ['earlier-launch', receipt, environmentFile],
      55_000,
      automationEnvironment,
    );
    const ownership = JSON.parse(await readFile(receipt, 'utf8'));
    report.releasedAppLaunched = true;
    report.pid = ownership.pid;
    await checked(
      'process-environment',
      helper,
      ['earlier-inspect', receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    const observe = async (mode, pid, bundle, timeout, reason) => {
      const result = run(
        driver,
        [String(pid), bundle, release.identifier, mode],
        timeout,
        automationEnvironment,
      );
      await writeFile(
        join(output, `${mode}.log`),
        `${result.stdout || ''}\n${result.stderr || ''}`,
      );
      if (result.error || result.signal || result.status !== 0)
        throw new Error(`${mode} UI did not pass; see its log`);
      const evidence = JSON.parse(result.stdout);
      if (evidence.code !== 0 || evidence.reason !== reason || !Array.isArray(evidence.nodes))
        throw new Error(`${mode} returned no required UI evidence`);
      await writeFile(join(output, `${mode}.json`), `${JSON.stringify(evidence, null, 2)}\n`);
    };
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
    report.beforePressInventory = await verifyPreviousInventory(earlierBundle);
    await checked(
      'arm-update',
      helper,
      ['earlier-arm-update', receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    pressAttempted = true; // AX success/timeout can race process exit; cleanup must resolve both.
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
    const successor = JSON.parse(await readFile(successorReceipt, 'utf8'));
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
    // (a) After the restart: still the earlier folder, now holding the
    // candidate's exact files, relaunched as a distinct process, data kept.
    report.restarted = {
      applications: requireBundleStep('restarted', await readdir('/Applications')),
      inventory: await verifyAppInventory(earlierBundle),
    };
    await appIdentity(earlierBundle, release.version, 'updated-plist');
    await validateSignature(earlierBundle, 'updated-app');
    await dataKept('After restart');
    report.restarted.dataKept = true;
    // A normal quit (the Quit Apple event, never forced) runs the app's own
    // exit path, which renames the earlier folder.
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
    // (b) After the quit: only the candidate's folder name, identical files,
    // a valid signature, ticket and Gatekeeper assessment, data kept.
    report.renamed = {
      applications: requireBundleStep('quit', await readdir('/Applications')),
      inventory: await verifyAppInventory(renamedBundle),
    };
    if (JSON.stringify(report.renamed.inventory) !== JSON.stringify(report.restarted.inventory))
      throw new Error('Renamed app contents differ from the updated app');
    await appIdentity(renamedBundle, release.version, 'renamed-plist');
    await validateSignature(renamedBundle, 'renamed-app');
    report.quarantine.renamedAfterQuit = await quarantineAt(renamedBundle, 'renamed-after-quit');
    await dataKept('After quit');
    report.databaseAfterUpdate = await verifyHostedDatabase(source, env.XTRACE_DATA_DIR, checked);
    report.renamed.dataKept = true;
    // Reopen the renamed folder as a person would: Dashboard, no Welcome.
    reopenAttempted = true;
    await checked(
      'reopen',
      helper,
      ['launch', reopenReceipt, environmentFile],
      55_000,
      automationEnvironment,
    );
    const reopened = JSON.parse(await readFile(reopenReceipt, 'utf8'));
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
      renamedBundle,
      95_000,
      `Observed usable native ${release.version} Dashboard after reopening the renamed app without Welcome`,
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
      applications: requireBundleStep('reopened', await readdir('/Applications')),
      inventory: await verifyAppInventory(renamedBundle),
    };
    report.quarantine.installedAfterExit = await quarantineAt(
      renamedBundle,
      'installed-after-exit',
    );
    await dataKept('After reopening');
    report.sentinelRetained = true;
    report.database = await verifyHostedDatabase(source, env.XTRACE_DATA_DIR, checked);
    report.nativeDatabaseVerified = true;
    // (c) No copy the updater set aside or extracted is left behind.
    const leftovers = [
      ...updaterLeftovers(await readdir(env.TMPDIR)).map((name) => join(env.TMPDIR, name)),
      ...updaterLeftovers(await readdir(tmpdir())).map((name) => join(tmpdir(), name)),
    ];
    if (leftovers.length) throw new Error(`Updater copies were left: ${leftovers.join(', ')}`);
    report.noLeftoverCopies = { applications: true, temporaryFolders: [env.TMPDIR, tmpdir()] };
  } catch (error) {
    if (error.message.startsWith('Production feed prerequisite unmet:'))
      report.status = 'prerequisite-unmet';
    failure = error;
  }
  budget.beginCleanup();
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
    // Baseline proved neither folder existed, so whichever exists now is this
    // run's: the installed earlier folder or the app's own rename of it.
    if (installed)
      for (const [name, bundle] of [
        ['unregister-earlier', earlierBundle],
        ['unregister-renamed', renamedBundle],
      ]) {
        if (!(await exists(bundle))) continue;
        await checked(
          name,
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

  if (failure) report.error = failure.message;
  else report.status = 'passed';
  await writeFile(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  if (failure) throw failure;
  console.log(
    `Pinned signed ${previousRelease.version} updated through its real UI to ${release.version} with synthetic data and telemetry disabled: distinct restart in "${basename(earlierBundle)}", rename to "${basename(renamedBundle)}" on a normal quit with identical signed contents, reopened Dashboard without Welcome, retained sentinel and database, no updater copies left. The default telemetry notice and default telemetry behavior were not validated.`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
