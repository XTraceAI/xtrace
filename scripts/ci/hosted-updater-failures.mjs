import { spawn, spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  appInventory,
  executionBudget,
  firstLaunchBaselinePaths,
  firstLaunchEnvironment,
  release,
  requireReleaseValues,
  sha256,
  verifyAppInventory,
} from './hosted-release.mjs';
import { previousRelease, verifyProductionFeed } from './hosted-update-checks.mjs';
import { requireFreshBaseline, requireProbeHost } from './hosted-validation.mjs';

// The candidate's public updater archive and signature, from the one set of
// build values in hosted-release.mjs. The lock is this checkout's Cargo.lock
// (0.1.8 product source), which the tests-only SDK helper is built from.
export const fixture = Object.freeze({
  archive: release.archive,
  bytes: release.archiveBytes,
  sha256: release.archiveSha256,
  signatureBytes: release.signatureBytes,
  signatureSha256: release.signatureSha256,
  members: release.inventoryEntries,
  root: basename(release.bundle),
  lockSha256: '6fc4133ae9ee10cdb4e8785d62843de3862b0578781a88055c5cf58e52a7ab7d',
});
// The synthetic old app reports the published earlier version and sits in the
// earlier folder name, as a 0.1.7 install does. The "version" case announces a
// version the signed archive was not signed for.
export const versions = Object.freeze({
  old: previousRelease.version,
  target: release.version,
  mismatch: '0.1.9',
});
const oldBundleName = basename(previousRelease.bundle);
export const limits = Object.freeze({ workMs: 16 * 60_000, cleanupMs: 2 * 60_000 });
const cases = ['valid', 'corrupt', 'version', 'timeout', 'interrupted', 'permission'];
const systemEnvironment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8' };
const securityExecutable =
  '/System/Library/Frameworks/Security.framework/Versions/A/MachServices/SecurityAgent.bundle/Contents/MacOS/SecurityAgent';
const exists = async (path) => {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error.code === 'ENOENT') return false;
    throw error;
  }
};

export function requireFailureHost(input) {
  if (
    input.mode !== 'updater-failures' ||
    input.env.WORKFLOW_REF !== `refs/heads/${input.env.DEFAULT_BRANCH}`
  )
    throw new Error('SDK failure tests require the exact reviewed default branch');
  requireProbeHost({ ...input, mode: 'probe' });
}
export function verifyFixture(archive, signature, lock, pins = fixture) {
  if (
    archive.length !== pins.bytes ||
    sha256(archive) !== pins.sha256 ||
    signature.length !== pins.signatureBytes ||
    sha256(signature) !== pins.signatureSha256 ||
    sha256(lock) !== pins.lockSha256
  )
    throw new Error('Archive, signature or dependency lock differs from pinned inputs');
}
function cancelledInstall(cancellation, pid) {
  return (
    Number.isInteger(pid) &&
    pid > 0 &&
    cancellation?.status === 'passed' &&
    cancellation.cancelPressed === true &&
    cancellation.requester?.pid === pid &&
    Boolean(cancellation.action) &&
    cancellation.after?.length === 0
  );
}
export function acceptResult(name, result, receipt, cancellation, pins = fixture) {
  if (
    !cases.includes(name) ||
    result.status !== 'passed' ||
    result.case !== name ||
    result.downloadTimeoutMs !== 600000 ||
    !Number.isInteger(result.pid) ||
    result.pid <= 0 ||
    receipt.filter((row) => row.path === `/${name}/archive`).length !== 1 ||
    receipt.filter((row) => row.path === `/${name}/latest.json`).length !== 1
  )
    throw new Error(`${name}: missing SDK or real fixture evidence`);
  if (['valid', 'permission'].includes(name)) {
    if (
      !result.verifiedDownload ||
      result.verifiedBytes !== pins.bytes ||
      result.transferred !== pins.bytes ||
      !result.installAttempted
    )
      throw new Error(`${name}: verified download/install not established`);
  } else if (
    result.verifiedDownload ||
    result.installAttempted ||
    !result.error ||
    !result.errorDebug
  ) {
    throw new Error(`${name}: rejection did not preserve download/install boundary`);
  }
  if (
    name === 'timeout' &&
    (result.elapsedMs < 599000 ||
      result.elapsedMs > 630000 ||
      result.transferred <= 0 ||
      result.transferred >= pins.bytes ||
      result.transferFinishedCallback)
  )
    throw new Error('Timeout did not exercise the full production download limit');
  if (
    name === 'interrupted' &&
    (result.transferred <= 0 || result.transferred >= pins.bytes || result.transferFinishedCallback)
  )
    throw new Error('Interrupted response did not establish an incomplete download');
  if (name === 'permission' && !cancelledInstall(cancellation, result.pid))
    throw new Error('Permission failure lacks a real attributed Cancel receipt');
}

