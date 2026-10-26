import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';

const cli = new URL('./select-ui.mjs', import.meta.url).pathname;

test('the actual selector defaults to UI unless a verified PR changes only allowed files', async (t) => {
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-ui-selection-'));
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Synthetic CI',
    GIT_AUTHOR_EMAIL: 'ci@example.invalid',
    GIT_COMMITTER_NAME: 'Synthetic CI',
    GIT_COMMITTER_EMAIL: 'ci@example.invalid',
  };
  const git = (...args) => {
    const result = spawnSync('git', args, { cwd: repo, env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  const write = async (path, content) => {
    await mkdir(dirname(join(repo, path)), { recursive: true });
    await writeFile(join(repo, path), content);
  };
  const commit = () => {
    git('add', '.');
    git('commit', '-qm', 'Synthetic change');
    return git('rev-parse', 'HEAD');
  };
  const manifest = { scripts: { build: 'build', 'test:ci': 'test' }, dependencies: { ui: '1' } };
  try {
    git('init', '--template=', '--initial-branch=main');
    await write('package.json', JSON.stringify(manifest));
    await write('apps/desktop/ui/src/app.tsx', 'original UI');
    await write('docs/CI.md', 'original docs');
    const base = commit();
    const eventFor = (head) => ({
      action: 'synchronize',
      pull_request: { base: { sha: base }, head: { sha: head } },
    });
    const run = async (event, expected, overrides = {}) => {
      const eventPath = join(repo, '.git', 'event.json');
      const outputPath = join(repo, '.git', 'output');
      await writeFile(eventPath, typeof event === 'string' ? event : JSON.stringify(event));
      await writeFile(outputPath, '');
      const result = spawnSync(process.execPath, [cli], {
        cwd: repo,
        env: {
          ...env,
          GITHUB_EVENT_NAME: 'pull_request',
          GITHUB_EVENT_PATH: eventPath,
          GITHUB_SHA: git('rev-parse', 'HEAD'),
          GITHUB_OUTPUT: outputPath,
          ...overrides,
        },
        encoding: 'utf8',
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, `run_ui=${expected}\n`);
      assert.equal(await readFile(outputPath, 'utf8'), result.stdout);
    };
    for (const [name, files, expected] of [
      [
        'docs and notes',
        ['docs/CI.md', 'docs/acceptance/ci.md', 'change-notes/synthetic.md'],
        false,
      ],
      [
        'PR preparation paths',
        [
          '.agents/skills/xtrace-pr-preparation/SKILL.md',
          '.github/pull_request_template.md',
          'AGENTS.md',
          'CONTRIBUTING.md',
          'scripts/pr/prepare.mjs',
          'scripts/pr/prepare.test.mjs',
        ],
        false,
      ],
      ['UI', ['apps/desktop/ui/src/app.tsx'], true],
      ['unknown', ['new-area/README.md'], true],
      ['dependency lock', ['pnpm-lock.yaml'], true],
      ['build', ['scripts/build-updater.mjs'], true],
      ['runtime docs input', ['docs/acceptance/metrics/rule-coverage.json'], true],
      ['UI markdown', ['apps/desktop/ui/public/fonts/README.md'], true],
      ['CI workflow', ['.github/workflows/ci.yml'], true],
      ['other skill', ['.agents/skills/other/SKILL.md'], true],
    ]) {
      await t.test(name, async () => {
        git('reset', '--hard', base);
        for (const path of files) await write(path, 'changed');
        await run(eventFor(commit()), expected);
      });
    }
    for (const [name, content, expected] of [
      [
        'only test:ci script',
        JSON.stringify({
          ...manifest,
          scripts: { ...manifest.scripts, 'test:ci': 'node --test scripts/pr/*.test.mjs' },
        }),
        false,
      ],
      ['formatting only', JSON.stringify(manifest, null, 2), false],
      ['dependency', JSON.stringify({ ...manifest, dependencies: { ui: '2' } }), true],
      [
        'other script',
        JSON.stringify({ ...manifest, scripts: { ...manifest.scripts, build: 'other' } }),
        true,
      ],
      ['malformed manifest', '{', true],
      ['invalid scripts', JSON.stringify({ ...manifest, scripts: null }), true],
      ['missing manifest', null, true],
    ]) {
      await t.test(name, async () => {
        git('reset', '--hard', base);
        if (content === null) await rm(join(repo, 'package.json'));
        else await write('package.json', content);
        await run(eventFor(commit()), expected);
      });
    }
    await t.test('rename from UI to docs keeps UI', async () => {
      git('reset', '--hard', base);
      git('mv', 'apps/desktop/ui/src/app.tsx', 'docs/renamed.md');
      await run(eventFor(commit()), true);
    });
    git('reset', '--hard', base);
    await write('docs/CI.md', 'changed');
    const head = commit();
    const event = eventFor(head);
    await write('package.json', 'uncommitted invalid manifest');
    await t.test('reads committed blobs instead of the working tree', () => run(event, false));
    git('restore', 'package.json');
    for (const action of ['opened', 'reopened', 'synchronize', 'ready_for_review'])
      await t.test(action, () => run({ ...event, action }, false));
    for (const [name, input, overrides] of [
      ['main push', event, { GITHUB_EVENT_NAME: 'push' }],
      ['merge group', event, { GITHUB_EVENT_NAME: 'merge_group' }],
      ['other event', event, { GITHUB_EVENT_NAME: 'workflow_call' }],
      ['base edit inherited by reusable CI', { ...event, action: 'edited', changes: { base: {} } }],
      ['unexpected base change', { ...event, changes: { base: {} } }],
      ['text edit', { ...event, action: 'edited' }],
      ['missing PR', {}],
      ['malformed event', '{'],
      ['missing event file', event, { GITHUB_EVENT_PATH: join(repo, 'absent') }],
      [
        'missing base ref',
        { ...event, pull_request: { ...event.pull_request, base: { sha: 'a'.repeat(40) } } },
      ],
      [
        'invalid head ref',
        { ...event, pull_request: { ...event.pull_request, head: { sha: '--help' } } },
      ],
      ['head context mismatch', event, { GITHUB_SHA: base }],
      ['missing context SHA', event, { GITHUB_SHA: '' }],
      ['event head mismatch', eventFor(base)],
    ])
      await t.test(name, () => run(input, true, overrides));
    const merge = git(
      'commit-tree',
      `${head}^{tree}`,
      '-p',
      base,
      '-p',
      head,
      '-m',
      'Synthetic merge',
    );
    git('reset', '--hard', merge);
    await t.test('verified GitHub merge checkout', () => run(event, false));
    await t.test('mismatched merge base', () =>
      run({ ...event, pull_request: { ...event.pull_request, base: { sha: head } } }, true),
    );
    git('reset', '--hard', base);
    await write('apps/desktop/ui/src/app.tsx', 'base branch UI change');
    const advancedBase = commit();
    git('reset', '--hard', head);
    await t.test('source checkout behind a diverged base', () =>
      run({ ...event, pull_request: { ...event.pull_request, base: { sha: advancedBase } } }, true),
    );
    git('reset', '--hard', base);
    await t.test('empty diff', () => run(eventFor(base), true));
    await t.test('failure to publish selection output fails the CLI', () => {
      const result = spawnSync(process.execPath, [cli], {
        cwd: repo,
        env: { ...env, GITHUB_EVENT_PATH: '', GITHUB_OUTPUT: join(repo, '.git') },
        encoding: 'utf8',
      });
      assert.equal(result.status, 1);
      assert.equal(result.stdout, '');
    });
    await write('package.json', '{');
    const malformedBase = commit();
    await write('package.json', JSON.stringify(manifest));
    await t.test('unreadable base manifest', () =>
      run(
        {
          ...eventFor(commit()),
          pull_request: { base: { sha: malformedBase }, head: { sha: git('rev-parse', 'HEAD') } },
        },
        true,
      ),
    );
  } finally {
    await rm(repo, { recursive: true, force: true });
  }
});
