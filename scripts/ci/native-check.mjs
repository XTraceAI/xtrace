import { spawnSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { installAbout, installSyft } from '../supply-chain/install-tools.mjs';

export async function checkNative({
  base,
  release = false,
  fixtureSmoke = false,
  repo = process.cwd(),
  platform = process.platform,
  run = spawnSync,
  install = async () => [await installSyft(), await installAbout()],
} = {}) {
  if (release && fixtureSmoke)
    throw new Error('--fixture-smoke cannot be combined with --release.');
  if (platform !== 'darwin' || !/^[a-f0-9]{40}$/.test(base ?? ''))
    throw new Error('Native checks require macOS and --base with a full reviewed commit SHA.');
  const env = { ...process.env, GH_TOKEN: '', GITHUB_TOKEN: '' };
  const execute = (command, args, options = {}) => {
    const result = run(command, args, { cwd: repo, env, stdio: 'inherit', ...options });
    if (result.error || result.signal || result.status !== 0)
      throw new Error(`Native validation stopped: ${command} failed.`);
    return result.stdout;
  };
  const git = (...args) => execute('git', args, { stdio: 'pipe', encoding: 'utf8' }).trim();
  const requireClean = () => {
    if (git('status', '--porcelain'))
      throw new Error('Native validation requires a clean checkout, including untracked files.');
  };
  requireClean();
  const head = git('rev-parse', 'HEAD');
  git('cat-file', '-e', `${base}^{commit}`);
  // Validate the combined result; a stale feature branch is not merge evidence.
  git('merge-base', '--is-ancestor', base, head);
  const version = execute('sw_vers', ['-productVersion'], {
    stdio: 'pipe',
    encoding: 'utf8',
  }).trim();
  const trusted = await mkdtemp(join(tmpdir(), 'xtrace-native-base-'));
  try {
    const archive = execute('git', ['archive', base], {
      stdio: 'pipe',
      maxBuffer: 64 * 1024 * 1024,
    });
    execute('tar', ['-xf', '-', '-C', trusted], { input: archive, stdio: 'pipe' });
    env.CI_TRUSTED_ROOT = trusted;
    execute('pnpm', ['install', '--frozen-lockfile', '--ignore-scripts']);
    execute('cargo', ['fmt', '--all', '--', '--check']);
    execute('cargo', [
      'clippy',
      '--workspace',
      '--all-targets',
      '--all-features',
      '--locked',
      '--',
      '-D',
      'warnings',
    ]);
    execute('cargo', ['test', '--workspace', '--all-features', '--locked']);
    for (const hook of ['plugin-conformance', 'dto'])
      execute('node', ['scripts/ci/run-hook.mjs', hook]);
    env.PATH = [...(await install()), env.PATH ?? ''].join(delimiter);
    execute('pnpm', ['sbom']);
    execute('pnpm', ['notices:check']);
    execute('node', ['scripts/security/scan-secrets.mjs', '--content', 'artifacts/sbom.cdx.json']);
    if (fixtureSmoke) {
      // Compile the test identity into Tauri's singleton too. Copy the cache,
      // then build normal code before building and launching only fixed F1.
      const identifier = `ai.xtrace.app.test.${randomUUID()}`;
      const testTarget = join(trusted, 'smoke-target.noindex');
      execute('cp', ['-c', '-R', join(repo, 'target'), testTarget]);
      env.CARGO_TARGET_DIR = testTarget;
      const build = [
        'tauri',
        'build',
        '--debug',
        '--bundles',
        'app',
        '--config',
        JSON.stringify({ identifier }),
      ];
      execute('pnpm', build);
      execute('pnpm', [...build, '--features', 'fixtures']);
      execute('node', [
        'scripts/ci/debug-bundle.mjs',
        join(testTarget, 'debug/bundle/macos/XTrace Desktop.app'),
        identifier,
        '--fixture-smoke',
      ]);
    } else {
      execute('pnpm', ['tauri', 'build', '--debug', '--bundles', 'app']);
      execute('node', [
        'scripts/ci/debug-bundle.mjs',
        'target/debug/bundle/macos/XTrace Desktop.app',
      ]);
    }
    if (release) {
      execute('pnpm', ['tauri', 'build', '--bundles', 'app']);
      execute('node', [
        'scripts/ci/debug-bundle.mjs',
        'target/release/bundle/macos/XTrace Desktop.app',
      ]);
    }
    if (git('rev-parse', 'HEAD') !== head)
      throw new Error('Source changed during native validation.');
    requireClean();
    return {
      head,
      base,
      macOS: version,
      release,
      ...(fixtureSmoke ? { smokeMode: 'fixture-f1' } : {}),
    };
  } finally {
    await rm(trusted, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const base = args[0] === '--base' ? args[1] : undefined;
  const flags = args.slice(2);
  if (
    !base ||
    new Set(flags).size !== flags.length ||
    flags.some((flag) => !['--release', '--fixture-smoke'].includes(flag))
  ) {
    console.error('Usage: pnpm check:native --base FULL_COMMIT_SHA [--release | --fixture-smoke]');
    process.exitCode = 1;
  } else {
    checkNative({
      base,
      release: flags.includes('--release'),
      fixtureSmoke: flags.includes('--fixture-smoke'),
    })
      .then((result) => console.log('Native validation passed: ' + JSON.stringify(result)))
      .catch((error) => {
        console.error(error.message);
        process.exitCode = 1;
      });
  }
}
