import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { CheckError } from './policy.mjs';
import {
  checkFreshness,
  checkoutSource,
  latestRelease,
  parseFreshnessArgs,
  parsePin,
  readPin,
  remoteSource,
  reportFreshness,
  tagsFromLsRemote,
  verifyCheckout,
} from './plugin-pin.mjs';

const valid = {
  repository: 'https://github.com/XTraceAI/agent-plugins.git',
  commit: 'd7c94227cc9bd8ff46f933539ac65b478dd8f8ef',
  plugin_root: 'plugins/memhub',
  plugin_version: '0.55.0',
  reader_sources: {
    'plugins/memhub/scripts/readers_cli.py': 'b95348419605e61029447d10de818618c59c131d',
  },
};

test('the committed pin parses and names a full commit, root, version and reader sources', async () => {
  const pin = await readPin(process.cwd());
  assert.match(pin.commit, /^[0-9a-f]{40}$/);
  assert.equal(pin.plugin_root, 'plugins/memhub');
  assert.ok(Object.keys(pin.reader_sources).length >= 3);
  assert.deepEqual(parsePin(await readFile('.plugin-pin', 'utf8')), pin);
});

test('malformed pins are rejected with a specific reason', () => {
  const variants = [
    ['not json', /JSON object/],
    ['[]', /JSON object/],
    [JSON.stringify({ ...valid, extra: 1 }), /known keys/],
    [
      JSON.stringify({ ...valid, repository: 'https://example.com/x.git' }),
      /reviewed public producer/,
    ],
    [JSON.stringify({ ...valid, commit: 'd7c9422' }), /full SHA/],
    [JSON.stringify({ ...valid, plugin_root: '../memhub' }), /relative path/],
    [JSON.stringify({ ...valid, plugin_version: 'v0.55' }), /release version/],
    [JSON.stringify({ ...valid, reader_sources: {} }), /list reader sources/],
    [JSON.stringify({ ...valid, reader_sources: { 'a/../b': valid.commit } }), /object IDs/],
    [JSON.stringify({ ...valid, reader_sources: { 'plugins/x.py': 'abc' } }), /object IDs/],
  ];
  for (const [text, reason] of variants) assert.throws(() => parsePin(text), reason, text);
  assert.deepEqual(parsePin(JSON.stringify(valid)), valid);
});

test('a checkout passes only at the pinned commit with identical reader objects and a clean tree', () => {
  const fake =
    (head, sources, dirty = '') =>
    (args) => {
      if (args[0] === 'rev-parse' && args[1] === 'HEAD') return `${head}\n`;
      if (args[0] === 'rev-parse') {
        const path = args[2].slice('HEAD:'.length);
        if (!(path in sources)) throw new Error('fatal: Needed a single revision');
        return `${sources[path]}\n`;
      }
      if (args[0] === 'status') return dirty;
      throw new Error(`unexpected git ${args.join(' ')}`);
    };
  const path = 'plugins/memhub/scripts/readers_cli.py';
  assert.equal(
    verifyCheckout(valid, fake(valid.commit, { [path]: valid.reader_sources[path] })),
    valid.commit,
  );
  assert.throws(
    () => verifyCheckout(valid, fake('0'.repeat(40), { [path]: valid.reader_sources[path] })),
    /not at the pinned commit/,
  );
  assert.throws(
    () => verifyCheckout(valid, fake(valid.commit, { [path]: '1'.repeat(40) })),
    /differs/,
  );
  assert.throws(() => verifyCheckout(valid, fake(valid.commit, {})), /absent/);
  assert.throws(
    () =>
      verifyCheckout(valid, fake(valid.commit, { [path]: valid.reader_sources[path] }, ' M x\n')),
    /local modifications/,
  );
});

const readerPin = {
  ...valid,
  reader_sources: {
    'plugins/memhub/scripts': 'a'.repeat(40),
    'plugins/memhub/scripts/readers': 'b'.repeat(40),
    'plugins/memhub/scripts/readers_cli.py': 'c'.repeat(40),
  },
};

