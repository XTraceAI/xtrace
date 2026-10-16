import { spawn, spawnSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function smokeEnvironment(
  directory,
  inherited = process.env,
  { fixtureSmoke = false } = {},
) {
  const env = { ...inherited, GITHUB_TOKEN: '', GH_TOKEN: '', XTRACE_TELEMETRY: '0' };
  delete env.XTRACE_FIXTURE;
  delete env.XTRACE_PYTHON;
  env.XTRACE_DATA_DIR = join(directory, 'data');
  // The smoke launch indexes an empty synthetic home, never the machine's own history.
  env.XTRACE_NATIVE_HOME = join(directory, 'home');
  if (fixtureSmoke) {
    env.XTRACE_FIXTURE = 'F1';
    env.XTRACE_PR_AUTO_CHECK = 'off';
  }
  return env;
}

/** Test identities are per-run and never use the installed app's identity. */
export function isSmokeIdentifier(value) {
  return /^ai\.xtrace\.app\.test\.[a-f0-9-]+$/.test(value);
}

export function inspectorArguments(pid, prepared) {
  if (!isSmokeIdentifier(prepared.identifier)) throw new Error('Invalid smoke identity');
  return [String(pid), prepared.bundle, prepared.identifier];
}

/** Copy first; all plist and signing writes belong to that disposable copy. */
export async function prepareSmokeBundle(source, directory, { identifier, run = spawnSync } = {}) {
  if (!isSmokeIdentifier(identifier)) throw new Error('Invalid smoke identity');
  const checked = (command, args) => {
    const result = run(command, args, { encoding: 'utf8', stdio: 'pipe', timeout: 120_000 });
    if (result.error || result.signal || result.status !== 0)
      throw new Error('Smoke preparation failed');
    return result.stdout?.trim();
  };
  // A plist copy cannot change Tauri's compiled single-instance identity.
  // The caller must build with this exact test ID before asking for a launch.
  if (
    checked('/usr/libexec/PlistBuddy', [
      '-c',
      'Print :CFBundleIdentifier',
      join(source, 'Contents/Info.plist'),
    ]) !== identifier
  )
    throw new Error('Source was not built with the expected test identity');
  const bundle = join(directory, basename(source));
  if (resolve(bundle) === resolve(source)) throw new Error('Smoke copy must be separate');
  // Dereference trusted build links so executable and signing writes cannot
  // follow a copied link back into the original build.
  await cp(source, bundle, {
    recursive: true,
    dereference: true,
    errorOnExist: true,
    force: false,
  });
  const plist = join(bundle, 'Contents/Info.plist');
  checked('codesign', ['--force', '--deep', '--sign', '-', bundle]);
  checked('codesign', ['--verify', '--deep', '--strict', bundle]);
  // Read after signing; never launch a copy whose identity was not established.
  if (checked('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier', plist]) !== identifier)
    throw new Error('Smoke identity mismatch');
  const executable = checked('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleExecutable', plist]);
  if (!executable || !/^[A-Za-z0-9_. -]+$/.test(executable))
    throw new Error('Invalid smoke executable');
  return { bundle, identifier, executable };
}

export async function launchSmokeBundle(
  source,
  directory,
  { spawnProcess = spawn, fixtureSmoke = false, ...options } = {},
) {
  const prepared = await prepareSmokeBundle(source, directory, options);
  const child = spawnProcess(join(prepared.bundle, 'Contents/MacOS', prepared.executable), [], {
    stdio: 'ignore',
    env: smokeEnvironment(directory, process.env, { fixtureSmoke }),
  });
  return { prepared, child, smokeMode: fixtureSmoke ? 'fixture-f1' : 'native' };
}

/** Unregister only the disposable copy this run owns, never the source app. */
export function unregisterSmokeBundle(prepared, directory, run = spawnSync) {
  if (
    !isSmokeIdentifier(prepared.identifier) ||
    resolve(dirname(prepared.bundle)) !== resolve(directory)
  )
    throw new Error('Cannot unregister an unowned smoke bundle');
  const result = run(
    '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
    ['-u', prepared.bundle],
    { stdio: 'pipe', timeout: 30_000 },
  );
  if (result.error || result.signal || result.status !== 0)
    throw new Error('Could not unregister the disposable smoke bundle');
}

