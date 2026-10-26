import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { runInNewContext } from 'node:vm';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkNative } from './native-check.mjs';

const source = await readFile(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
const metadata = await readFile(
  new URL('../../.github/workflows/pr-metadata.yml', import.meta.url),
  'utf8',
);
const jobsOf = (workflow) =>
  Object.fromEntries(
    [
      ...workflow
        .split('\njobs:\n')[1]
        .matchAll(/^ {2}([a-z0-9-]+):\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n|$(?![\s\S]))/gm),
    ].map(([, name, body]) => [name, body]),
  );
const release = await readFile(
  new URL('../../.github/workflows/release-native.yml', import.meta.url),
  'utf8',
);
const jobs = jobsOf(source);
const releaseJobs = jobsOf(release);
const editedJobs = jobsOf(metadata);

test('candidate CI jobs have no metadata credentials or grants', () => {
  assert.match(source, /^permissions:\n {2}contents: read\n/m);
  assert.doesNotMatch(
    source.split('\njobs:\n')[0],
    /\b(?:token|secrets)\b|^\s+(?:pull-requests|issues|checks):/im,
  );
  for (const [name, job] of Object.entries(jobs)) {
    if (name === 'policy') continue;
    assert.doesNotMatch(job, /\b(?:GITHUB_TOKEN|GH_TOKEN)\b/i, name);
    assert.doesNotMatch(job, /\$\{\{[^}]*\b(?:token|secrets)\b/i, name);
    assert.doesNotMatch(job, /^\s+(?:permissions|issues|pull-requests|checks|actions):/m, name);
    for (const checkout of job.split(/- uses: actions\/checkout@/).slice(1)) {
      assert.match(checkout.split(/\n\s+- (?:uses|name|run):/)[0], /persist-credentials: false/);
    }
  }
  assert.doesNotMatch(jobs.security, /publication:check/);
  assert.match(
    jobs.security,
    /scan-secrets\.mjs --diff "\$PUBLICATION_BASE\.\.\$PUBLICATION_HEAD"/,
  );
});

test('routine CI is Linux-only and native work is explicit release preparation', () => {
  assert.deepEqual(Object.keys(jobs).sort(), [
    'check',
    'ci-ok',
    'policy',
    'security',
    'selection',
    'ui',
    'ui-extras',
  ]);
  for (const [name, job] of Object.entries(jobs)) assert.match(job, /runs-on: ubuntu-24\.04/, name);
  assert.match(editedJobs.policy, /runs-on: ubuntu-24\.04/);
  assert.deepEqual(
    [
      ...release
        .split('\non:\n')[1]
        .split('\npermissions:')[0]
        .matchAll(/^ {2}(\w+):/gm),
    ].map((x) => x[1]),
    ['workflow_dispatch'],
  );
  assert.match(release, /runs-on: macos-14/);
  assert.match(release, /test "\$\(uname -m\)" = arm64/);
  assert.match(release, /sw_vers -productVersion/);
  assert.match(release, /CANDIDATE_SHA.*\^\[0-9a-f\]\{40\}\$/);
  assert.match(release, /test "\$WORKFLOW_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/);
  assert.match(release, /git merge-base --is-ancestor "\$CANDIDATE_SHA"/);
  assert.match(release, /ref: \$\{\{ inputs\.candidate_sha \}\}/);
  assert.match(release, /REVIEWED_BASE_SHA="\$\(git rev-parse "\$CANDIDATE_SHA\^1"\)"/);
  assert.match(release, /pnpm check:native --base "\$REVIEWED_BASE_SHA" --release/);
  assert.doesNotMatch(
    release,
    /GITHUB_TOKEN|GH_TOKEN|contents: write|gh release|permissions:\n(?! {2}contents: read\n)/,
  );
  assert.doesNotMatch(releaseJobs.native, /secrets\.|environment:/);
  assert.match(release, /persist-credentials: false/);
  assert.deepEqual(Object.keys(jobsOf(release)), [
    'validate-mode',
    'native',
    'package',
    'ui-capability-probe',
    'updater-failures',
    'signed-first-launch',
    'signed-update',
  ]);
  assert.match(release, /^permissions:\n {2}contents: read\n\n/m);
  assert.doesNotMatch(jobsOf(release).native, /^\s+permissions:/m);
  for (const action of release.matchAll(/uses: (\S+)/g)) assert.match(action[1], /@[a-f0-9]{40}$/);
  for (const checkout of release.split(/uses: actions\/checkout@/).slice(1))
    assert.match(checkout.split(/\n {6}- /)[0], /persist-credentials: false/);
  for (const job of [jobs.ui, jobs['ui-extras']]) {
    assert.match(job, /path: ~\/\.cache\/ms-playwright/);
    assert.match(job, /playwright install --with-deps webkit/);
  }
});

test('routine CI checks once and covers every browser gate across four independent WebKit shards', () => {
  const commands = (job) => [...job.matchAll(/^ {6}- run: (.+)$/gm)].map((match) => match[1]);
  assert.deepEqual(
    Object.entries(jobs).flatMap(([name, job]) =>
      commands(job)
        .filter((command) => command === 'pnpm check')
        .map(() => name),
    ),
    ['check'],
  );
  const shards = jobs.ui
    .match(/^ {8}shard: \[([^\]]+)\]/m)[1]
    .split(',')
    .map(Number);
  assert.deepEqual(shards, [1, 2, 3, 4]);
  assert.match(jobs.ui, /^ {6}fail-fast: false$/m);
  assert.doesNotMatch(jobs.ui, /max-parallel: [1-3]$/m);
  const expanded = shards.flatMap((shard) =>
    commands(jobs.ui).map((command) => command.replace('${{ matrix.shard }}', shard)),
  );
  assert.deepEqual(
    expanded.filter((command) => command.startsWith('pnpm e2e')),
    shards.map((shard) => `pnpm e2e --project=webkit --shard=${shard}/${shards.length}`),
  );
  assert.deepEqual(
    commands(jobs['ui-extras']).filter((command) => command.startsWith('pnpm e2e')),
    ['pnpm e2e:gallery:smoke --project=webkit', 'pnpm e2e:production --project=webkit'],
  );
  for (const name of ['check', 'ui', 'ui-extras']) {
    const job = jobs[name];
    // Each browser test can use up to three minutes. Setup and the remaining
    // tests need time too; the job must not expire at an individual test limit.
    assert.ok(Number(job.match(/timeout-minutes: (\d+)/)[1]) > 3, name);
    assert.doesNotMatch(job, /continue-on-error|--(?:grep|timeout|retries|max-failures)/m);
    if (name === 'check') assert.doesNotMatch(job, /^ {4}(?:if|needs):/m);
  }
  assert.doesNotMatch(jobs.ui + jobs['ui-extras'], /chromium|matrix\.browser/);
  assert.match(jobs['ci-ok'], /if: \$\{\{ always\(\) \}\}/);
  assert.doesNotMatch(jobs['ci-ok'], /continue-on-error/);
});

test('only browser jobs depend on a dependency-free, credential-free selector', () => {
  assert.match(jobs.selection, /fetch-depth: 0/);
  assert.match(jobs.selection, /run_ui: \$\{\{ steps\.select\.outputs\.run_ui \}\}/);
  assert.match(jobs.selection, /- id: select\n {8}run: node scripts\/ci\/select-ui\.mjs/);
  assert.doesNotMatch(jobs.selection, /pnpm|install|cache:|GITHUB_TOKEN|GH_TOKEN/);
  for (const name of ['ui', 'ui-extras']) {
    assert.match(jobs[name], /^ {4}needs: selection$/m);
    assert.match(jobs[name], /^ {4}if: \$\{\{ needs\.selection\.outputs\.run_ui == 'true' \}\}$/m);
  }
  for (const name of ['policy', 'check', 'security', 'selection'])
    assert.doesNotMatch(jobs[name], /^ {4}(?:if|needs):/m);
});

test('actual release shell guards accept only default history or matching reviewed tags', async (t) => {
  const steps = release.split(/\n {6}- /);
  const guardSteps = [
    steps.find((step) => step.startsWith('name: Require an explicit candidate')),
    steps.find((step) => step.startsWith('name: Verify candidate identity')),
  ];
  for (const step of guardSteps) {
    for (const [name, expression] of Object.entries({
      CANDIDATE_SHA: 'inputs.candidate_sha',
      WORKFLOW_REF: 'github.ref',
      WORKFLOW_SHA: 'github.workflow_sha',
      SOURCE_SHA: 'github.sha',
      DEFAULT_BRANCH: 'github.event.repository.default_branch',
    }))
      assert.ok(step.includes(`${name}: \${{ ${expression} }}`), name);
  }
  assert.ok(guardSteps[0].includes('WORKFLOW_REF_TYPE: ${{ github.ref_type }}'));
  const guards = guardSteps.map((step) =>
    step.split('        run: |\n')[1].replace(/^ {10}/gm, ''),
  );
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-release-guard-'));
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
    const result = spawnSync('git', args, { cwd: repo, env, input: '', encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  // Only the OS probes are controlled: both scripts and all Git checks run as written.
  const runGuard = (index, values) =>
    spawnSync(
      'bash',
      [
        '--noprofile',
        '--norc',
        '-e',
        '-o',
        'pipefail',
        '-c',
        'uname() { echo "${TEST_MACHINE:-arm64}"; }; sw_vers() { echo "${TEST_MACOS:-14.7}"; };\n' +
          guards[index],
      ],
      { cwd: repo, env: { ...env, ...values }, encoding: 'utf8' },
    );
  try {
    git('init', '--template=', '--initial-branch=main');
    const tree = git('mktree');
    const base = git('commit-tree', tree, '-m', 'Synthetic baseline');
    const candidate = git('commit-tree', tree, '-p', base, '-m', 'Synthetic candidate');
    const descendant = git('commit-tree', tree, '-p', candidate, '-m', 'Synthetic later commit');
    git('update-ref', 'HEAD', candidate);
    git('update-ref', 'refs/remotes/origin/main', descendant);
    git('tag', 'v0.1.1', candidate);
    git('tag', '-a', 'v0.1.1-beta.1+qa.14', candidate, '-m', 'Synthetic annotated beta');
    const defaults = {
      CANDIDATE_SHA: candidate,
      WORKFLOW_REF: 'refs/heads/main',
      WORKFLOW_REF_TYPE: 'branch',
      WORKFLOW_SHA: descendant,
      SOURCE_SHA: descendant,
      DEFAULT_BRANCH: 'main',
    };
    const tagged = {
      ...defaults,
      WORKFLOW_REF: 'refs/tags/v0.1.1',
      WORKFLOW_REF_TYPE: 'tag',
      WORKFLOW_SHA: candidate,
      SOURCE_SHA: candidate,
    };
    const check = async (name, values, accepted, rejectedAt = 0) => {
      await t.test(name, () => {
        const results = [runGuard(0, values)];
        if (results[0].status === 0) results.push(runGuard(1, values));
        assert.equal(
          results.every((result) => result.status === 0),
          accepted,
          results.map((result) => result.stderr).join('\n'),
        );
        if (!accepted) assert.notEqual(results[rejectedAt]?.status, 0);
        if (!accepted && rejectedAt === 1) assert.equal(results[0].status, 0);
      });
    };
    await check(
      'default branch accepts an ancestor, without requiring workflow SHA equality',
      defaults,
      true,
    );
    git('update-ref', 'refs/remotes/origin/main', base);
    await check('default branch rejects a candidate outside main history', defaults, false, 1);
    await check('tag accepts identical commits outside main history', tagged, true);
    await check(
      'annotated SemVer beta resolves to the candidate commit',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v0.1.1-beta.1+qa.14' },
      true,
    );
    for (const [field, value] of [
      ['WORKFLOW_REF', 'refs/heads/feature'],
      ['WORKFLOW_REF', 'refs/heads/v0.1.1'],
      ['WORKFLOW_REF', 'refs/pull/1/head'],
      ['WORKFLOW_REF', 'v0.1.1'],
      ['WORKFLOW_REF_TYPE', 'branch'],
      ['WORKFLOW_REF_TYPE', ''],
      ['WORKFLOW_SHA', base],
      ['WORKFLOW_SHA', ''],
      ['SOURCE_SHA', base],
      ['SOURCE_SHA', 'malformed'],
      ['CANDIDATE_SHA', base],
      ['CANDIDATE_SHA', candidate.slice(0, 7)],
      ['CANDIDATE_SHA', 'A'.repeat(40)],
      ['CANDIDATE_SHA', ''],
      ['CANDIDATE_SHA', `${candidate}\n`],
      ['CANDIDATE_SHA', `${candidate}; echo unexpected`],
    ])
      await check(`tag rejects ${field}=${value}`, { ...tagged, [field]: value }, false);
    for (const version of [
      '0.1',
      '01.1.1',
      '0.01.1',
      '0.1.01',
      '0.1.1-01',
      '0.1.1-',
      '0.1.1+',
      '0.1.1-beta..1',
      '0.1.1+qa..14',
      '0.1.1/extra',
      '0.1.1^{commit}',
      '0.1.1; echo unexpected',
      '0.1.1\n',
    ])
      await check(
        `tag rejects malformed v${version}`,
        { ...tagged, WORKFLOW_REF: `refs/tags/v${version}` },
        false,
      );
    await check(
      'default branch rejects a tag ref type',
      { ...defaults, WORKFLOW_REF_TYPE: 'tag' },
      false,
    );
    await check(
      'missing tag rejects even with matching event identities',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v9.9.9' },
      false,
      1,
    );
    git('tag', 'v0.1.2', tree);
    await check(
      'a tag pointing to a tree rejects',
      { ...tagged, WORKFLOW_REF: 'refs/tags/v0.1.2' },
      false,
      1,
    );
    await check(
      'macOS 15 rejects despite identical source commits',
      { ...tagged, TEST_MACOS: '15.7' },
      false,
      1,
    );
    await check(
      'x86_64 rejects despite identical source commits',
      { ...tagged, TEST_MACHINE: 'x86_64' },
      false,
      1,
    );
    git('update-ref', 'HEAD', base);
    await check('tag rejects a different checked-out HEAD', tagged, false, 1);
    await check(
      'tag rejects a different resolved commit with all other identities matching',
      { ...tagged, CANDIDATE_SHA: base, WORKFLOW_SHA: base, SOURCE_SHA: base },
      false,
      1,
    );
    git('update-ref', 'HEAD', candidate);
    await t.test('tag moved after the first guard rejects at the Git identity guard', () => {
      assert.equal(runGuard(0, tagged).status, 0);
      git('update-ref', 'refs/tags/v0.1.1', base);
      assert.notEqual(runGuard(1, tagged).status, 0);
    });
  } finally {
    await rm(repo, { recursive: true, force: true });
  }
});

test('release validation explicitly uses verified vendored producer conformance', async () => {
  const pin = JSON.parse(
    await readFile(new URL('../../.plugin-pin', import.meta.url), 'utf8'),
  ).commit;
  assert.match(pin, /^[a-f0-9]{40}$/);
  const steps = release.split(/\n {6}- /);
  const validation = steps.findIndex((step) => step.includes('pnpm check:native'));
  assert.ok(validation > 0);
  assert.ok(!release.includes('repository: XTraceAI/agent-plugins'));
  assert.match(steps[validation], /AGENT_PLUGINS_SOURCE: bundle/);
  assert.ok(
    steps[validation].includes(
      'AGENT_PLUGINS_DIR: ${{ github.workspace }}/vendor/agent-plugins/plugins/memhub',
    ),
  );
  const gate = await readFile(new URL('./plugin-conformance.sh', import.meta.url), 'utf8');
  assert.match(gate, /AGENT_PLUGINS_SOURCE:-checkout/);
  assert.match(gate, /--example conformance_bundle --locked/);
  assert.match(gate, /from bundle_source import verify_bundle/);
  assert.match(gate, /verifyCheckout\(pin, git\)/);
  assert.match(gate, /assert-no-skipped-conformance\.sh/);
});

test('release validation warns about stale plugin readers before validating, without failing', async () => {
  const steps = releaseJobs.native.split(/\n {6}- /);
  const freshness = steps.findIndex((step) => step.includes('pnpm check:plugin-pin'));
  const validation = steps.findIndex((step) => step.includes('pnpm check:native'));
  assert.ok(freshness > 0 && validation === freshness + 1);
  assert.doesNotMatch(steps[freshness], /continue-on-error|\|\| true|env:|secrets\.|token/i);
  const pkg = JSON.parse(await readFile(new URL('../../package.json', import.meta.url), 'utf8'));
  assert.equal(pkg.scripts['check:plugin-pin'], 'node scripts/ci/plugin-pin.mjs freshness');
  // Run the step's own shell as GitHub does, with a stand-in pnpm, against a
  // candidate that has the script and one that predates it.
  const guard = steps[freshness].split('        run: |\n')[1].replace(/^ {10}/gm, '');
  const directory = await mkdtemp(join(tmpdir(), 'xtrace-reader-guard-'));
  try {
    const summary = join(directory, 'summary.md');
    const runStep = async (scripts, pnpmStatus = 0) => {
      await writeFile(join(directory, 'package.json'), JSON.stringify({ scripts }, null, 2));
      await writeFile(summary, '');
      const result = spawnSync(
        'bash',
        [
          '--noprofile',
          '--norc',
          '-e',
          '-o',
          'pipefail',
          '-c',
          `pnpm() { echo "pnpm $*"; return ${pnpmStatus}; }\n${guard}`,
        ],
        {
          cwd: directory,
          env: { PATH: process.env.PATH, GITHUB_STEP_SUMMARY: summary },
          encoding: 'utf8',
        },
      );
      return { ...result, summary: await readFile(summary, 'utf8') };
    };
    const current = await runStep(pkg.scripts);
    assert.equal(current.status, 0, current.stderr);
    assert.equal(current.stdout, 'pnpm check:plugin-pin\n');
    assert.equal(current.summary, '');
    // The checker's own failure, such as a malformed pin, still fails the step.
    assert.equal((await runStep(pkg.scripts, 1)).status, 1);
    const older = await runStep({ 'check:native': 'node scripts/ci/native-check.mjs' });
    assert.equal(older.status, 0, older.stderr);
    const message =
      'This candidate predates the plugin-reader check, so its bundled plugin readers were not checked against the latest memhub release.';
    assert.equal(older.stdout, `::warning title=Plugin readers::${message}\n`);
    assert.equal(older.summary, `- Warning: ${message}\n`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('only the signing step of the packaging job sees Apple values, after validation passes', () => {
  assert.deepEqual(Object.keys(releaseJobs), [
    'validate-mode',
    'native',
    'package',
    'ui-capability-probe',
    'updater-failures',
    'signed-first-launch',
    'signed-update',
  ]);
  const job = releaseJobs.package;
  assert.ok(
    job.includes(
      "    if: (inputs.delivery_mode == 'native' || inputs.delivery_mode == '') && github.ref == format('refs/heads/{0}', github.event.repository.default_branch)",
    ),
    'packaging runs only on the default branch and skips reviewed tag validation',
  );
  assert.match(job, /^ {4}needs: native$/m);
  assert.match(job, /^ {4}environment: macos-signing$/m);
  assert.match(job, /runs-on: macos-14/);
  assert.doesNotMatch(job, /permissions:|continue-on-error|\|\| true/);
  for (const check of [
    /CANDIDATE_SHA.*\^\[0-9a-f\]\{40\}\$/,
    /test "\$WORKFLOW_REF" = "refs\/heads\/\$DEFAULT_BRANCH"/,
    /test "\$\(git rev-parse HEAD\)" = "\$CANDIDATE_SHA"/,
    /git merge-base --is-ancestor "\$CANDIDATE_SHA"/,
    /test "\$\(uname -m\)" = arm64/,
  ])
    assert.match(job, check);
  for (const checkout of job.split(/- uses: actions\/checkout@/).slice(1))
    assert.match(checkout.split(/\n\s+- (?:uses|name|run):/)[0], /persist-credentials: false/);
  const steps = job.split(/\n {6}- /);
  const build = steps.findIndex((step) => step.includes('pnpm tauri build --no-bundle'));
  const sign = steps.findIndex((step) => step.includes('node scripts/ci/release-dmg.mjs'));
  const upload = steps.findIndex((step) => step.includes('actions/upload-artifact@'));
  assert.ok(
    build > 0 && sign === build + 1 && upload === sign + 1,
    'compile, then sign, then upload',
  );
  assert.match(steps[build], /pnpm install --frozen-lockfile --ignore-scripts/);
  steps.forEach((step, index) => {
    if (index !== sign) assert.doesNotMatch(step, /secrets\.|APPLE_/, step.split('\n')[0]);
  });
  assert.deepEqual(
    [...steps[sign].matchAll(/secrets\.(\w+)/g)].map((match) => match[1]),
    [
      'APPLE_CERTIFICATE',
      'APPLE_CERTIFICATE_PASSWORD',
      'APPLE_API_KEY',
      'APPLE_API_ISSUER',
      'APPLE_API_PRIVATE_KEY',
    ],
  );
  assert.match(
    steps[sign],
    /APPLE_SIGNING_IDENTITY: 'Developer ID Application: .+ \([A-Z0-9]{10}\)'/,
  );
  assert.match(steps[upload], /path: artifacts\/release\/\n/);
  assert.match(steps[upload], /if-no-files-found: error/);
});

test('credentialed contribution checks run only reviewed source', () => {
  for (const job of [jobs.policy, editedJobs.policy]) {
    assert.equal(job.match(/actions\/checkout@/g).length, 1);
    assert.match(
      job,
      /ref: \$\{\{ vars\.CI_POLICY_SHA \|\| github\.event\.repository\.default_branch \}\}/,
    );
    assert.match(job, /\^\[0-9a-f\]\{40\}\$/);
    assert.match(job, /persist-credentials: false/);
    assert.doesNotMatch(job, /echo.*CI_POLICY_SHA=|issues: read/);
    assert.doesNotMatch(
      job,
      /pull_request\.head|github\.sha|download-artifact|cache:|test:ci|test:supply-chain/,
    );
    assert.match(job, /pnpm install --frozen-lockfile --ignore-scripts/);
    assert.match(job, /GITHUB_TOKEN: \$\{\{ github\.token \}\}/);
  }
  assert.equal(
    jobs.policy.split('    steps:\n')[1].trimEnd(),
    editedJobs.policy.split('    steps:\n')[1].trimEnd(),
  );
});

test('the actual aggregate rejects every missing, skipped, cancelled or failed workflow dependency', () => {
  const required = Object.keys(jobs).filter((name) => name !== 'ci-ok');
  const declared = jobs['ci-ok'].match(/needs: \[([^\]]+)\]/)[1].split(', ');
  assert.deepEqual(declared.sort(), [...required].sort());
  const good = Object.fromEntries(required.map((name) => [name, { result: 'success' }]));
  good.selection.outputs = { run_ui: 'true' };
  const aggregate = (needs) =>
    spawnSync(process.execPath, [new URL('./aggregate.mjs', import.meta.url).pathname], {
      env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
      encoding: 'utf8',
    });
  assert.equal(aggregate(good).status, 0);
  for (const name of required) {
    const missing = { ...good };
    delete missing[name];
    assert.equal(aggregate(missing).status, 1, `missing ${name}`);
    // GitHub exposes the combined four-shard outcome as needs.ui.result.
    for (const result of ['skipped', 'cancelled', 'failure', 'in_progress', undefined]) {
      assert.equal(aggregate({ ...good, [name]: { result } }).status, 1, `${name}: ${result}`);
    }
  }
  const noUi = {
    ...good,
    selection: { result: 'success', outputs: { run_ui: 'false' } },
    ui: { result: 'skipped' },
    'ui-extras': { result: 'skipped' },
  };
  assert.equal(aggregate(noUi).status, 0);
  for (const name of required) {
    const missing = { ...noUi };
    delete missing[name];
    assert.equal(aggregate(missing).status, 1, `unselected missing ${name}`);
    for (const result of ['failure', 'cancelled', 'skipped', undefined]) {
      if (['ui', 'ui-extras'].includes(name) && result === 'skipped') continue;
      assert.equal(
        aggregate({ ...noUi, [name]: { ...noUi[name], result } }).status,
        1,
        `unselected ${name}: ${result}`,
      );
    }
  }
  for (const run_ui of [undefined, '', 'TRUE', 'False', true, false, 'false\n'])
    assert.equal(
      aggregate({ ...noUi, selection: { result: 'success', outputs: { run_ui } } }).status,
      1,
    );
  assert.equal(aggregate({ ...noUi, selection: { result: 'success' } }).status, 1);
});

test('text edits run metadata only; target-branch edits call the full CI and cannot cancel it', () => {
  assert.match(source, /on:\n {2}workflow_call:\n/);
  assert.match(source, /types: \[opened, reopened, synchronize, ready_for_review\]/);
  assert.doesNotMatch(source, /\bedited\b/);
  assert.match(metadata, /pull_request:\n {4}types: \[edited\]/);
  assert.match(editedJobs['base-ci'], /uses: \.\/\.github\/workflows\/ci\.yml/);
  assert.deepEqual(Object.keys(editedJobs).sort(), ['base-ci', 'policy']);
  assert.doesNotMatch(metadata, /name: ci-ok|aggregate\.mjs|needs:/);
  const condition = (job, changes) => {
    const expression = job.match(/if: \$\{\{ (.+) \}\}/)[1];
    return Boolean(runInNewContext(expression, { github: { event: { changes } } }));
  };
  for (const changes of [
    {},
    { body: { from: 'previous text' } },
    { title: { from: 'old title' } },
  ]) {
    assert.equal(condition(editedJobs['base-ci'], changes), false);
    assert.equal(condition(editedJobs.policy, changes), true);
  }
  for (const changes of [{ base: { ref: { from: 'previous-base' } } }, { base: {}, body: {} }]) {
    assert.equal(condition(editedJobs['base-ci'], changes), true);
    assert.equal(condition(editedJobs.policy, changes), false);
  }
  // The base-triggered reusable CI and text-only workflow have distinct groups.
  assert.match(metadata, /changes\.base && 'base' \|\| 'text'/);
  assert.match(metadata, /group: pr-metadata-/);
  assert.match(source, /group: ci-/);
});

test('required hosted gates retain their commands and pinned actions without accepting failures', () => {
  for (const command of [
    'pnpm test:ci',
    'pnpm test:supply-chain',
    'pnpm publication:test',
    'scan-secrets.mjs --diff',
  ])
    assert.ok(jobs.security.includes(command), command);
  for (const action of source.matchAll(/uses: (\S+)/g)) assert.match(action[1], /@[a-f0-9]{40}$/);
  assert.doesNotMatch(source + metadata, /continue-on-error|\|\| true/);
});

test('native validation runs every named gate and fails before subsequent work when one fails', async () => {
  const gates = [
    ['cargo', 'fmt', '--all', '--', '--check'],
    [
      'cargo',
      'clippy',
      '--workspace',
      '--all-targets',
      '--all-features',
      '--locked',
      '--',
      '-D',
      'warnings',
    ],
    ['cargo', 'test', '--workspace', '--all-features', '--locked'],
    ['node', 'scripts/ci/run-hook.mjs', 'plugin-conformance'],
    ['node', 'scripts/ci/run-hook.mjs', 'dto'],
    ['pnpm', 'sbom'],
    ['pnpm', 'notices:check'],
    ['node', 'scripts/security/scan-secrets.mjs', '--content', 'artifacts/sbom.cdx.json'],
  ];
  const invoke = async (failedGate) => {
    const calls = [];
    const trustedRoots = [];
    const run = (command, args, { env }) => {
      const call = [command, ...args];
      calls.push(call);
      if (env.CI_TRUSTED_ROOT) trustedRoots.push(env.CI_TRUSTED_ROOT);
      assert.equal(env.GH_TOKEN, '');
      assert.equal(env.GITHUB_TOKEN, '');
      const status = JSON.stringify(call) === JSON.stringify(failedGate) ? 1 : 0;
      const stdout =
        command === 'git' && args[0] === 'rev-parse'
          ? 'b'.repeat(40)
          : command === 'sw_vers'
            ? '14.7'
            : '';
      return { status, stdout };
    };
    const result = checkNative({
      base: 'a'.repeat(40),
      platform: 'darwin',
      run,
      repo: new URL('../../', import.meta.url).pathname,
      environment: { GH_TOKEN: 'withheld', GITHUB_TOKEN: 'withheld' },
      install: async () => [],
    });
    if (failedGate) await assert.rejects(result, /Native validation stopped/);
    else await result;
    assert.ok(trustedRoots.length > 0);
    assert.equal(new Set(trustedRoots).size, 1);
    await assert.rejects(access(trustedRoots[0]), { code: 'ENOENT' });
    return calls;
  };
  const calls = await invoke();
  const positions = gates.map((gate) =>
    calls.findIndex((call) => JSON.stringify(call) === JSON.stringify(gate)),
  );
  assert.ok(
    positions.every(
      (position, index) => position >= 0 && (index === 0 || position > positions[index - 1]),
    ),
  );
  assert.ok(
    calls.some(
      ([command, ...args]) => command === 'pnpm' && args[0] === 'tauri' && args.includes('--debug'),
    ),
  );
  for (const [index, gate] of gates.entries()) {
    const failed = await invoke(gate);
    assert.deepEqual(failed.at(-1), gate);
    const reached = failed.filter((call) =>
      gates.some((named) => JSON.stringify(named) === JSON.stringify(call)),
    );
    assert.deepEqual(reached, gates.slice(0, index + 1));
  }
});
