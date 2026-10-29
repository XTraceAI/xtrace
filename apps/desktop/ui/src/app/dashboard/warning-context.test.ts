import { expect, it } from 'vitest';
import { warningExcerpt } from './warning-context';
import { contextFixture, contextExcerpt } from './warning-context.synthetic';

it('renders the bounded native excerpt with one trigger', () => {
  const status = contextExcerpt(contextFixture('target'), 'target');
  const view = warningExcerpt({ phase: 'read', status });
  expect(view.kind).toBe('excerpt');
  if (view.kind !== 'excerpt') return;
  expect(view.records.map(({ record, target }) => [record.id, target])).toEqual([
    ['before', false],
    ['target', true],
    ['after', false],
  ]);
});
it('passes on the native lead and its gap without working them out', () => {
  // far-before(0), before(1), target(2): the lead is directly before them.
  const adjacent = warningExcerpt({
    phase: 'read',
    status: contextExcerpt(contextFixture('target'), 'target', null, [], 'far-before'),
  });
  if (adjacent.kind !== 'excerpt') throw new Error('excerpt');
  expect(adjacent.hasLead).toBe(true);
  expect(adjacent.records.map(({ record, gapAfter }) => [record.id, gapAfter])).toEqual([
    ['far-before', false],
    ['before', false],
    ['target', false],
    ['after', false],
  ]);
  // A lead already beside the target is not repeated, and no lead is kept.
  const beside = warningExcerpt({
    phase: 'read',
    status: contextExcerpt(contextFixture('target'), 'target', null, [], 'before'),
  });
  if (beside.kind !== 'excerpt') throw new Error('excerpt');
  expect(beside.hasLead).toBe(false);
  expect(beside.records).toHaveLength(3);
  // With a step between, the native gap flag is shown as given.
  const status = contextExcerpt(contextFixture('target'), 'target', null, [], 'far-before');
  if (status.state !== 'loaded') throw new Error('loaded');
  status.records[0]!.gap_after = true;
  const gap = warningExcerpt({ phase: 'read', status });
  if (gap.kind !== 'excerpt') throw new Error('excerpt');
  expect(gap.records.map(({ gapAfter }) => gapAfter)).toEqual([true, false, false, false]);
});
it('does not infer call results or their tool names from an excerpt', () => {
  const status = contextFixture('target');
  status.records[2]!.blocks = [
    { kind: 'tool_call', index: 150, name: 'Read', call_id: 'call', input: null },
  ];
  status.records[3]!.blocks = [
    {
      kind: 'tool_result',
      index: 0,
      call_id: 'call',
      failed: true,
      parts: [{ kind: 'text', text: 'failure' }],
    },
  ];
  const view = warningExcerpt({ phase: 'read', status: contextExcerpt(status, 'target', 150) });
  if (view.kind !== 'excerpt') throw new Error('excerpt missing');
  expect(view.records[1]!.record.blocks[0]).toMatchObject({
    kind: 'tool_call',
    id: 'target:150',
    outcome: { state: 'not_checked' },
  });
  expect(view.records[2]!.record.blocks[0]).toMatchObject({
    kind: 'tool_result',
    name: null,
    outcome: { state: 'failed', output: { text: 'failure' } },
  });
});
it('preserves the human caption after filtering a mixed message to its result', () => {
  const status = contextFixture('target');
  status.records[2]!.blocks.push({
    kind: 'tool_result',
    index: 99,
    call_id: 'call',
    failed: true,
    parts: [],
  });
  const view = warningExcerpt({ phase: 'read', status: contextExcerpt(status, 'target', 99) });
  if (view.kind !== 'excerpt') throw new Error('excerpt missing');
  expect(view.records[1]!.record.role).toBe('user');
  expect(view.records[1]!.omitted).toBe(true);
});
for (const reason of ['target_ambiguous', 'incomplete', 'deadline'] as const)
  it(`does not return records for ${reason}`, () => {
    expect(
      warningExcerpt({ phase: 'read', status: { state: 'unavailable', reason: { reason } } }).kind,
    ).toBe('unavailable');
  });
it('explains oversized saved messages and output honestly', () => {
  for (const limit of ['line_bytes', 'output_bytes'] as const) {
    const view = warningExcerpt({
      phase: 'read',
      status: { state: 'unavailable', reason: { reason: 'limit', limit } },
    });
    expect(view).toMatchObject({
      kind: 'unavailable',
      reason: expect.stringContaining('too large'),
    });
  }
});

it('keeps unsupported history truthful in the Dashboard warning excerpt', () => {
  const view = warningExcerpt({
    phase: 'read',
    status: {
      state: 'unavailable',
      reason: {
        reason: 'source',
        source: { reason: 'unsupported_history' },
      },
    },
  });
  expect(view).toMatchObject({
    kind: 'unavailable',
    reason: expect.stringContaining('history format'),
  });
  if (view.kind !== 'unavailable') throw new Error('expected unavailable context');
  expect(view.reason).not.toMatch(/Cursor|database/i);
});

for (const reason of ['target_missing', 'block_missing'] as const)
  it(`keeps ${reason} unavailable instead of choosing a neighboring message`, () => {
    expect(
      warningExcerpt({ phase: 'read', status: { state: 'unavailable', reason: { reason } } }),
    ).toEqual({
      kind: 'unavailable',
      reason: 'The message or tool block saved for this issue is not in the original conversation.',
    });
  });

it('keeps the native record bounds without searching or adding neighbors in the UI', () => {
  const status = contextExcerpt(contextFixture('target'), 'target');
  if (status.state !== 'loaded') throw Error('Missing fixture');
  status.records = status.records.slice(1, 2);
  const view = warningExcerpt({ phase: 'read', status });
  if (view.kind !== 'excerpt') throw Error('Missing excerpt');
  expect(view.records.map(({ record }) => record.id)).toEqual(['target']);
});