// A fake producer: tag names plus the reader objects each tag holds.
const fakeSource = (tags, objectsByTag) => ({
  tags: () => tags,
  objects: (tag, paths) =>
    new Map(
      paths.filter((path) => path in objectsByTag[tag]).map((p) => [p, objectsByTag[tag][p]]),
    ),
});

test('the latest release is the highest memhub SemVer tag, not the newest or another plugin', () => {
  assert.deepEqual(
    latestRelease(
      [
        'memhub--v0.99.0',
        'memhub--v0.133.0',
        'memhub--v0.124.9',
        'memhub--v1.0.0-beta.1',
        'memhub--v0.200',
        'memhub--v0.0140.0',
        'other--v9.0.0',
        'v5.0.0',
        'memhub-v3.0.0',
      ],
      'memhub',
    ),
    { tag: 'memhub--v0.133.0', version: '0.133.0' },
  );
  assert.equal(latestRelease(['memhub--v0.9.10', 'memhub--v0.10.2'], 'memhub').version, '0.10.2');
  assert.throws(() => latestRelease(['other--v1.0.0'], 'memhub'), /no memhub release tag/);
  assert.deepEqual(
    tagsFromLsRemote(
      'a50949cc\trefs/tags/memhub--v0.133.0\nffff\trefs/tags/memhub--v0.99.0\nbeef\trefs/heads/main\n',
    ),
    ['memhub--v0.133.0', 'memhub--v0.99.0'],
  );
});

test('identical reader objects at the latest release report up to date', () => {
  const source = fakeSource(['memhub--v0.99.0', 'memhub--v0.133.0'], {
    'memhub--v0.133.0': { ...readerPin.reader_sources },
  });
  const result = checkFreshness(readerPin, source);
  assert.equal(result.state, 'current');
  const lines = [];
  const message = reportFreshness(result, { env: {}, log: (line) => lines.push(line) });
  assert.match(message, /up to date with memhub 0\.133\.0/);
  assert.deepEqual(lines, [message]);
});

