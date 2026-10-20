import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { guardProcessDetails, guardFailureMetadata } from './hosted-guard-diagnostics.mjs';

// BEGIN COMPILER DIAGNOSTICS: local assertions and owned temporary cleanup.
function recordCompiler(owner, label, timeout, elapsed, result) {
  owner.hasResult = true;
  owner.retain ||= Boolean(result.error || result.signal || result.status === null);
  let message = 'Compiler result diagnostics unavailable';
  try {
    message = JSON.stringify(guardProcessDetails(label, timeout, elapsed, result));
    console.log(message);
  } catch {
    /* Logging cannot change the status assertion. */
  }
  assert.equal(result.status, 0, message);
}
function diagnoseCompiler(owner, deadline) {
  if (!owner.hasResult || owner.metadataCollected) return;
  owner.metadataCollected = true;
  try {
    const metadata = guardFailureMetadata('swift', deadline);
    owner.retain ||= metadata.uncertainCompletion;
    console.log(JSON.stringify({ compilerMetadata: metadata }));
  } catch {
    owner.retain = true;
    try {
      console.log('Compiler metadata unavailable; descendant cleanup unconfirmed');
    } catch {
      /* Keep the primary error. */
    }
  }
}
async function cleanupCompiler(directory, owner) {
  if (owner.retain) {
    try {
      console.log(
        JSON.stringify({
          temporaryProofRetained: true,
          directory: basename(directory),
          compilerDescendants: 'unconfirmed',
          runnerTeardownVerified: false,
        }),
      );
    } catch {
      /* Keep the primary error. */
    }
  } else await rm(directory, { recursive: true, force: true });
}
// END COMPILER DIAGNOSTICS

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
          fileURLToPath(new URL('./hosted-launch.swift', import.meta.url)),
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
