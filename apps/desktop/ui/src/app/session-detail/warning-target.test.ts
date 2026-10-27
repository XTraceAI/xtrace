import { expect, it } from 'vitest';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import type { SourceBlock } from '../../data/generated/SourceBlock';
import type { TranscriptRead } from './useSessionTranscript';
import { resolveWarningJump, warningUnavailableText } from './timeline-jump';

const text: SourceBlock = { kind: 'text', index: 0, text: 'Original words' };
const result = (index: number): SourceBlock => ({
  kind: 'tool_result',
  index,
  call_id: 'saved-call',
  failed: true,
  parts: [{ kind: 'text', text: 'Original result' }],
});
const record = (
  role: 'user' | 'assistant',
  blocks: SourceBlock[] = [text],
  id = 'saved',
): SourceRecord => ({ id, role, at: null, blocks });
const read = (records: SourceRecord[]): TranscriptRead => ({
  phase: 'read',
  status: {
    state: 'loaded',
    generation: { generation: 'unrecorded' },
    sources: 1,
    record_ranges: [{ start: 0, end: records.length }],
    records,
    dropped_records: 0,
    gaps: [],
  },
});

it('requires a unique user-role message when no block was saved', () => {
  const point = { record_uuid: 'saved', block: null };
  expect(resolveWarningJump(point, read([record('user')]))).toEqual({
    kind: 'target',
    target: { kind: 'record', recordId: 'saved' },
  });
  expect(resolveWarningJump(point, read([record('assistant')]))).toEqual({
    kind: 'unavailable',
    reason: 'not_a_user_message',
  });
  expect(resolveWarningJump(point, read([record('user'), record('assistant')]))).toEqual({
    kind: 'unavailable',
    reason: 'ambiguous',
  });
});
it.each([0, 128])('keeps the exact result block index %i', (block) => {
  expect(
    resolveWarningJump(
      { record_uuid: 'saved', block },
      read([record('user', [{ ...text, index: block === 0 ? 1 : 0 }, result(block)])]),
    ),
  ).toEqual({ kind: 'target', target: { kind: 'block', blockId: `saved:${block}` } });
});
it('accepts an exact tool call but refuses another kind, absent block and repeated block index', () => {
  const point = { record_uuid: 'saved', block: 0 };
  expect(
    resolveWarningJump(
      point,
      read([
        record('assistant', [
          { kind: 'tool_call', index: 0, name: 'Read', call_id: 'call', input: null },
        ]),
      ]),
    ),
  ).toEqual({ kind: 'target', target: { kind: 'block', blockId: 'saved:0' } });
  for (const blocks of [[text], [], [result(0), result(0)]])
    expect(resolveWarningJump(point, read([record('user', blocks)])).kind).toBe('unavailable');
});
it('refuses duplicate record IDs even when only one contains the selected block', () => {
  expect(
    resolveWarningJump(
      { record_uuid: 'saved', block: 128 },
      read([record('user', [result(128)]), record('assistant')]),
    ),
  ).toEqual({ kind: 'unavailable', reason: 'ambiguous' });
});
it('compares the saved message ID exactly and never substitutes a nearby message', () => {
  expect(
    resolveWarningJump({ record_uuid: ' saved ', block: null }, read([record('user')])),
  ).toEqual({ kind: 'unavailable', reason: 'not_in_transcript' });
  expect(
    resolveWarningJump({ record_uuid: 'missing', block: null }, read([record('user')])),
  ).toEqual({ kind: 'unavailable', reason: 'not_in_transcript' });
});
it.each<TranscriptRead>([
  { phase: 'loading' },
  { phase: 'failed' },
  { phase: 'read', status: { state: 'unavailable', reason: { reason: 'replaced' } } },
])('returns a closed refusal while the source cannot confirm a target: %j', (state) => {
  const answer = resolveWarningJump({ record_uuid: 'saved', block: null }, state);
  expect(answer.kind).toBe('unavailable');
  if (answer.kind === 'unavailable')
    expect(warningUnavailableText(answer.reason)).not.toMatch(/activity period|earliest|Showing/);
});