export async function serveFixtures(archive, signature) {
  const receipts = [],
    sockets = new Set();
  const server = createServer((request, response) => {
    const match =
      /^\/(valid|corrupt|version|timeout|interrupted|permission)\/(latest\.json|archive)$/.exec(
        request.url || '',
      );
    if (request.method !== 'GET' || !match) {
      response.writeHead(404).end();
      return;
    }
    const [, name, resource] = match;
    const row = {
      path: request.url,
      method: request.method,
      at: Date.now(),
      responseStatus: 200,
      sentBytes: 0,
    };
    receipts.push(row);
    if (resource === 'latest.json') {
      const body = Buffer.from(
        JSON.stringify({
          version: name === 'version' ? versions.mismatch : versions.target,
          platforms: {
            'darwin-aarch64': {
              signature,
              url: `http://127.0.0.1:${server.address().port}/${name}/archive`,
            },
          },
        }),
      );
      row.sentBytes = body.length;
      response
        .writeHead(200, { 'Content-Type': 'application/json', 'Content-Length': body.length })
        .end(body);
      return;
    }
    response.writeHead(200, {
      'Content-Type': 'application/octet-stream',
      'Content-Length': archive.length,
      Connection: 'close',
    });
    if (name === 'timeout' || name === 'interrupted') {
      row.sentBytes = 4096;
      response.write(archive.subarray(0, 4096), () => {
        if (name === 'interrupted') setTimeout(() => response.socket?.end(), 100);
      });
      return; // timeout deliberately leaves a partial body open until the real SDK timer expires.
    }
    const body = Buffer.from(archive);
    if (name === 'corrupt') body[Math.floor(body.length / 2)] ^= 1;
    row.sentBytes = body.length;
    response.end(body);
  });
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  await new Promise((accept, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', accept);
  });
  return {
    port: server.address().port,
    receipts,
    close: async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise((accept, reject) =>
        server.close((error) => (error ? reject(error) : accept())),
      );
    },
  };
}

