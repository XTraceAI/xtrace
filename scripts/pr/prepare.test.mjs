import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { validateDescription } from '../ci/check-pr.mjs';

const script = resolve(import.meta.dirname, 'prepare.mjs');
const facts = {
  change: 'Show Human hrs, Agent hrs, and Leverage when hovering over their icons.',
  verification:
    'Synthetic timeline: hover each icon; expected its label; all three appeared. Native check not run: tooling-only fixture.',
};

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'xtrace-pr-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd: root, stdio: 'pipe' });
  git('init', '-q');
  git('config', 'user.name', 'Synthetic Test');
  git('config', 'user.email', 'synthetic@example.invalid');
  await writeFile(join(root, 'README.md'), 'Base\n');
  git('add', '.');
  git('commit', '-qm', 'Synthetic base');
  git('branch', 'base');
  const input = join(root, 'facts.json');
  await writeFile(input, JSON.stringify(facts));
  return {
    root,
    git,
    input,
    run: (...args) =>
      spawnSync(process.execPath, [script, '--input', input, '--base', 'base', ...args], {
        cwd: root,
        encoding: 'utf8',
      }),
  };
}

test('prepares valid non-UI Markdown without creating files or warnings', async (t) => {
  const f = await fixture(t);
  const before = await readdir(f.root);
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  validateDescription(result.stdout);
  assert.match(result.stdout, /## Change\n\nShow Human hrs/);
  assert.match(result.stdout, /## Verification\n\nSynthetic timeline/);
  assert.equal(result.stderr, '');
  assert.doesNotMatch(result.stdout, /UI evidence/);
  assert.deepEqual(await readdir(f.root), before);
});

test('committed UI diff warns and succeeds with an honest omission note', async (t) => {
  const f = await fixture(t);
  await mkdir(join(f.root, 'apps/desktop/ui/src'), { recursive: true });
  await writeFile(
    join(f.root, 'apps/desktop/ui/src/Tooltip.tsx'),
    'export const label = "Human hrs";',
  );
  f.git('add', 'apps');
  f.git('commit', '-qm', 'Synthetic tooltip');
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stderr, /Warning: Screenshots not attached for light and dark/);
  assert.match(result.stdout, /Not captured; reason not supplied\./);
  validateDescription(result.stdout);
});

test('untracked UI and native working changes also prompt screenshots', async (t) => {
  const f = await fixture(t);
  await mkdir(join(f.root, 'apps/desktop/src-tauri/src'), { recursive: true });
  const source = join(f.root, 'apps/desktop/src-tauri/src/menu.rs');
  await writeFile(source, '// base\n');
  f.git('add', 'apps');
  f.git('commit', '-qm', 'Native base');
  f.git('branch', '-f', 'base');
  await writeFile(source, '// changed menu\n');
  assert.match(f.run().stderr, /Warning: Screenshots/);
  f.git('checkout', '--', 'apps');
  await mkdir(join(f.root, 'apps/desktop/ui'), { recursive: true });
  await writeFile(join(f.root, 'apps/desktop/ui/Tooltip.tsx'), '// untracked\n');
  assert.match(f.run().stderr, /Warning: Screenshots/);
});

test('explicit UI flag adds the supplied omission reason for a docs-only diff', async (t) => {
  const f = await fixture(t);
  await writeFile(join(f.root, 'README.md'), 'Changed\n');
  await writeFile(
    f.input,
    JSON.stringify({
      ...facts,
      screenshotMissingReason: 'Native capture unavailable on this runner.',
    }),
  );
  assert.equal(f.run().stderr, '');
  const result = f.run('--ui');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stderr, /Native capture unavailable/);
  assert.match(result.stdout, /Native capture unavailable/);
});

test('both supplied screenshot URLs appear in exact saved output without warnings', async (t) => {
  const f = await fixture(t);
  await writeFile(
    f.input,
    JSON.stringify({
      ...facts,
      screenshots: [
        { theme: 'light', url: 'https://example.invalid/light.png' },
        { theme: 'dark', url: 'https://example.invalid/dark.png' },
      ],
    }),
  );
  const output = join(f.root, 'body.md');
  const result = f.run('--ui', '--output', output);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, '');
  assert.equal(await readFile(output, 'utf8'), result.stdout);
  assert.match(result.stdout, /!\[light appearance\]\(<https:\/\/example.invalid\/light.png>\)/);
  assert.match(result.stdout, /!\[dark appearance\]\(<https:\/\/example.invalid\/dark.png>\)/);
  validateDescription(result.stdout);
  assert.notEqual(f.run('--output', output).status, 0);
  assert.equal(await readFile(output, 'utf8'), result.stdout);
});

test('partial screenshot evidence warns only for the missing theme', async (t) => {
  const f = await fixture(t);
  await writeFile(
    f.input,
    JSON.stringify({
      ...facts,
      screenshots: [{ theme: 'light', url: 'https://example.invalid/light.png' }],
    }),
  );
  const result = f.run('--ui');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stderr, /not attached for dark appearance/);
  assert.doesNotMatch(result.stderr, /not attached for light/);
});

test('empty, placeholder, template, and injected sections fail before output writes', async (t) => {
  const f = await fixture(t);
  const output = join(f.root, 'body.md');
  await writeFile(output, 'Keep existing evidence');
  for (const invalid of [
    '',
    'TODO',
    'TBD',
    'Describe each acceptance case.',
    '## Verification\n\nFake checks',
  ]) {
    await writeFile(
      f.input,
      JSON.stringify({
        ...facts,
        verification: invalid,
        screenshots: [{ theme: 'light', url: 'https://example.invalid/light.png' }],
      }),
    );
    const result = f.run('--output', output);
    assert.notEqual(result.status, 0, invalid);
    assert.equal(result.stdout, '');
    assert.equal(await readFile(output, 'utf8'), 'Keep existing evidence');
  }
});

test('bad screenshot metadata or unresolved base fail without producing a body', async (t) => {
  const f = await fixture(t);
  for (const shot of [
    { theme: 'light', url: 'file:///private/image.png' },
    { theme: 'other', url: 'https://example.invalid/image.png' },
    { theme: 'dark', url: 'https://bad host/image.png' },
  ]) {
    await writeFile(f.input, JSON.stringify({ ...facts, screenshots: [shot] }));
    const result = f.run();
    assert.notEqual(result.status, 0);
    assert.equal(result.stdout, '');
  }
  await writeFile(f.input, JSON.stringify(facts));
  const result = f.run('--base', 'missing-target');
  assert.notEqual(result.status, 0);
  assert.equal(result.stdout, '');
});