/** A sent signal is not an exit. Keep the copy until this exact child is gone. */
export async function stopSmokeChild(child, { graceMs = 3000, forceMs = 3000 } = {}) {
  const exited = () => child.exitCode != null || child.signalCode != null;
  // Failed spawn: no process was created. An exited child must not be signalled again.
  if (child.pid === undefined || exited()) return;
  await new Promise((resolve, reject) => {
    let graceful;
    let forced;
    const finish = (error) => {
      clearTimeout(graceful);
      clearTimeout(forced);
      child.removeListener('exit', onExit);
      if (error) reject(error);
      else resolve();
    };
    const onExit = () => finish();
    const signal = (name) => {
      try {
        child.kill(name);
      } catch {
        // A failed signal is not proof of exit either. The bounded checks below
        // still require the exit event or the child's recorded exit status.
      }
    };
    child.once('exit', onExit);
    if (exited()) return finish();
    graceful = setTimeout(() => {
      if (exited()) return finish();
      forced = setTimeout(() => {
        if (exited()) finish();
        else
          finish(new Error('Smoke child exit was not confirmed; its temporary copy is retained.'));
      }, forceMs);
      signal('SIGKILL');
    }, graceMs);
    signal('SIGTERM');
  });
}

async function main() {
  const fixtureSmoke = process.argv[4] === '--fixture-smoke';
  if (
    process.platform !== 'darwin' ||
    (process.argv.length !== 4 && !(process.argv.length === 5 && fixtureSmoke))
  )
    throw new Error();
  const identifier = process.argv[3];
  if (!isSmokeIdentifier(identifier)) throw new Error();
  const bundle = resolve(process.argv[2]);
  if (!(await stat(bundle)).isDirectory() || !bundle.endsWith('.app')) throw new Error();
  const temporary = await mkdtemp(join(tmpdir(), 'xtrace-native-smoke-'));
  const directory = fixtureSmoke ? join(temporary, 'staging.noindex') : temporary;
  const script = join(dirname(fileURLToPath(import.meta.url)), 'debug-bundle.swift');
  const inspector = join(temporary, 'inspect');
  let child;
  let prepared;
  try {
    const compile = spawnSync('swiftc', [script, '-o', inspector], {
      stdio: 'pipe',
      timeout: 120_000,
    });
    if (compile.error || compile.status !== 0) throw new Error();
    await mkdir(directory, { recursive: true });
    const env = smokeEnvironment(directory, process.env, { fixtureSmoke });
    await mkdir(env.XTRACE_NATIVE_HOME);
    const launched = await launchSmokeBundle(bundle, directory, { identifier, fixtureSmoke });
    child = launched.child;
    prepared = launched.prepared;
    await new Promise((resolve, reject) => {
      child.once('spawn', resolve);
      child.once('error', reject);
    });
    const inspect = spawnSync(inspector, inspectorArguments(child.pid, launched.prepared), {
      stdio: 'pipe',
      timeout: 35_000,
    });
    if (inspect.error || inspect.status !== 0 || child.exitCode !== null) throw new Error();
    console.log(
      fixtureSmoke
        ? 'Fixture F1 debug app launched and created its native main window (smokeMode: fixture-f1).'
        : 'Debug app bundle launched and created its native main window.',
    );
  } finally {
    // Only terminate the exact child created here; never kill by app name.
    if (child) await stopSmokeChild(child);
    try {
      if (fixtureSmoke && prepared) unregisterSmokeBundle(prepared, directory);
    } finally {
      await rm(temporary, { recursive: true, force: true });
    }
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(() => {
    console.error('Debug bundle smoke failed; native launch evidence is unavailable.');
    process.exitCode = 1;
  });
}
