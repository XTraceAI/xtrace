import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const nativeControlBudgetMs = 120_000;
const launcher = await readFile(new URL('./hosted-launch.swift', import.meta.url), 'utf8');
const block = (name) =>
  launcher.split(`// BEGIN ${name}:`)[1].split('\n').slice(1).join('\n').split(`// END ${name}`)[0];

test('both signed jobs include stop controls and allow their native test budget', async () => {
  const workflow = await readFile(
    new URL('../../.github/workflows/release-native.yml', import.meta.url),
    'utf8',
  );
  for (const name of ['signed-first-launch', 'signed-update']) {
    const job = workflow.split(`  ${name}:\n`)[1].split(/\n {2}[a-z].*:\n/)[0];
    assert.match(job, /node --test[^\n]*scripts\/ci\/hosted-stop.test.mjs/);
    const minutes = [...job.matchAll(/^ {8}timeout-minutes: (\d+)$/gm)].map((match) =>
      Number(match[1]),
    );
    assert.ok(minutes[3] * 60_000 > nativeControlBudgetMs);
  }
});

test(
  'actual Swift stop policy and kernel query preserve ownership through owned-child exit',
  { skip: process.platform !== 'darwin', timeout: nativeControlBudgetMs },
  async () => {
    const temporary = await mkdtemp(join(tmpdir(), 'hosted-stop-'));
    try {
      const owned = join(temporary, 'checks.noindex');
      await mkdir(owned);
      const cache = join(owned, 'compiler-cache');
      const fullCheck = spawnSync(
        '/usr/bin/xcrun',
        [
          'swiftc',
          '-typecheck',
          '-module-cache-path',
          cache,
          fileURLToPath(new URL('./hosted-launch.swift', import.meta.url)),
        ],
        { encoding: 'utf8', timeout: 60000 },
      );
      assert.equal(fullCheck.status, 0, fullCheck.stderr);
      const receipt = launcher.slice(
        launcher.indexOf('struct Receipt:'),
        launcher.indexOf('// BEGIN TRANSITION POLICY'),
      );
      const strictStart = launcher.slice(
        launcher.indexOf('func processStart('),
        launcher.indexOf('\nvar stopPhase'),
      );
      const controls = await readFile(
        new URL('./fixtures/hosted-stop-controls.swift', import.meta.url),
        'utf8',
      );
      const source =
        'import Foundation\nimport Darwin\nfunc fail(_ text: String) -> Never { fatalError(text) }\n' +
        receipt +
        block('TRANSITION POLICY') +
        block('STOP POLICY') +
        block('PROCESS QUERY') +
        strictStart +
        controls;
      const sourcePath = join(owned, 'main.swift');
      const binary = join(owned, 'controls');
      await writeFile(sourcePath, source);
      const built = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', cache, sourcePath, '-o', binary],
        { encoding: 'utf8', timeout: 40000 },
      );
      assert.equal(built.status, 0, built.stderr);
      const ran = spawnSync(binary, [], { encoding: 'utf8', timeout: 10000 });
      assert.equal(ran.status, 0, ran.stderr);
      const report = JSON.parse(ran.stdout);
      assert.equal(report.status, 'passed');
      assert.equal(report.pureStopSequences, 12);
      assert.equal(report.deadlineControls, 8);
      assert.equal(report.children.length, 2);
      for (const child of report.children) {
        assert.equal(child.status, 'passed');
        assert.equal(child.reaped, true);
        assert.equal(child.waitpidReturn, child.pid);
        assert.equal(child.observations.at(-1).postRequestDecision, 'absent');
      }
      assert.equal(report.xtraceLaunched, false);
      console.log(JSON.stringify(report));
    } finally {
      await rm(temporary, { recursive: true, force: true });
    }
  },
);
