import { spawnSync } from 'node:child_process';
import { lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  telemetryBudget,
  telemetryLimits,
  telemetryOwnedPaths,
  requireTelemetryHost,
} from './hosted-telemetry.mjs';
import {
  runTelemetryPhases,
  telemetrySession,
  stopTelemetryOwners,
  stopTelemetryProxies,
  telemetryCleanupFacts,
  cleanupTelemetry,
  unregisterTelemetryProbe,
  telemetryProxiesHealthy,
  removeTelemetryProfiles,
} from './hosted-telemetry-run.mjs';
import { requireFreshBaseline } from './hosted-validation.mjs';
import { signatureProblems, notarizedByGatekeeper } from './release-dmg.mjs';
import { inspectMetadataResponse, readMetadataCapture, metadataLimit } from './hosted-metadata.mjs';
import {
  release,
  signedLaunchLimits,
  firstLaunchBaselinePaths,
  firstLaunchEnvironment,
  requireFirstLaunchHost,
  verifyAppInventory,
  verifyHostedDatabase,
  verifyDownload,
  verifyTag,
  executionBudget,
  firstLaunchBudget,
} from './hosted-release.mjs';

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
  const telemetry = process.argv.length === 3 && process.argv[2] === 'telemetry';
  const mode = telemetry ? 'telemetry' : 'first-launch';
  const limits = telemetry ? telemetryBudget : firstLaunchBudget;
  const session = telemetry ? telemetrySession() : null;
  const output = resolve(`artifacts/hosted-${mode}`);
  await mkdir(output, { recursive: true });
  const report = {
    status: 'failed',
    mode,
    release,
    validationLimits: telemetry ? telemetryLimits : signedLaunchLimits,
    releasedAppLaunched: false,
    commandBudget: limits,
  };
  const budget = executionBudget(undefined, limits);
  let staging,
    installed = false,
    mount,
    attached = false,
    launchAttempted = false,
    failure;
  let helper, receipt, environmentFile, automationEnvironment;
  let telemetryContext;
  let proxyStopReserve = 0;
  const run = (command, args, timeout = 30_000, env = systemEnvironment) => {
    const remaining = budget.remaining(timeout + proxyStopReserve) - proxyStopReserve;
    if (remaining <= 0) throw new Error('Command would consume reserved proxy cleanup time');
    return spawnSync(command, args, {
      encoding: 'utf8',
      env,
      timeout: remaining,
      killSignal: 'SIGKILL',
      maxBuffer: 4 * 1024 * 1024,
    });
  };
  const checked = async (name, command, args, timeout, env) => {
    const result = run(command, args, timeout, env);
    await writeFile(join(output, `${name}.log`), `${result.stdout || ''}\n${result.stderr || ''}`);
    if (result.error || result.signal || result.status !== 0)
      throw new Error(
        `${name} failed (${result.error?.code || result.signal || result.status}); see its log`,
      );
    return `${result.stdout || ''}${result.stderr || ''}`.trim();
  };
  const validateSignature = async (path, name, dmg = false) => {
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
      [
        '--assess',
        '--type',
        dmg ? 'open' : 'execute',
        ...(dmg ? ['--context', 'context:primary-signature'] : []),
        '--verbose=4',
        path,
      ],
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
    if (process.argv.length !== 2 && !telemetry)
      throw new Error('Runner accepts only the explicit telemetry mode; no positional overrides');
    const head = run('/usr/bin/git', ['rev-parse', 'HEAD']).stdout?.trim();
    const osVersion =
      process.platform === 'darwin'
        ? run('/usr/bin/sw_vers', ['-productVersion']).stdout?.trim()
        : '';
    (telemetry ? requireTelemetryHost : requireFirstLaunchHost)({
      mode,
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
    if (telemetry && (await realpath(homedir())) !== homedir())
      throw new Error('Hosted home must be canonical');
    const baseline = [
      ...firstLaunchBaselinePaths(homedir()),
      ...(telemetry ? telemetryOwnedPaths(homedir()) : []),
    ];
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
    if (session) session.stagingOwned = true;
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
    await checked('curl-version', '/usr/bin/curl', ['--disable', '--version']);
    const readTagMetadata = async (name, url) => {
      const headers = join(owned, `${name}-headers`);
      const body = join(owned, `${name}-body`);
      const result = run(
        '/usr/bin/curl',
        [
          '--disable',
          '--noproxy',
          '*',
          '--fail-with-body',
          '--silent',
          '--show-error',
          '--proto',
          '=https',
          '--max-time',
          '60',
          '--max-filesize',
          String(metadataLimit),
          '--dump-header',
          headers,
          '--output',
          body,
          '--write-out',
          '%{http_code}',
          url,
        ],
        65_000,
      );
      const response = inspectMetadataResponse(
        result,
        await readMetadataCapture(headers),
        await readMetadataCapture(body),
      );
      await writeFile(
        join(output, `${name}-http.json`),
        `${JSON.stringify(response.diagnostic, null, 2)}\n`,
      );
      if (response.failure) throw new Error(`${name}: ${response.failure}; see ${name}-http.json`);
      await writeFile(join(output, `${name}.json`), `${JSON.stringify(response.value, null, 2)}\n`);
      return response.value;
    };
    report.tagSource = await verifyTag(
      await readTagMetadata(
        'release-tag',
        `https://api.github.com/repos/XTraceAI/xtrace-desktop/git/ref/tags/${release.tag}`,
      ),
      (url) => readTagMetadata('release-tag-annotation', url),
    );
    const dmg = join(owned, release.dmg);
    await checked(
      'download',
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
        String(release.bytes),
        '--output',
        dmg,
        `https://github.com/XTraceAI/xtrace-desktop/releases/download/${release.tag}/${release.dmg}`,
      ],
      95_000,
    );
    verifyDownload(await readFile(dmg));
    report.downloadVerified = true;
    // Observe metadata only. curl is not a browser download; normal system
    // quarantine decisions and any security prompts are left untouched.
    report.quarantine = {
      downloadMethod: 'anonymous curl; metadata unchanged by harness',
      dmg: await quarantineAt(dmg, 'dmg'),
      browserDownloadTested: false,
      securityPromptsAutomaticallyApproved: false,
    };
    await validateSignature(dmg, 'dmg', true);
    await checked('dmg-integrity', '/usr/bin/hdiutil', ['verify', dmg], 60_000);
    mount = join(owned, 'mount');
    await mkdir(mount);
    attached = true; // Even failed attachment can leave a mounted volume.
    await checked(
      'dmg-mount',
      '/usr/bin/hdiutil',
      ['attach', dmg, '-readonly', '-nobrowse', '-noautoopen', '-mountpoint', mount],
      60_000,
    );
    report.mountedInventory = await verifyAppInventory(join(mount, 'XTrace Desktop.app'));
    report.quarantine.mountedApp = await quarantineAt(
      join(mount, 'XTrace Desktop.app'),
      'mounted-app',
    );
    if (await exists(release.bundle))
      throw new Error('Installation path appeared after baseline check');
    installed = true;
    await checked(
      'install',
      '/usr/bin/ditto',
      [join(mount, 'XTrace Desktop.app'), release.bundle],
      60_000,
    );
    await checked('dmg-detach', '/usr/bin/hdiutil', ['detach', mount], 30_000);
    attached = false;
    report.installedInventory = await verifyAppInventory(release.bundle);
    const plist = await checked('app-plist', '/usr/bin/plutil', [
      '-convert',
      'json',
      '-o',
      '-',
      join(release.bundle, 'Contents/Info.plist'),
    ]);
    const info = JSON.parse(plist);
    if (
      info.CFBundleIdentifier !== release.identifier ||
      info.CFBundleShortVersionString !== release.version ||
      info.CFBundleVersion !== release.version ||
      info.CFBundleExecutable !== 'xtrace-desktop'
    )
      throw new Error('Installed app identity/version mismatch');
    await validateSignature(release.bundle, 'app');
    report.quarantine.installedBeforeLaunch = await quarantineAt(
      release.bundle,
      'installed-before-launch',
    );
    if (telemetry) {
      telemetryContext = {
        source,
        owned,
        output,
        home: homedir(),
        checked,
        helper,
        driver,
        automationEnvironment,
        budget,
        compilerEnvironment: env,
      };
      report.telemetryPhases = session.phases;
      await runTelemetryPhases(telemetryContext, session);
      report.releasedAppLaunched = true;
      report.nativeDatabaseVerified = true;
      report.quarantine.installedAfterExit = await quarantineAt(
        release.bundle,
        'installed-after-exit',
      );
    } else {
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
      const ui = run(
        driver,
        [String(ownership.pid), release.bundle, release.identifier, 'first-launch'],
        100_000,
        automationEnvironment,
      );
      await writeFile(join(output, 'ui.log'), `${ui.stdout || ''}\n${ui.stderr || ''}`);
      if (ui.error || ui.signal || ui.status !== 0)
        throw new Error('First-launch UI did not pass; see ui.log');
      const evidence = JSON.parse(ui.stdout);
      if (
        evidence.code !== 0 ||
        evidence.reason !==
          'Observed native-ready Welcome, Open Dashboard action and usable 0.1.7 Dashboard'
      )
        throw new Error('Driver returned no required first-launch evidence');
      await writeFile(join(output, 'ui.json'), `${JSON.stringify(evidence, null, 2)}\n`);
      await checked(
        'stop',
        helper,
        ['stop', receipt, environmentFile],
        20_000,
        automationEnvironment,
      );
      report.exitConfirmed = true;
      report.afterExitInventory = await verifyAppInventory(release.bundle);
      report.quarantine.installedAfterExit = await quarantineAt(
        release.bundle,
        'installed-after-exit',
      );
      if (!(await exists(join(env.XTRACE_DATA_DIR, 'xtrace.db'))))
        throw new Error('Owned data directory has no database');
      for (const id of ['ai.xtrace.app', 'ai.xtrace.desktop'])
        if (await exists(join(homedir(), 'Library/Application Support', id, 'xtrace.db')))
          throw new Error('App wrote a database outside the owned data directory');
      report.database = await verifyHostedDatabase(source, env.XTRACE_DATA_DIR, checked);
      report.nativeDatabaseVerified = true;
    }
  } catch (error) {
    failure = error;
  }
  budget.beginCleanup();
  proxyStopReserve = session
    ? session.proxies.filter((proxy) => proxy.exit?.exitConfirmed !== true).length * 8000
    : 0;
  const removeOwned = async () => {
    if (session) {
      await telemetryProxiesHealthy(session);
      await unregisterTelemetryProbe(session, checked);
    }
    if (installed) {
      await checked(
        'unregister',
        '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
        ['-u', release.bundle],
      );
      await rm(release.bundle, { recursive: true });
    }
    if (attached) {
      await checked('cleanup-detach', '/usr/bin/hdiutil', ['detach', mount], 30_000);
      attached = false;
    }
    if (session) {
      await removeTelemetryProfiles(session);
      session.appAndProfilesRemoved = true;
    }
  };
  try {
    if (session) {
      await cleanupTelemetry(session, {
        stopOwners: async () => {
          if (telemetryContext) await stopTelemetryOwners(session, telemetryContext);
        },
        removeOwned,
        stopProxies: () => {
          proxyStopReserve = 0;
          return stopTelemetryProxies(session, output, { budget });
        },
      });
    } else {
      if (launchAttempted && !report.exitConfirmed) {
        if (!(await exists(receipt)))
          throw new Error('Launch outcome has no ownership receipt; app retained');
        await checked(
          'cleanup-stop',
          helper,
          ['stop', receipt, environmentFile],
          20_000,
          automationEnvironment,
        );
        report.exitConfirmed = true;
      }
      await removeOwned();
    }
    if (staging) {
      if (session) session.stagingRemovalAttempted = true;
      await rm(staging, { recursive: true });
    }
    if (session) session.stagingRemoved = true;
    report.cleanupConfirmed = true;
  } catch (error) {
    if (session) session.cleanupFailed = true;
    report.cleanupError = error.message;
    if (error.proxyCleanupError) report.proxyCleanupError = error.proxyCleanupError;
    failure ||= error;
  }
  if (session) {
    const attempts = session.owners.filter((owner) => owner.kind === 'telemetry');
    report.exitConfirmed =
      attempts.some((owner) => owner.launchConfirmed === true) &&
      attempts.every((owner) => owner.exited === true);
    report.releasedAppLaunchAttempted = attempts.length > 0;
    report.releasedAppLaunchOutcomeUnknown = attempts.some(
      (owner) => owner.launchConfirmed !== true,
    );
    report.releasedAppLaunched = attempts.some((owner) => owner.launchConfirmed === true)
      ? true
      : attempts.length
        ? null
        : false;
  }

  if (session) report.telemetryCleanup = telemetryCleanupFacts(session);
  if (failure) report.error = failure.message;
  else report.status = 'passed';
  await writeFile(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  if (failure) throw failure;
  console.log(
    telemetry
      ? 'Pinned signed 0.1.7 passed default telemetry notice, saved preferences, restart/override and rejected-attempt checks. Successful HTTP delivery, real PostHog delivery and before-first-pixel ordering were not validated.'
      : 'Pinned signed 0.1.7 passed hosted first-launch UI and empty native database checks with synthetic data and telemetry disabled. The default telemetry notice and default telemetry behavior were not validated. Updater was not tested.',
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
