import { spawnSync } from 'node:child_process';
import { lstat, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { requireFreshBaseline } from './hosted-validation.mjs';
import { signatureProblems, notarizedByGatekeeper } from './release-dmg.mjs';
import {
  release,
  firstLaunchBaselinePaths,
  firstLaunchEnvironment,
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
  verifyPreviousDownload,
  verifyPreviousInventory,
  verifyProductionFeed,
  stopUpdateProcesses,
} from './hosted-update-checks.mjs';

const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
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
    releasedAppLaunched: false,
    commandBudget: updateBudget,
  };
  const budget = executionBudget(undefined, updateBudget);
  let staging,
    installed = false,
    launchAttempted = false,
    pressAttempted = false,
    failure;
  let helper, receipt, successorReceipt, environmentFile, automationEnvironment;
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
    if (await exists(receipt)) throw new Error('A prior launch receipt exists');
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
    report.previousRelease = previousRelease;
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
    report.extractedInventory = await verifyPreviousInventory(join(owned, 'XTrace Desktop.app'));
    if (await exists(release.bundle)) throw new Error('Installation path appeared after baseline');
    installed = true;
    await checked(
      'install',
      '/usr/bin/ditto',
      [join(owned, 'XTrace Desktop.app'), release.bundle],
      60_000,
    );
    report.installedInventory = await verifyPreviousInventory(release.bundle);
    const plist = JSON.parse(
      await checked('old-plist', '/usr/bin/plutil', [
        '-convert',
        'json',
        '-o',
        '-',
        join(release.bundle, 'Contents/Info.plist'),
      ]),
    );
    if (
      plist.CFBundleIdentifier !== release.identifier ||
      plist.CFBundleShortVersionString !== previousRelease.version ||
      plist.CFBundleVersion !== previousRelease.version ||
      plist.CFBundleExecutable !== 'xtrace-desktop'
    )
      throw new Error('Old app identity/version mismatch');
    await validateSignature(release.bundle, 'old-app');
    report.quarantine.installedBeforeLaunch = await quarantineAt(
      release.bundle,
      'installed-before-launch',
    );
    await writeFile(join(env.XTRACE_DATA_DIR, 'update-sentinel.txt'), sentinel, { flag: 'wx' });
    if (await exists(join(env.XTRACE_DATA_DIR, 'xtrace.db')))
      throw new Error('Synthetic database must start absent');
    launchAttempted = true;
    await checked(
      'launch-services',
      helper,
      ['launch', receipt, environmentFile],
      55_000,
      automationEnvironment,
    );
    const ownership = JSON.parse(await readFile(receipt, 'utf8'));
    report.releasedAppLaunched = true;
    report.pid = ownership.pid;
    await checked(
      'process-environment',
      helper,
      ['inspect', receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    const observe = async (mode, pid, timeout, reason) => {
      const result = run(
        driver,
        [String(pid), release.bundle, release.identifier, mode],
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
      185_000,
      'Observed usable 0.1.6 Dashboard and target 0.1.7 ready',
    );
    // Ready means verified download, not installation. Prove old bytes remain
    // immediately before asking the shipped UI to install and restart itself.
    report.beforePressInventory = await verifyPreviousInventory(release.bundle);
    await checked(
      'arm-update',
      helper,
      ['arm-update', receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    pressAttempted = true; // AX success/timeout can race process exit; cleanup must resolve both.
    await observe(
      'update-press',
      ownership.pid,
      15_000,
      'Pressed real Restart to update once for target 0.1.7',
    );
    report.restartPressed = true;
    await checked(
      'adopt-update',
      helper,
      ['adopt-update', receipt, environmentFile],
      60_000,
      automationEnvironment,
    );
    const successor = JSON.parse(await readFile(successorReceipt, 'utf8'));
    report.originalExitConfirmed = true;
    report.successorPid = successor.pid;
    await checked(
      'successor-environment',
      helper,
      ['inspect', successorReceipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    await observe(
      'updated-dashboard',
      successor.pid,
      95_000,
      'Observed usable native 0.1.7 Dashboard and retained completed Welcome',
    );
    report.updatedInventory = await verifyAppInventory(release.bundle);
    await validateSignature(release.bundle, 'updated-app');
    await checked(
      'stop-successor',
      helper,
      ['stop', successorReceipt, environmentFile],
      20_000,
      automationEnvironment,
    );
    await checked(
      'all-exited',
      helper,
      ['absent', receipt, environmentFile],
      10_000,
      automationEnvironment,
    );
    report.exitConfirmed = true;
    report.afterExitInventory = await verifyAppInventory(release.bundle);
    report.quarantine.installedAfterExit = await quarantineAt(
      release.bundle,
      'installed-after-exit',
    );
    if ((await readFile(join(env.XTRACE_DATA_DIR, 'update-sentinel.txt'), 'utf8')) !== sentinel)
      throw new Error('Synthetic sentinel changed across update');
    report.sentinelRetained = true;
    if (!(await exists(join(env.XTRACE_DATA_DIR, 'xtrace.db'))))
      throw new Error('Owned data directory has no database');
    for (const id of ['ai.xtrace.app', 'ai.xtrace.desktop'])
      if (await exists(join(homedir(), 'Library/Application Support', id, 'xtrace.db')))
        throw new Error('App wrote a database outside the owned data directory');
    report.database = await verifyHostedDatabase(source, env.XTRACE_DATA_DIR, checked);
    report.nativeDatabaseVerified = true;
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
              ['adopt-update', receipt, environmentFile],
              60_000,
              automationEnvironment,
            ),
          stopOriginal: () =>
            checked(
              'cleanup-stop-original',
              helper,
              ['stop', receipt, environmentFile],
              20_000,
              automationEnvironment,
            ),
          stopSuccessor: () =>
            checked(
              'cleanup-stop-successor',
              helper,
              ['stop', successorReceipt, environmentFile],
              20_000,
              automationEnvironment,
            ),
          confirmAbsent: () =>
            checked(
              'cleanup-all-exited',
              helper,
              ['absent', receipt, environmentFile],
              10_000,
              automationEnvironment,
            ),
        },
      );
      report.exitConfirmed = true;
    }
    if (installed) {
      await checked(
        'unregister',
        '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
        ['-u', release.bundle],
      );
      await rm(release.bundle, { recursive: true });
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
    'Pinned signed 0.1.6 updated through its real UI to 0.1.7; distinct restart, retained sentinel and empty native database verified.',
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
