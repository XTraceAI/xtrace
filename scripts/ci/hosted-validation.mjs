import { spawn, spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { lstat, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isSmokeIdentifier, stopSmokeChild, unregisterSmokeBundle } from './debug-bundle.mjs';

const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
const shaPattern = /^[0-9a-f]{40}$/;
const number = '(0|[1-9][0-9]*)';
const prerelease = '(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
const tagPattern = new RegExp(
  `^refs/tags/v${number}\\.${number}\\.${number}(-${prerelease}(\\.${prerelease})*)?(\\+[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$`,
);

export function requireProbeHost({ mode, platform, arch, osVersion, env, head }) {
  // No opt-in environment variable or argument can enable released-app modes.
  if (mode !== 'probe') throw new Error('Only the harmless test-ID probe is implemented');
  if (
    platform !== 'darwin' ||
    arch !== 'arm64' ||
    !/^14\./.test(osVersion) ||
    env.GITHUB_ACTIONS !== 'true' ||
    env.GITHUB_EVENT_NAME !== 'workflow_dispatch' ||
    env.RUNNER_ENVIRONMENT !== 'github-hosted' ||
    env.RUNNER_OS !== 'macOS' ||
    env.RUNNER_ARCH !== 'ARM64'
  )
    throw new Error('Requires a fresh GitHub-hosted macOS 14 arm64 manual run');
  if (
    !shaPattern.test(head) ||
    head !== env.CANDIDATE_SHA ||
    head !== env.SOURCE_SHA ||
    head !== env.WORKFLOW_SHA
  )
    throw new Error('Checked-out source must match the exact workflow and candidate SHA');
  const defaultBranch =
    env.DEFAULT_BRANCH && env.WORKFLOW_REF === `refs/heads/${env.DEFAULT_BRANCH}`;
  if (
    !(defaultBranch
      ? env.WORKFLOW_REF_TYPE === 'branch'
      : env.WORKFLOW_REF_TYPE === 'tag' && tagPattern.test(env.WORKFLOW_REF))
  ) {
    throw new Error('Requires the default branch or an exact reviewed release tag');
  }
}

export function probeEnvironment(directory) {
  // Do not inherit HOME, account tokens, Actions credentials, shell startup files,
  // arbitrary PATH entries or provider settings. Never override the real HOME.
  return {
    ...systemEnvironment,
    TMPDIR: join(directory, 'tmp'),
    XTRACE_DATA_DIR: join(directory, 'data'),
    XTRACE_NATIVE_HOME: join(directory, 'home'),
    CLAUDE_CONFIG_DIR: join(directory, 'claude'),
    XTRACE_TELEMETRY: '0',
  };
}

export function baselinePaths(home) {
  return [
    '/Applications/XTrace Desktop.app',
    join(home, 'Applications/XTrace Desktop.app'),
    ...['ai.xtrace.app', 'ai.xtrace.desktop'].map((id) =>
      join(home, 'Library/Application Support', id),
    ),
    // Product provider discovery also checks these paths independently of PATH.
    '/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex',
    join(home, '.local/bin/codex'),
    join(home, '.local/bin/claude'),
    join(home, '.claude/local/claude'),
    ...['/opt/homebrew/bin', '/usr/local/bin', ...systemEnvironment.PATH.split(':')].flatMap(
      (directory) => ['codex', 'claude'].map((tool) => join(directory, tool)),
    ),
  ];
}

export function requireFreshBaseline(existingPaths, processes) {
  if (existingPaths.length)
    throw new Error(`Fresh-host baseline failed: ${existingPaths.join(', ')}`);
  if (
    processes
      .split('\n')
      .some((line) =>
        /\/XTrace Desktop\.app\/Contents\/MacOS\/|(?:^|[\s/])xtrace-desktop\s*$/i.test(line),
      )
  ) {
    throw new Error('An XTrace process already exists');
  }
}

export function requireDriverResult(result, expectedCode) {
  if (result.error || result.signal || result.status !== expectedCode) {
    throw new Error(
      `AX driver failed (expected ${expectedCode}, received ${result.status ?? result.signal})`,
    );
  }
  const report = JSON.parse(result.stdout);
  const reason =
    expectedCode === 3
      ? 'Missing control timed out without changing the fixture'
      : 'Observed window, AXPress and changed visible label';
  if (report.code !== expectedCode || report.reason !== reason || !Array.isArray(report.nodes)) {
    throw new Error('AX driver did not return the required evidence');
  }
  return report;
}

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
  const output = resolve('artifacts/hosted-validation');
  await mkdir(output, { recursive: true });
  const report = { status: 'failed', mode: process.argv[2], releaseValidated: false };
  let directory, child, prepared, failure;
  const run = (command, args, timeout = 30_000, env = systemEnvironment) =>
    spawnSync(command, args, { encoding: 'utf8', env, timeout, maxBuffer: 4 * 1024 * 1024 });
  const checked = (command, args, timeout, env) => {
    const result = run(command, args, timeout, env);
    if (result.error || result.signal || result.status !== 0) {
      throw new Error(
        `${command} failed: ${result.error?.message || result.stderr || result.signal || result.status}`,
      );
    }
    return result.stdout.trim();
  };
  try {
    if (process.argv.length !== 3) throw new Error('Usage: hosted-validation.mjs probe');
    const osVersion =
      process.platform === 'darwin' ? checked('/usr/bin/sw_vers', ['-productVersion']) : '';
    const head = checked('/usr/bin/git', ['rev-parse', 'HEAD']);
    requireProbeHost({
      mode: process.argv[2],
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
      consoleUser: checked('/usr/bin/stat', ['-f', '%Su', '/dev/console']),
    });
    if (['root', 'loginwindow'].includes(report.consoleUser))
      throw new Error('No interactive console user');
    const paths = baselinePaths(homedir());
    const present = await Promise.all(paths.map(exists));
    requireFreshBaseline(
      paths.filter((_, index) => present[index]),
      checked('/bin/ps', ['-axo', 'pid=,comm=']),
    );
    report.freshBaseline = true;
    directory = await mkdtemp(join(output, 'staging.noindex-'));
    // Keep the suffix .noindex on the actual staging directory, not just its prefix.
    const staging = join(directory, 'staging.noindex');
    await mkdir(staging);
    const env = probeEnvironment(staging);
    for (const key of ['TMPDIR', 'XTRACE_DATA_DIR', 'XTRACE_NATIVE_HOME', 'CLAUDE_CONFIG_DIR']) {
      await mkdir(env[key]);
    }
    const identifier = `ai.xtrace.app.test.${randomUUID()}`;
    if (!isSmokeIdentifier(identifier)) throw new Error('Invalid generated test identity');
    const bundle = join(staging, 'Hosted AX Probe.app');
    const executable = join(bundle, 'Contents/MacOS/HostedAXProbe');
    await mkdir(dirname(executable), { recursive: true });
    const sourceDirectory = dirname(fileURLToPath(import.meta.url));
    const fixture = (
      await readFile(join(sourceDirectory, 'hosted-probe-app.swift'), 'utf8')
    ).replace('__PROBE_IDENTIFIER__', identifier);
    const source = join(staging, 'main.swift');
    await writeFile(source, fixture);
    await writeFile(
      join(bundle, 'Contents/Info.plist'),
      `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${identifier}</string>
<key>CFBundleExecutable</key><string>HostedAXProbe</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleName</key><string>Hosted AX Probe</string><key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>\n`,
    );
    const cache = join(staging, 'compiler-cache');
    checked(
      '/usr/bin/xcrun',
      ['swiftc', '-module-cache-path', cache, source, '-o', executable],
      120_000,
      env,
    );
    const driver = join(staging, 'hosted-ax');
    checked(
      '/usr/bin/xcrun',
      [
        'swiftc',
        '-module-cache-path',
        cache,
        join(sourceDirectory, 'hosted-ax.swift'),
        '-o',
        driver,
      ],
      120_000,
      env,
    );
    checked('/usr/bin/codesign', ['--force', '--sign', '-', bundle]);
    checked('/usr/bin/codesign', ['--verify', '--deep', '--strict', bundle]);
    if (
      checked('/usr/libexec/PlistBuddy', [
        '-c',
        'Print :CFBundleIdentifier',
        join(bundle, 'Contents/Info.plist'),
      ]) !== identifier
    ) {
      throw new Error('Compiled fixture bundle identity mismatch');
    }
    prepared = { bundle, identifier };
    report.identifier = identifier;
    child = spawn(executable, [], { env, stdio: 'ignore' });
    await new Promise((accept, reject) => {
      child.once('spawn', accept);
      child.once('error', reject);
    });
    report.pid = child.pid;
    for (const [mode, expectedCode] of [
      ['missing-control', 3],
      ['press', 0],
    ]) {
      const result = run(driver, [String(child.pid), bundle, identifier, mode], 35_000, env);
      await writeFile(
        join(output, `${mode}.log`),
        `${result.stdout || ''}\n${result.stderr || ''}`,
      );
      const evidence = requireDriverResult(result, expectedCode);
      await writeFile(join(output, `${mode}.json`), `${JSON.stringify(evidence, null, 2)}\n`);
    }
    report.probePassed = true;
  } catch (error) {
    failure = error;
  } finally {
    try {
      if (child) await stopSmokeChild(child);
      if (prepared)
        unregisterSmokeBundle(prepared, dirname(prepared.bundle), (command, args) =>
          run(command, args),
        );
      if (directory) await rm(directory, { recursive: true, force: true });
      report.cleanupConfirmed = true;
    } catch (error) {
      report.cleanupError = error.message;
      failure ||= error;
    }
  }
  if (failure) report.error = failure.message;
  else report.status = 'passed';
  await writeFile(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  if (failure) throw failure;
  console.log('Harmless WKWebView AX probe passed; XTrace first launch and update are not tested.');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
