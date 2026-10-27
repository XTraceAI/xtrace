import {
  runUpdateLifecycle,
  cleanupUpdate,
  prepareUpdateInstall,
} from './hosted-update-lifecycle.mjs';
import { writeHostedSwift } from './hosted-swift.mjs';
import { spawnSync } from 'node:child_process';
import { lstat, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
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
  executionBudget,
} from './hosted-release.mjs';
import {
  previousRelease,
  updateBudget,
  sentinel,
  requireUpdateHost,
  requireAppIdentity,
  requireSameDatabase,
} from './hosted-update-checks.mjs';

const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
// The published earlier app is installed and updated in place at XTrace.app.
const earlierBundle = previousRelease.bundle;
const candidateBundle = release.bundle;
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
          await writeHostedSwift(source, name, join(owned, `${name}.swift`)),
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
    const sentinelPath = join(env.XTRACE_DATA_DIR, 'update-sentinel.txt');
    const databasePath = join(env.XTRACE_DATA_DIR, 'xtrace.db');
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
    await runUpdateLifecycle({
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
      prepare: async () => {
        await prepareUpdateInstall({
          report,
          owned,
          output,
          feedPath,
          fetch,
          checked,
          exists,
          markInstalled: () => {
            installed = true;
          },
          appIdentity,
          validateSignature,
          quarantineAt,
        });
        await writeFile(sentinelPath, sentinel, { flag: 'wx' });
        if (await exists(databasePath)) throw new Error('Synthetic database must start absent');
      },
      markAttempt: (kind) => {
        if (kind === 'launch') launchAttempted = true;
        else if (kind === 'press') pressAttempted = true;
        else if (kind === 'reopen') reopenAttempted = true;
      },
      appIdentity,
      validateSignature,
      quarantineAt,
    });
  } catch (error) {
    if (error.message.startsWith('Production feed prerequisite unmet:'))
      report.status = 'prerequisite-unmet';
    failure = error;
  }
  budget.beginCleanup();
  failure = await cleanupUpdate({
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
  });

  if (failure) report.error = failure.message;
  else report.status = 'passed';
  await writeFile(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  if (failure) throw failure;
  console.log(
    `Pinned signed ${previousRelease.version} updated through its real UI to ${release.version} with synthetic data and telemetry disabled: distinct restart in "${basename(earlierBundle)}", normal quit at "${basename(candidateBundle)}" with identical signed contents, reopened Dashboard without Welcome, retained sentinel and database, no updater copies left. The default telemetry notice and default telemetry behavior were not validated.`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