async function main() {
  const output = resolve('artifacts/hosted-updater-failures');
  await mkdir(output, { recursive: true });
  const report = {
    status: 'failed',
    cases: [],
    commands: [],
    releasedAppLaunched: false,
    limits: {
      shippedUI: false,
      shippedOldAppUsability: false,
      restart: false,
      syntheticOldTarget: true,
    },
    commandBudget: limits,
  };
  const budget = executionBudget(undefined, limits),
    processes = [];
  let owned,
    parent,
    server,
    targetParent,
    observerBinary,
    requesterPath,
    helperName,
    childEnv,
    failure;
  let permissionStarted = false;
  // BEGIN CHECKED COMMAND: regression controls use the real refusal behavior.
  const command = (name, executable, args, timeout = 30000, env = systemEnvironment) => {
    const result = spawnSync(executable, args, {
      encoding: 'utf8',
      env,
      timeout: budget.remaining(timeout),
      killSignal: 'SIGKILL',
      maxBuffer: 4 * 1024 * 1024,
    });
    report.commands.push({
      name,
      executable,
      args,
      status: result.status,
      signal: result.signal,
      error: result.error?.message,
      stdout: result.stdout,
      stderr: result.stderr,
    });
    if (result.error || result.signal || result.status !== 0)
      throw new Error(
        `${name} failed: ${result.error?.code || result.signal || result.status}; ${result.stderr || ''}`,
      );
    return result.stdout.trim();
  };
  // END CHECKED COMMAND
  // BEGIN OWNED REGISTRY CLEANUP: only called after process/prompt/evidence gates.
  const cleanupBundles = async () => {
    if (
      (await realpath(owned)) !== owned ||
      (await realpath(parent)) !== parent ||
      dirname(owned) !== parent ||
      !owned.endsWith('/staging.noindex')
    )
      throw new Error('Cleanup paths are not the canonical owned staging directories');
    const paths = [
      ...(helperName ? [join(owned, `${helperName}.app`)] : []),
      ...cases.map((name) => join(owned, `${name}-target`, oldBundleName)),
    ];
    const register =
      '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister';
    const registry = {
      checkedRoot: owned,
      checkedPaths: paths,
      unregister: [],
      absenceConfirmed: false,
    };
    report.registryCleanup = registry;
    let unregisterError;
    for (const bundle of paths) {
      if (!(await exists(bundle))) continue;
      if (!(await lstat(bundle)).isDirectory() || (await realpath(bundle)) !== bundle)
        throw new Error('Owned bundle changed into a noncanonical path');
      const result = spawnSync(register, ['-u', bundle], {
        encoding: 'utf8',
        env: systemEnvironment,
        timeout: budget.remaining(30000),
        killSignal: 'SIGKILL',
        maxBuffer: 4 * 1024 * 1024,
      });
      const receipt = {
        executable: register,
        args: ['-u', bundle],
        status: result.status,
        signal: result.signal,
        error: result.error?.message,
        stdout: result.stdout,
        stderr: result.stderr,
      };
      registry.unregister.push(receipt);
      const missing =
        result.status === 1 &&
        !result.error &&
        !result.signal &&
        result.stdout === '' &&
        result.stderr.trimEnd() === `failed to scan ${bundle}: -10814\n from spotlight`;
      receipt.reportedMissing = missing;
      if (result.error || result.signal || (result.status !== 0 && !missing))
        unregisterError ||= new Error('An owned unregister command failed unexpectedly');
    }
    // One full read-only dump. Its larger bound is dedicated to this command;
    // the raw bytes stay in one artifact rather than duplicating a large JSON field.
    const dump = spawnSync(register, ['-dump'], {
      env: systemEnvironment,
      timeout: budget.remaining(30000),
      killSignal: 'SIGKILL',
      maxBuffer: 64 * 1024 * 1024,
    });
    const raw = dump.stdout || Buffer.alloc(0);
    const dumpText = raw.toString('utf8');
    const pathRows = dumpText.match(/^path:.*$/gm) || [];
    const stderr = dump.stderr?.toString('utf8') || '';
    const log = join(output, 'launch-services-final.log');
    registry.dump = {
      executable: register,
      args: ['-dump'],
      status: dump.status,
      signal: dump.signal,
      error: dump.error?.message,
      stderr,
      log,
      bytes: raw.length,
      sha256: sha256(raw),
      usable: false,
    };
    await writeFile(log, raw);
    if (
      dump.error ||
      dump.signal ||
      dump.status !== 0 ||
      stderr ||
      !raw.equals(Buffer.from(dumpText, 'utf8')) ||
      !/^Status:\s+\S.+$/m.test(dumpText) ||
      // Hosted macOS 14 rows (run 39531328355): "bundle id:   SSMenuAgent (0x17a0)",
      // and the root volume as "path:   / (0x4)". Every native row ends in its address.
      !/^bundle id:\s+\S.*\(0x[0-9a-f]+\)$/m.test(dumpText) ||
      !pathRows.length ||
      pathRows.some((row) => !/^path:\s+\/.*\(0x[0-9a-f]+\)$/.test(row)) ||
      !dumpText.endsWith('\n')
    )
      throw new Error('Launch Services dump is incomplete or unrecognized; staging retained');
    registry.dump.usable = true;
    // Native path rows carry an address suffix. Search raw paths; do not compare
    // a parsed whole value such as "/App.app (0x1234)" with "/App.app".
    const rootPattern = owned.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    registry.rootPresent = new RegExp(`${rootPattern}(?=[/\\s)]|$)`, 'm').test(dumpText);
    registry.pathsPresent = paths.filter((bundle) => dumpText.includes(bundle));
    if (registry.rootPresent || registry.pathsPresent.length)
      throw new Error('An owned path remains in Launch Services; staging retained');
    registry.absenceConfirmed = true;
    if (unregisterError) throw unregisterError;
    await rm(parent, { recursive: true });
    registry.filesystemAbsent = !(await exists(parent));
    if (!registry.filesystemAbsent) throw new Error('Owned staging remains after removal');
  };
  // END OWNED REGISTRY CLEANUP
  // BEGIN OWNED CHILD: safe regression controls exercise this exact private closure.
  const launch = (name, executable, args, timeout, env) => {
    const timeoutMs = budget.remaining(timeout);
    const child = spawn(executable, args, {
      env,
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const entry = {
      name,
      executable,
      pid: child.pid,
      exited: false,
      completed: false,
      timedOut: false,
      stdout: '',
      stderr: '',
    };
    processes.push(entry);
    const promise = new Promise((accept, reject) => {
      let timer;
      child.once('error', (error) => {
        clearTimeout(timer);
        Object.assign(entry, { exited: true, error: error.message });
        reject(error);
      });
      child.once('close', async (code, signal) => {
        clearTimeout(timer);
        Object.assign(entry, { exited: true, code, signal });
        try {
          await writeFile(join(output, `${name}.log`), entry.stdout + '\n' + entry.stderr);
          if (entry.timedOut || signal) throw new Error(`${name} watchdog/signal: ${signal}`);
          const parsed = JSON.parse(entry.stdout);
          entry.process = parsed.process;
          entry.result = parsed;
          await writeFile(join(output, `${name}.json`), JSON.stringify(parsed, null, 2) + '\n');
          accept({ code, parsed });
        } catch (error) {
          entry.error = error.message;
          reject(error);
        }
      });
      const stop = () => {
        if (!entry.exited && child.pid) {
          try {
            process.kill(-child.pid, 'SIGKILL');
          } catch (error) {
            if (error.code !== 'ESRCH') entry.stopError = error.message;
          }
        }
      };
      timer = setTimeout(() => {
        entry.timedOut = true;
        stop();
      }, timeoutMs);
      for (const stream of ['stdout', 'stderr'])
        child[stream].on('data', (chunk) => {
          entry[stream] += chunk.toString();
          if (entry[stream].length > 4 * 1024 * 1024 && !entry.exited) {
            entry.timedOut = true;
            stop();
          }
        });
    });
    // Failure may arrive before the caller reaches await (observer runs beside SDK).
    promise
      .finally(() => {
        entry.completed = true;
      })
      .catch(() => {});
    return { entry, promise };
  };
  // END OWNED CHILD
  const saveCase = (row) =>
    writeFile(join(output, `${row.name}-case.json`), JSON.stringify(row, null, 2) + '\n');
  // BEGIN FINAL CASE: preserve failed observations at the artifact upload root.
  const preserveCase = async (row, processExitConfirmed) => {
    row.finalObservation = { processExitConfirmed };
    for (const [key, path] of [
      ['target', row.config.target],
      ['data', childEnv.XTRACE_DATA_DIR],
    ]) {
      try {
        row.finalObservation[key] = await appInventory(path);
      } catch (error) {
        row.finalObservation[`${key}Error`] = error.message;
      }
    }
    try {
      row.installAttemptReceipt = await readFile(row.config.installReceipt, 'utf8');
    } catch (error) {
      row.installAttemptReceiptError = error.message;
    }
    row.processes = processes
      .filter((entry) => entry.name === row.name || entry.name.startsWith(`${row.name}-`))
      .map(
        ({
          name,
          pid,
          process: identity,
          exited,
          completed,
          timedOut,
          code,
          signal,
          error,
          stopError,
          result,
        }) => ({
          name,
          pid,
          identity,
          exited,
          completed,
          timedOut,
          code,
          signal,
          error,
          stopError,
          result,
        }),
      );
    await saveCase(row);
  };
  // END FINAL CASE
  const probe = async (name, config, targetExecutable) => {
    const actualID = command('old-probe-id', '/usr/libexec/PlistBuddy', [
      '-c',
      'Print :CFBundleIdentifier',
      join(config.target, 'Contents/Info.plist'),
    ]);
    if (actualID !== config.identifier || !(await lstat(targetExecutable)).isFile())
      throw new Error('Old probe bundle/executable identity mismatch');
    const path = join(owned, `${name}.config.json`);
    await writeFile(path, JSON.stringify({ ...config, case: 'probe' }));
    const { code, parsed } = await launch(name, targetExecutable, [path], 10000, childEnv).promise;
    if (code !== 0 || parsed.oldTargetUsable !== true || parsed.identifier !== config.identifier)
      throw new Error('Owned old fixture did not remain usable');
    return parsed;
  };
  try {
    if (process.argv.length !== 2) throw new Error('SDK failure runner accepts no overrides');
    const head = command('head', '/usr/bin/git', ['rev-parse', 'HEAD']);
    const osVersion =
      process.platform === 'darwin' ? command('os', '/usr/bin/sw_vers', ['-productVersion']) : '';
    requireFailureHost({
      mode: process.env.DELIVERY_MODE,
      platform: process.platform,
      arch: process.arch,
      osVersion,
      head,
      env: process.env,
    });
    requireReleaseValues();
    Object.assign(report, {
      head,
      osVersion,
      architecture: process.arch,
      imageOS: process.env.ImageOS,
      imageVersion: process.env.ImageVersion,
      fixture,
    });
    const baseline = firstLaunchBaselinePaths(homedir());
    requireFreshBaseline(
      (
        await Promise.all(baseline.map(async (path) => ((await exists(path)) ? path : null)))
      ).filter(Boolean),
      command('baseline', '/bin/ps', ['-axo', 'pid=,comm=']),
    );
    const uid = command('uid', '/usr/bin/id', ['-u']);
    const consoleUid = command('console', '/usr/bin/stat', ['-f', '%u', '/dev/console']);
    if (uid === '0' || uid !== consoleUid)
      throw new Error('Runner must own the active non-root console session');
    report.console = { uid, consoleUid, groups: command('groups', '/usr/bin/id', ['-G']) };
    parent = await mkdtemp(join(output, 'run-'));
    owned = join(await realpath(parent), 'staging.noindex');
    if (/['"\\\r\n]/.test(owned))
      throw new Error('Owned path is unsafe for SDK AppleScript quoting');
    await mkdir(owned);
    childEnv = {
      ...firstLaunchEnvironment(owned),
      HOME: join(owned, 'home'),
      GITHUB_ACTIONS: 'true',
      RUNNER_ENVIRONMENT: 'github-hosted',
      GITHUB_EVENT_NAME: 'workflow_dispatch',
      RUNNER_OS: 'macOS',
      RUNNER_ARCH: 'ARM64',
    };
    for (const key of [
      'TMPDIR',
      'XTRACE_DATA_DIR',
      'XTRACE_NATIVE_HOME',
      'CLAUDE_CONFIG_DIR',
      'GH_CONFIG_DIR',
    ])
      await mkdir(childEnv[key]);
    helperName = `XTraceUpdaterTest-${randomUUID().replaceAll('-', '')}`;
    const identifier = `ai.xtrace.app.test.updater-failures.${helperName.slice(18)}`;
    const sourceBinary = resolve('target/debug/examples/updater_failures');
    if (!(await lstat(sourceBinary)).isFile()) throw new Error('Missing reviewed built helper');
    const driverBundle = join(owned, `${helperName}.app`);
    requesterPath = join(driverBundle, 'Contents/MacOS', helperName);
    await mkdir(dirname(requesterPath), { recursive: true });
    await copyFile(sourceBinary, requesterPath);
    await chmod(requesterPath, 0o755);
    const plist = `<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${identifier}</string><key>CFBundleName</key><string>${helperName}</string><key>CFBundleDisplayName</key><string>${helperName}</string><key>CFBundleExecutable</key><string>${helperName}</string><key>CFBundleVersion</key><string>${versions.old}</string><key>CFBundleShortVersionString</key><string>${versions.old}</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>`;
    await writeFile(join(driverBundle, 'Contents/Info.plist'), plist);
    command(
      'sign-helper',
      '/usr/bin/codesign',
      ['--force', '--sign', '-', driverBundle],
      30000,
      childEnv,
    );
    command(
      'verify-helper',
      '/usr/bin/codesign',
      ['--verify', '--strict', driverBundle],
      30000,
      childEnv,
    );
    const actualID = command('helper-id', '/usr/libexec/PlistBuddy', [
      '-c',
      'Print :CFBundleIdentifier',
      join(driverBundle, 'Contents/Info.plist'),
    ]);
    if (actualID !== identifier) throw new Error('Built helper test identifier mismatch');
    report.helper = {
      identifier,
      name: helperName,
      executable: requesterPath,
      sha256: sha256(await readFile(requesterPath)),
    };
    observerBinary = resolve('target/debug/hosted-updater-cancel');
    if (
      !(await lstat(observerBinary)).isFile() ||
      (await realpath(observerBinary)) !== observerBinary
    )
      throw new Error('Expected regular canonical prebuilt observer');
    report.observer = {
      executable: observerBinary,
      sha256: sha256(await readFile(observerBinary)),
    };
    // BEGIN APPLE SIGNATURE CHECK
    command('security-agent-apple-signature', '/usr/bin/codesign', [
      '--verify',
      '--strict',
      '-R',
      '=anchor apple',
      securityExecutable,
    ]);
    // END APPLE SIGNATURE CHECK
    const fetch = (name, destination, bytes) =>
      command(
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
          String(bytes),
          '--output',
          destination,
          `https://github.com/XTraceAI/xtrace-desktop/releases/download/${release.tag}/${fixture.archive}${name === 'signature' ? '.sig' : ''}`,
        ],
        95000,
      );
    const archivePath = join(owned, fixture.archive),
      signaturePath = archivePath + '.sig';
    fetch('archive', archivePath, fixture.bytes);
    fetch('signature', signaturePath, fixture.signatureBytes);
    const archive = await readFile(archivePath),
      signatureBytes = await readFile(signaturePath);
    const lock = await readFile('Cargo.lock');
    verifyFixture(archive, signatureBytes, lock);
    report.lockSha256 = sha256(lock);
    const signature = signatureBytes.toString().trim();
    // The committed feed must be the exact pinned candidate feed.
    const feedBytes = await readFile(
      new URL('./fixtures/hosted-update-latest.json', import.meta.url),
    );
    verifyProductionFeed(feedBytes);
    const feed = JSON.parse(feedBytes);
    if (feed.platforms['darwin-aarch64'].signature !== signature)
      throw new Error('Pinned signature differs from reviewed feed');
    const overlay = JSON.parse(await readFile('apps/desktop/src-tauri/tauri.updater.release.json'));
    // Inspect original archive before it reaches the SDK's actual extraction.
    report.archiveMembers = JSON.parse(
      command('archive-members', '/usr/bin/python3', [
        '-I',
        '-c',
        'import json,sys,tarfile,pathlib\nwith tarfile.open(sys.argv[1],"r:gz") as t:\n m=t.getmembers()\n assert len(m)==int(sys.argv[2]) and sum(x.size for x in m)<100_000_000\n names=set()\n for x in m:\n  p=pathlib.PurePosixPath(x.name)\n  assert not p.is_absolute() and ".." not in p.parts and p.parts[0]==sys.argv[3]\n  assert x.isfile() or x.isdir()\n  assert x.name not in names\n  names.add(x.name)\n print(json.dumps([{"path":x.name,"size":x.size,"mode":x.mode,"directory":x.isdir()} for x in m]))',
        archivePath,
        String(fixture.members),
        fixture.root,
      ]),
    );
    server = await serveFixtures(archive, signature);
    // These immutable owned routes produce only 200/404, never redirects. The
    // SDK retains its standard redirect policy; no redirect response is supplied.
    report.sdkTransport = {
      endpoint: 'owned IPv4 loopback',
      proxies: false,
      redirectPolicy: 'SDK default; owned fixture emits no redirects',
    };
    const sentinel = `Owned updater failure sentinel ${helperName}\n`;
    await writeFile(join(childEnv.XTRACE_DATA_DIR, 'sentinel.txt'), sentinel, { flag: 'wx' });
    const dataBefore = await appInventory(childEnv.XTRACE_DATA_DIR);
    for (const name of cases) {
      targetParent = join(owned, `${name}-target`);
      const target = join(targetParent, oldBundleName);
      const targetExecutable = join(target, 'Contents/MacOS', helperName);
      await mkdir(dirname(targetExecutable), { recursive: true });
      await copyFile(requesterPath, targetExecutable);
      await chmod(targetExecutable, 0o755);
      await writeFile(join(target, 'Contents/Info.plist'), plist);
      const installReceipt = join(
        owned,
        name === 'permission' ? 'permission-install.json' : `${name}-install.json`,
      );
      await writeFile(installReceipt, '', { flag: 'wx' });
      const config = {
        root: owned,
        target,
        targetExecutable,
        identifier,
        case: name,
        endpoint: `http://127.0.0.1:${server.port}/${name}/latest.json`,
        pubkey: overlay.plugins.updater.pubkey,
        signature,
        sentinel,
        installReceipt,
        archiveBytes: fixture.bytes,
        versions,
      };
      const configPath = join(owned, `${name}.config.json`);
      await writeFile(configPath, JSON.stringify(config));
      const before = await appInventory(target);
      const row = { name, config, before, dataBefore, status: 'running' };
      report.cases.push(row);
      await saveCase(row);
      row.oldBefore = await probe(`${name}-old-before`, config, targetExecutable);
      await saveCase(row);
      let observer, cancellation;
      if (name === 'permission') {
        await writeFile(join(targetParent, 'rename-probe'), 'owned');
        await chmod(targetParent, 0o555);
        let denial;
        try {
          await rename(join(targetParent, 'rename-probe'), join(owned, 'rename-probe-moved'));
        } catch (error) {
          denial = error.code;
        }
        if (denial !== 'EACCES' && denial !== 'EPERM')
          throw new Error('Target rename was not permission-denied');
        report.permissionTarget = {
          uid,
          mode: (await lstat(targetParent)).mode & 0o7777,
          renameProbe: denial,
        };
        if ((await lstat(targetParent)).dev !== (await lstat(childEnv.TMPDIR)).dev)
          throw new Error('Target and SDK temporary storage must share a filesystem');
        observer = launch(
          'permission-cancel',
          observerBinary,
          [owned, helperName, requesterPath, 'observe'],
          45000,
          childEnv,
        );
        const readinessDeadline = Date.now() + 5000;
        while (
          !(await exists(join(owned, 'observer-ready'))) &&
          !observer.entry.exited &&
          Date.now() < readinessDeadline
        )
          await new Promise((accept) => setTimeout(accept, 50));
        if (!(await exists(join(owned, 'observer-ready')))) {
          await observer.promise;
          throw new Error('Cancel observer did not establish a fresh baseline');
        }
      }
      const sdk = launch(
        name,
        requesterPath,
        [configPath],
        name === 'timeout' ? 640000 : name === 'permission' ? 45000 : 60000,
        childEnv,
      );
      if (name === 'permission') {
        permissionStarted = true;
        await writeFile(
          join(owned, 'permission-attempt.json'),
          JSON.stringify({ pid: sdk.entry.pid }),
          { flag: 'wx' },
        );
      }
      const sdkOutcome = await sdk.promise;
      if (observer) cancellation = (await observer.promise).parsed;
      const result = sdkOutcome.parsed;
      if (result.pid !== sdk.entry.pid) throw new Error(`${name}: SDK result PID mismatch`);
      const after = await appInventory(target),
        dataAfter = await appInventory(childEnv.XTRACE_DATA_DIR);
      Object.assign(row, { result, after, dataAfter, cancellation });
      await writeFile(
        join(output, 'fixture-receipts.json'),
        JSON.stringify(server.receipts, null, 2) + '\n',
      );
      if (name === 'valid') {
        row.installed = await verifyAppInventory(target);
        row.installedIdentifier = command('installed-id', '/usr/libexec/PlistBuddy', [
          '-c',
          'Print :CFBundleIdentifier',
          join(target, 'Contents/Info.plist'),
        ]);
        if (row.installedIdentifier !== release.identifier)
          throw new Error('Installed signed archive identity mismatch');
      } else {
        if (JSON.stringify(before) !== JSON.stringify(after))
          throw new Error(`${name}: old target changed`);
        row.oldAfter = await probe(`${name}-old-after`, config, targetExecutable);
      }
      if (JSON.stringify(dataBefore) !== JSON.stringify(dataAfter))
        throw new Error(`${name}: synthetic data changed`);
      acceptResult(name, result, server.receipts, cancellation);
      if (sdkOutcome.code !== 0) throw new Error(`${name}: SDK process did not exit successfully`);
      row.status = 'passed';
      if (name === 'permission') await chmod(targetParent, 0o755);
      await saveCase(row);
      await writeFile(join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
    }
  } catch (error) {
    failure = error;
    const row = report.cases.at(-1);
    if (row?.status === 'running') {
      row.status = 'failed';
      row.error = error.message;
    }
  }
  budget.beginCleanup();
  try {
    let exitFailure;
    try {
      for (const entry of processes) {
        if (!entry.exited && entry.pid) {
          try {
            process.kill(-entry.pid, 'SIGKILL');
          } catch (error) {
            if (error.code !== 'ESRCH') {
              entry.stopError = error.message;
              throw error;
            }
          }
        }
        const deadline = Date.now() + 5000;
        while (!entry.completed && Date.now() < deadline)
          await new Promise((accept) => setTimeout(accept, 50));
        if (!entry.exited || !entry.completed)
          throw new Error('Owned child exit remains unknown; staging retained');
      }
      const remaining = command('owned-groups', '/bin/ps', ['-axo', 'pid=,pgid=,comm=']);
      if (
        remaining
          .split('\n')
          .some((line) =>
            processes.some((entry) => Number(line.trim().split(/\s+/)[1]) === entry.pid),
          )
      )
        throw new Error('An owned process group remains; staging retained');
    } catch (error) {
      exitFailure = error;
    }
    for (const row of report.cases) await preserveCase(row, !exitFailure);
    if (exitFailure) throw exitFailure;
    if (
      report.cases.some(
        (row) =>
          row.finalObservation.targetError ||
          row.finalObservation.dataError ||
          row.installAttemptReceiptError,
      )
    )
      throw new Error('A final case observation could not be saved completely; staging retained');
    if (permissionStarted) {
      const cancel = processes.find((entry) => entry.name === 'permission-cancel')?.result;
      const requester = processes.find((entry) => entry.name === 'permission');
      if (!cancelledInstall(cancel, requester?.pid)) {
        report.authorizationCleanup = 'unknown: no attributed cancellation and disappearance';
        throw new Error('Authorization cleanup unknown; staging retained');
      }
      const absent = await launch(
        'permission-after-stop',
        observerBinary,
        [owned, helperName, requesterPath, 'absent'],
        10000,
        childEnv,
      ).promise;
      if (absent.code !== 0 || absent.parsed.status !== 'passed')
        throw new Error('Authorization window cleanup unconfirmed; staging retained');
      report.authorizationCleanup = 'attributed cancellation and post-stop absence confirmed';
    }
    if (server) {
      await server.close();
      report.listenerClosed = true;
    }
    if (targetParent && (await exists(targetParent))) await chmod(targetParent, 0o755);
    if (owned) await cleanupBundles();
    else if (parent) throw new Error('Owned staging identity was not established; parent retained');
    report.cleanupConfirmed = true;
  } catch (error) {
    report.cleanupError = error.message;
    failure ||= error;
  }
  // Always close the owned listener, including when process/prompt uncertainty retains staging.
  if (server && !report.listenerClosed) {
    await server.close();
    report.listenerClosed = true;
  }
  report.processes = processes.map(
    ({
      name,
      executable,
      pid,
      process: identity,
      exited,
      completed,
      timedOut,
      code,
      signal,
      error,
      stopError,
    }) => ({
      name,
      executable,
      pid,
      identity,
      exited,
      completed,
      timedOut,
      code,
      signal,
      error,
      stopError,
    }),
  );
  if (server)
    await writeFile(
      join(output, 'fixture-receipts.json'),
      JSON.stringify(server.receipts, null, 2) + '\n',
    );
  if (failure) report.error = failure.message;
  else report.status = 'passed';
  await writeFile(join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  if (failure) throw failure;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
