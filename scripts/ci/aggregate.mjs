import { requireSuccessfulJobs } from './policy.mjs';

try {
  const results = JSON.parse(process.env.NEEDS_JSON);
  const runUi = results?.selection?.outputs?.run_ui;
  if (!['true', 'false'].includes(runUi)) throw new Error('Invalid UI selection');
  requireSuccessfulJobs(
    results,
    ['selection', 'policy', 'check', 'ui', 'ui-extras', 'security'],
    runUi === 'false' ? ['ui', 'ui-extras'] : [],
  );
  console.log('Every required CI job succeeded or was explicitly inapplicable.');
} catch {
  console.error(
    'CI aggregate failed: a required job was missing, skipped, cancelled or unsuccessful.',
  );
  process.exitCode = 1;
}
