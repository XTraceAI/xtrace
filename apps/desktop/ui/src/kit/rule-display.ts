import { ruleText, type RuleId } from './rules';

/** Readable references for display only; the technical registry remains unchanged. */
const referenceNames = {
  'C-02': 'storage deduplication',
  'C-04': 'retry handling',
  'C-08': 'session and capture receipt identity',
  F7: 'the unmeasured-usage fixture',
  F18: 'the replay and coverage fixture',
  F20: 'the surface discovery fixture',
  'M-01': 'event windows',
  'M-02': 'human messages',
  'M-03': 'assistant turns',
  'M-04': 'tokens and cost',
  'M-05': 'active spans',
  'M-07': 'human-in-the-loop',
  'M-09': 'hands-off',
  'M-11': 'linked-session tokens per merged PR',
  'M-12': 'linked-session human messages per merged PR',
  'M-16': 'sessions per day',
  'M-17': 'environment usage',
  'M-18': 'session capture coverage',
  'P-01': 'metadata-only storage',
  'P-02': 'explicit content purge',
  'U-08': 'complete capture verification',
} satisfies Partial<Record<RuleId, string>> &
  Record<'C-02' | 'C-04' | 'F7' | 'F18' | 'F20', string>;

// Match complete known references, including slash-separated references. Do not
// change suffixed IDs, algorithm names, or any report context/user-provided text.
const referencePattern = new RegExp(
  `(?<![\\w-])(?:${Object.keys(referenceNames).join('|')})(?![\\w-])`,
  'g',
);

export function displayRuleText(id: RuleId): string {
  return ruleText(id)
    .replace(/(?<![\w-])M-05 minutes(?![\w-])/g, 'agent minutes')
    .replace(
      referencePattern,
      (reference) => referenceNames[reference as keyof typeof referenceNames],
    );
}
