import { lstat, realpath, writeFile, readFile, chmod } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { appInventory, sha256 } from './hosted-release.mjs';
const exists = async (path) => {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error.code === 'ENOENT') return false;
    throw error;
  }
};
export function createCheckedCommand({ report, systemEnvironment, budget, spawnSync }) {
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

  return command;
}

export function createRegistryCleanup({
  owned,
  parent,
  helperName,
  cases,
  oldBundleName,
  output,
  report,
  budget,
  systemEnvironment,
  spawnSync,
  writeFile,
  rm,
}) {
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

  return cleanupBundles;
}

export function createOwnedChild({ budget, processes, output, spawn }) {
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

  return launch;
}

export function createFinalCase({ processes, childEnv, saveCase }) {
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

  return preserveCase;
}

export function verifyAppleSignature(command, securityExecutable) {
  command('security-agent-apple-signature', '/usr/bin/codesign', [
    '--verify',
    '--strict',
    '-R',
    '=anchor apple',
    securityExecutable,
  ]);
}

export async function finishFailureCleanup({
  report,
  failure,
  processes,
  command,
  preserveCase,
  permissionStarted,
  cancelledInstall,
  server,
  targetParent,
  owned,
  parent,
  cleanupBundles,
  launch,
  observerBinary,
  helperName,
  requesterPath,
  childEnv,
}) {
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
  return { report, failure };
}
