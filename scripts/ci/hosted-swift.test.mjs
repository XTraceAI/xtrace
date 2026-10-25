import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { hostedSwift } from './hosted-swift.mjs';

test('fixed Swift assembly rejects unknown, missing, duplicate and unexpanded inputs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'hosted-swift-'));
  const first = 'hosted-ax-update-ui-policy.swift';
  const second = 'hosted-ax-telemetry-ui-policy.swift';
  const marker = (name) => `// __${name}__`;
  const path = join(root, 'hosted-ax.swift');
  try {
    await writeFile(join(root, first), 'func update() {} // $& stays literal');
    await writeFile(join(root, second), 'func telemetry() {}');
    await assert.rejects(hostedSwift('unrecognized', root), /Unknown hosted Swift entrypoint/);
    for (const source of [marker(first), `${marker(first)}\n${marker(first)}\n${marker(second)}`]) {
      await writeFile(path, source);
      await assert.rejects(
        hostedSwift('hosted-ax', root),
        /Expected one Swift policy input marker/,
      );
    }
    await writeFile(path, `${marker(first)}\n${marker(second)}\n${marker('unexpected.swift')}`);
    await assert.rejects(hostedSwift('hosted-ax', root), /Unexpanded Swift policy input/);
    await writeFile(path, `${marker(first)}\n${marker(second)}\nprint("done")`);
    assert.equal(
      await hostedSwift('hosted-ax', root),
      'func update() {} // $& stays literal\nfunc telemetry() {}\nprint("done")',
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('workflow materialization command writes the complete cancellation program', async () => {
  const root = await mkdtemp(join(tmpdir(), 'hosted-swift-command-'));
  try {
    const output = join(root, 'cancel.swift');
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL('./hosted-swift.mjs', import.meta.url)),
        'hosted-updater-cancel',
        output,
      ],
      { encoding: 'utf8' },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(output, 'utf8'), await hostedSwift('hosted-updater-cancel'));
    assert.doesNotMatch(await readFile(output, 'utf8'), /\/\/ __[^\n]+\.swift__/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
