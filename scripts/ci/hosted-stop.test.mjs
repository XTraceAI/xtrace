import { swiftPolicy, writeHostedSwift } from './hosted-swift.mjs';
import { compilerControls } from './hosted-compiler-controls.mjs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const { recordCompiler, diagnoseCompiler, cleanupCompiler } = compilerControls();

const nativeControlBudgetMs = 120_000;

test('both signed jobs include stop controls and allow their native test budget', async () => {
  const workflow = await readFile(
    new URL('../../.github/workflows/release-native.yml', import.meta.url),
    'utf8',
  );
  for (const name of ['signed-first-launch', 'signed-update']) {
    const job = workflow.split(`  ${name}:\n`)[1].split(/\n {2}[a-z].*:\n/)[0];
    assert.match(job, /(?:node --test|TESTS=\()[^\n]*scripts\/ci\/hosted-stop.test.mjs/);
    if (name === 'signed-first-launch') assert.match(job, /node --test "\$\{TESTS\[@\]\}"/);
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
    const deadline = performance.now() + nativeControlBudgetMs;
    const compiler = { retain: false };
    const temporary = await mkdtemp(join(tmpdir(), 'hosted-stop-'));
    try {
      const owned = join(temporary, 'checks.noindex');
      await mkdir(owned);
      const cache = join(owned, 'compiler-cache');
      const typecheckBegan = performance.now();
      const fullCheck = spawnSync(
        '/usr/bin/xcrun',
        [
          'swiftc',
          '-typecheck',
          '-module-cache-path',
          cache,
          await writeHostedSwift(
            fileURLToPath(new URL('.', import.meta.url)),
            'hosted-launch',
            join(owned, 'hosted-launch.swift'),
          ),
        ],
        { encoding: 'utf8', timeout: 60000 },
      );
      recordCompiler(
        compiler,
        'stop-launcher-typecheck',
        60000,
        performance.now() - typecheckBegan,
        fullCheck,
      );
      const receipt = await swiftPolicy('RECEIPT');
      const strictStart = await swiftPolicy('PROCESS START');
      const controls = await readFile(
        new URL('./fixtures/hosted-stop-controls.swift', import.meta.url),
        'utf8',
      );
      const source =
        'import Foundation\nimport Darwin\nfunc fail(_ text: String) -> Never { fatalError(text) }\n' +
        receipt +
        (await swiftPolicy('TRANSITION POLICY')) +
        (await swiftPolicy('STOP POLICY')) +
        (await swiftPolicy('PROCESS QUERY')) +
        strictStart +
        controls;
      const sourcePath = join(owned, 'main.swift');
      const binary = join(owned, 'controls');
      await writeFile(sourcePath, source);
      const compileBegan = performance.now();
      const built = spawnSync(
        '/usr/bin/xcrun',
        ['swiftc', '-module-cache-path', cache, sourcePath, '-o', binary],
        { encoding: 'utf8', timeout: 40000 },
      );
      recordCompiler(
        compiler,
        'stop-controls-compile',
        40000,
        performance.now() - compileBegan,
        built,
      );
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
    } catch (error) {
      diagnoseCompiler(compiler, deadline);
      throw error;
    } finally {
      await cleanupCompiler(temporary, compiler);
    }
  },
);
