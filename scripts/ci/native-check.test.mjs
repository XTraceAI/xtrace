import assert from 'node:assert/strict';
import { access } from 'node:fs/promises';
import test from 'node:test';
import { checkNative } from './native-check.mjs';

const base = 'a'.repeat(40);
const head = 'b'.repeat(40);
function fixture({ fail, dirty = false, changed = false, notAncestor = false } = {}) {
  const calls = [];
  const invocations = [];
  let identities = 0;
  let trusted;
  const run = (command, args, options) => {
    calls.push([command, ...args].join(' '));
    invocations.push({ command, args: [...args], env: { ...options.env } });
    assert.equal(options.env.GH_TOKEN, '');
    assert.equal(options.env.GITHUB_TOKEN, '');
    if (options.env.CI_TRUSTED_ROOT) trusted = options.env.CI_TRUSTED_ROOT;
    let stdout = '';
    if (command === 'git' && args[0] === 'status') stdout = dirty ? ' M source.rs' : '';
    if (command === 'git' && args[0] === 'rev-parse')
      stdout = changed && identities++ > 0 ? base : head;
    if (command === 'sw_vers') stdout = '26.5.2';
    const failed =
      (typeof fail === 'function' ? fail(command, args) : calls.at(-1) === fail) ||
      (notAncestor && args[0] === 'merge-base');
    return { status: failed ? 1 : 0, stdout };
  };
  return { calls, invocations, run, trusted: () => trusted, install: async () => [] };
}

test('native validation stops before work on an unsupported platform, dirty checkout or stale base', async () => {
  for (const options of [{ platform: 'linux' }, { base: 'main' }]) {
    await assert.rejects(
      checkNative({ base, platform: 'darwin', run: () => assert.fail(), ...options }),
    );
  }
  for (const options of [{ dirty: true }, { notAncestor: true }]) {
    const f = fixture(options);
    await assert.rejects(checkNative({ base, platform: 'darwin', ...f }));
    assert.ok(!f.calls.some((x) => x.startsWith('pnpm')));
  }
});

test('native failures stop packaging and clean the temporary hook baseline', async () => {
  for (const fail of [
    'cargo test --workspace --all-features --locked',
    'node scripts/ci/run-hook.mjs dto',
    'pnpm notices:check',
  ]) {
    const f = fixture({ fail });
    await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /stopped/);
    assert.ok(!f.calls.some((x) => x.includes('tauri build')));
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('native evidence binds source, base and actual OS; release mode also launches the production build', async () => {
  for (const release of [false, true]) {
    const f = fixture();
    assert.deepEqual(await checkNative({ base, release, platform: 'darwin', ...f }), {
      head,
      base,
      macOS: '26.5.2',
      release,
    });
    assert.ok(f.calls.includes('node scripts/ci/run-hook.mjs plugin-conformance'));
    assert.ok(f.calls.includes('node scripts/ci/run-hook.mjs dto'));
    assert.ok(
      f.calls.includes('node scripts/security/scan-secrets.mjs --content artifacts/sbom.cdx.json'),
    );
    assert.ok(f.calls.includes('pnpm tauri build --debug --bundles app'));
    assert.ok(
      f.calls.includes(
        'node scripts/ci/debug-bundle.mjs target/debug/bundle/macos/XTrace Desktop.app',
      ),
    );
    assert.ok(
      f.calls.every(
        (call) =>
          !call.includes('--fixture-smoke') &&
          !call.includes('--features') &&
          !call.startsWith('cp '),
      ),
    );
    assert.equal(f.calls.includes('pnpm tauri build --bundles app'), release);
    assert.equal(
      f.calls.includes(
        'node scripts/ci/debug-bundle.mjs target/release/bundle/macos/XTrace Desktop.app',
      ),
      release,
    );
    assert.ok(f.calls.some((x) => x.includes('target/debug/bundle/macos/')));
    assert.equal(
      f.calls.some((x) => x.includes('target/release/bundle/macos/')),
      release,
    );
    await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
  }
});

test('fixture smoke compiles normal code first, then builds and launches only fixed fixtures', async () => {
  const f = fixture();
  assert.deepEqual(await checkNative({ base, fixtureSmoke: true, platform: 'darwin', ...f }), {
    head,
    base,
    macOS: '26.5.2',
    release: false,
    smokeMode: 'fixture-f1',
  });
  const builds = f.invocations.filter(
    (call) => call.command === 'pnpm' && call.args[0] === 'tauri',
  );
  assert.equal(builds.length, 2);
  assert.ok(!builds[0].args.includes('--features'));
  assert.deepEqual(builds[1].args, [...builds[0].args, '--features', 'fixtures']);
  assert.ok(builds.every((call) => call.args.includes('--debug')));
  assert.ok(
    builds.every((call) => call.env.CARGO_TARGET_DIR === f.trusted() + '/smoke-target.noindex'),
  );
  const launches = f.invocations.filter(
    (call) => call.command === 'node' && call.args[0] === 'scripts/ci/debug-bundle.mjs',
  );
  assert.equal(launches.length, 1);
  assert.ok(f.invocations.indexOf(launches[0]) > f.invocations.indexOf(builds[1]));
  assert.equal(launches[0].args.at(-1), '--fixture-smoke');
  assert.ok(launches[0].args[1].includes('/smoke-target.noindex/debug/'));
  const config = JSON.parse(builds[0].args.at(-1));
  assert.equal(launches[0].args[2], config.identifier);
  await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
});

test('fixture smoke and release are incompatible before any command or build', async () => {
  await assert.rejects(
    checkNative({
      base,
      platform: 'darwin',
      release: true,
      fixtureSmoke: true,
      run: () => assert.fail('No command may run'),
      install: () => assert.fail('No tool may install'),
    }),
    /cannot be combined/,
  );
});

test('a failed fixture build cannot launch the preceding normal app', async () => {
  const f = fixture({ fail: (command, args) => command === 'pnpm' && args.includes('--features') });
  await assert.rejects(
    checkNative({ base, fixtureSmoke: true, platform: 'darwin', ...f }),
    /stopped/,
  );
  assert.ok(!f.calls.some((call) => call.startsWith('node scripts/ci/debug-bundle.mjs')));
  await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
});

test('a source advance cannot produce successful native evidence', async () => {
  const f = fixture({ changed: true });
  await assert.rejects(checkNative({ base, platform: 'darwin', ...f }), /Source changed/);
  await assert.rejects(access(f.trusted()), { code: 'ENOENT' });
});