test('changed or removed reader paths are named in a warning that does not fail', async () => {
  const source = fakeSource(['memhub--v0.133.0'], {
    'memhub--v0.133.0': {
      'plugins/memhub/scripts': 'd'.repeat(40),
      'plugins/memhub/scripts/readers': 'b'.repeat(40),
    },
  });
  const result = checkFreshness(readerPin, source);
  assert.equal(result.state, 'stale');
  assert.deepEqual(result.differing, [
    { path: 'plugins/memhub/scripts', missing: false },
    { path: 'plugins/memhub/scripts/readers_cli.py', missing: true },
  ]);
  const directory = await mkdtemp(join(tmpdir(), 'xtrace-pin-summary-'));
  try {
    const summary = join(directory, 'summary.md');
    await writeFile(summary, '');
    const lines = [];
    const message = reportFreshness(result, {
      env: { GITHUB_ACTIONS: 'true', GITHUB_STEP_SUMMARY: summary },
      log: (line) => lines.push(line),
    });
    assert.match(message, /from memhub 0\.55\.0, but the latest release is memhub 0\.133\.0/);
    assert.match(
      message,
      /plugins\/memhub\/scripts, plugins\/memhub\/scripts\/readers_cli\.py \(no longer present\)/,
    );
    assert.doesNotMatch(message, /scripts\/readers,/);
    assert.match(message, /scripts\/vendor-readers\.sh/);
    assert.deepEqual(lines, [`::warning title=Plugin readers::${message}`]);
    assert.equal(await readFile(summary, 'utf8'), `- Warning: ${message}\n`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('a failed lookup is a visible warning, never up to date', () => {
  const failures = [
    {
      tags: () => {
        throw new CheckError('git ls-remote failed (could not resolve host)');
      },
    },
    {
      tags: () => ['memhub--v0.133.0'],
      objects: () => {
        throw new Error('disk full');
      },
    },
    fakeSource(['other--v1.0.0'], {}),
  ];
  for (const source of failures) {
    const result = checkFreshness(readerPin, source);
    assert.equal(result.state, 'unknown');
    const lines = [];
    const message = reportFreshness(result, { env: {}, log: (line) => lines.push(line) });
    assert.match(message, /Could not check .*memhub 0\.55\.0.*This is not a pass/);
    assert.doesNotMatch(message, /up to date with/);
    assert.deepEqual(lines, [`Warning: ${message}`]);
  }
  const annotated = [];
  reportFreshness(checkFreshness(readerPin, failures[0]), {
    env: { GITHUB_ACTIONS: 'true' },
    log: (line) => annotated.push(line),
  });
  assert.match(annotated[0], /^::warning title=Plugin readers::Could not check/);
});

test('the remote source lists published tags and fetches only the chosen tag without blobs', () => {
  const calls = [];
  const removed = [];
  const git = (args, cwd) => {
    calls.push([cwd, ...args]);
    if (args[0] === 'ls-remote')
      return 'x\trefs/tags/memhub--v0.9.0\ny\trefs/tags/memhub--v0.10.0\n';
    if (args[0] === 'rev-parse' && args[2] === 'FETCH_HEAD^{commit}') return `${'e'.repeat(40)}\n`;
    if (args[0] === 'rev-parse') {
      const path = args[3].split(':')[1];
      if (!(path in readerPin.reader_sources)) throw new Error('absent');
      return `${readerPin.reader_sources[path]}\n`;
    }
    return '';
  };
  const source = remoteSource(readerPin.repository, git, () => ({
    path: '/temporary',
    remove: () => removed.push('/temporary'),
  }));
  const result = checkFreshness(readerPin, source);
  assert.equal(result.state, 'current');
  assert.equal(result.latest, '0.10.0');
  assert.deepEqual(calls[0], [undefined, 'ls-remote', '--tags', '--refs', readerPin.repository]);
  assert.deepEqual(calls[2], [
    '/temporary',
    'fetch',
    '-q',
    '--depth',
    '1',
    '--filter=blob:none',
    '--no-tags',
    readerPin.repository,
    'refs/tags/memhub--v0.10.0',
  ]);
  assert.deepEqual(removed, ['/temporary']);
});

test('a local producer checkout answers from its own tags', async () => {
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-pin-checkout-'));
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Synthetic CI',
    GIT_AUTHOR_EMAIL: 'ci@example.invalid',
    GIT_COMMITTER_NAME: 'Synthetic CI',
    GIT_COMMITTER_EMAIL: 'ci@example.invalid',
  };
  const git = (args, input = '') => {
    const result = spawnSync('git', args, { cwd: repo, env, input, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  const runner = (args, cwd) => {
    const result = spawnSync('git', args, { cwd, env, encoding: 'utf8' });
    if (result.status !== 0) throw new CheckError(`git ${args[0]} failed`);
    return result.stdout;
  };
  try {
    git(['init', '-q', '--template=']);
    const blob = (text) => git(['hash-object', '-w', '--stdin'], text);
    const tree = (entries) => git(['mktree'], entries.join('\n') + '\n');
    const commit = (cli) => {
      const scripts = tree([`100755 blob ${blob(cli)}\treaders_cli.py`]);
      const memhub = tree([`040000 tree ${scripts}\tscripts`]);
      const plugins = tree([`040000 tree ${memhub}\tmemhub`]);
      const root = tree([`040000 tree ${plugins}\tplugins`]);
      return { id: git(['commit-tree', root, '-m', 'Synthetic release']), scripts };
    };
    const old = commit('old reader\n');
    const fresh = commit('new reader\n');
    git(['tag', 'memhub--v0.99.0', fresh.id]);
    git(['tag', '-a', 'memhub--v0.133.0', old.id, '-m', 'Synthetic annotated release']);
    const pin = {
      ...valid,
      reader_sources: {
        'plugins/memhub/scripts': old.scripts,
        'plugins/memhub/scripts/readers_cli.py': git([
          'rev-parse',
          `${old.id}:plugins/memhub/scripts/readers_cli.py`,
        ]),
      },
    };
    // 0.133.0 is the higher version, though 0.99.0 points at the newer content.
    assert.deepEqual(checkFreshness(pin, checkoutSource(repo, runner)), {
      state: 'current',
      plugin: 'memhub',
      pinned: '0.55.0',
      latest: '0.133.0',
      differing: [],
    });
    git(['tag', 'memhub--v0.134.0', fresh.id]);
    const stale = checkFreshness(pin, checkoutSource(repo, runner));
    assert.equal(stale.state, 'stale');
    assert.deepEqual(
      stale.differing.map(({ path }) => path),
      ['plugins/memhub/scripts', 'plugins/memhub/scripts/readers_cli.py'],
    );
    assert.equal(
      checkFreshness(pin, checkoutSource(join(repo, 'missing'), runner)).state,
      'unknown',
    );
  } finally {
    await rm(repo, { recursive: true, force: true });
  }
});

test('the command warns but exits zero on lookup failure and rejects bad usage or pins', async () => {
  const script = new URL('./plugin-pin.mjs', import.meta.url).pathname;
  const directory = await mkdtemp(join(tmpdir(), 'xtrace-pin-cli-'));
  // The child must not write real annotations or summary lines when this
  // suite itself runs inside GitHub Actions.
  const childEnv = { ...process.env };
  for (const name of Object.keys(childEnv))
    if (name.startsWith('GITHUB_') || name === 'CI') delete childEnv[name];
  const run = (args) =>
    spawnSync(process.execPath, [script, ...args], {
      cwd: directory,
      encoding: 'utf8',
      env: childEnv,
    });
  try {
    await writeFile(join(directory, '.plugin-pin'), JSON.stringify(valid));
    const missing = run(['freshness', '--checkout', join(directory, 'no-checkout')]);
    assert.equal(missing.status, 0, missing.stderr);
    assert.match(missing.stdout, /Warning: Could not check whether the bundled plugin readers/);
    assert.doesNotMatch(missing.stdout, /up to date with/);
    for (const args of [
      ['freshness', '--checkout'],
      ['freshness', '--offline'],
      ['commit', 'extra'],
      [],
    ]) {
      const result = run(args);
      assert.equal(result.status, 1, args.join(' '));
      assert.match(result.stderr, /Usage:/);
    }
    assert.equal(run(['commit']).stdout.trim(), valid.commit);
    await writeFile(join(directory, '.plugin-pin'), '{"repository": 1}');
    const malformed = run(['freshness', '--checkout', directory]);
    assert.equal(malformed.status, 1);
    assert.match(malformed.stderr, /known keys/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
  assert.deepEqual(parseFreshnessArgs([]), {});
  assert.deepEqual(parseFreshnessArgs(['--checkout', '../agent-plugins']), {
    checkout: '../agent-plugins',
  });
});

test('differing readers read correctly when the pin names the latest or a newer version', () => {
  const differing = {
    'memhub--v0.133.0': { ...readerPin.reader_sources, 'plugins/memhub/scripts': 'd'.repeat(40) },
  };
  for (const pinned of ['0.133.0', '0.140.0', '1.0.0']) {
    const result = checkFreshness(
      { ...readerPin, plugin_version: pinned },
      fakeSource(['memhub--v0.133.0'], differing),
    );
    assert.equal(result.state, 'stale');
    const lines = [];
    const message = reportFreshness(result, { env: {}, log: (line) => lines.push(line) });
    assert.equal(
      message,
      `The bundled plugin readers differ from memhub 0.133.0, the latest release, even though .plugin-pin names memhub ${pinned}. ` +
        'These reader paths differ: plugins/memhub/scripts. Check that the pinned commit is the released one; ' +
        'to match the release, refresh vendor/agent-plugins with scripts/vendor-readers.sh and update .plugin-pin.',
    );
    assert.doesNotMatch(message, /but the latest release is/);
    assert.deepEqual(lines, [`Warning: ${message}`]);
  }
  const behind = checkFreshness(
    { ...readerPin, plugin_version: '0.124.9' },
    fakeSource(['memhub--v0.133.0'], differing),
  );
  assert.match(
    reportFreshness(behind, { env: {}, log: () => {} }),
    /from memhub 0\.124\.9, but the latest release is memhub 0\.133\.0/,
  );
});
