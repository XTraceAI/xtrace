import { expect, it } from 'vitest';
import { displayRuleText } from './rule-display';
import { rules, type RuleId } from './rules';

it('removes internal references from all 30 displayed definitions', () => {
  expect(Object.keys(rules)).toHaveLength(30);
  for (const id of Object.keys(rules) as RuleId[]) {
    expect(displayRuleText(id)).not.toMatch(/\b(?:[CMOPRU]-\d+[a-z]?|F\d+)\b/);
  }
});

it('keeps meaningful formulas, technical details and readable references', () => {
  expect(displayRuleText('M-08')).toBe(
    'Agent-to-human ratio = agent minutes ÷ human-in-the-loop minutes over the window. No assumed workday.',
  );
  expect(displayRuleText('M-04')).toContain('First dedupe storage by uuid (storage deduplication)');
  expect(displayRuleText('M-11a')).toContain(
    "Linked-session agent hours per merged PR = Σ active spans over the PR's distinct linked sessions",
  );
  expect(displayRuleText('M-04')).toContain(
    'Keep all content-block/tool rows for assistant turns/environment usage.',
  );
  expect(displayRuleText('C-08')).toContain('64 lowercase hexadecimal SHA-256 characters');
  expect(displayRuleText('C-08')).toContain(
    'the replay and coverage fixture tests partial late-turn receipt=false, full same-data hook coverage=true, and missing-usage receipt followed by native usage enrichment=false.',
  );
  expect(displayRuleText('M-18')).toContain(
    'it is not complete measurement verification (complete capture verification)',
  );
  expect(displayRuleText('R-05')).toContain(
    'retry handling governs retries and metadata-only storage/explicit content purge content persistence.',
  );
  expect(displayRuleText('R-05')).toContain(
    'Adherence trend = fires per 100 opportunities/week; FP rate = judged fp ÷ all judged.',
  );
  expect(displayRuleText('M-06')).toBe(rules['M-06']);
});

it('keeps linked-PR totals token-only and capture verification tied to selected token usage', () => {
  expect(displayRuleText('M-11')).toContain(
    'Linked-session tokens per merged PR = Σ token usage over distinct sessions linked to that PR, within the selected event window;',
  );
  expect(displayRuleText('M-11')).not.toContain('tokens and cost');
  expect(displayRuleText('U-08')).toContain(
    'every selected token usage representative at the exact measurement revision used by that aggregate.',
  );
});
