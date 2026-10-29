import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceRecord } from '../../data/generated/SourceRecord';

/** Invented text only; never copied from a conversation. */
export const contextMessage = (id: string, text: string): SourceRecord => ({
  id,
  role: 'user',
  at: '2026-09-07T15:00:00Z',
  blocks: [{ kind: 'text', index: 0, text }],
});
export function contextFixture(target: string): Extract<SessionSourceStatus, { state: 'loaded' }> {
  return {
    state: 'loaded',
    generation: { generation: 'indexed', appended: false },
    sources: 1,
    record_ranges: [{ start: 0, end: 5 }],
    gaps: [],
    dropped_records: 0,
    records: [
      contextMessage('far-before', 'This unrelated message must stay outside the excerpt.'),
      {
        ...contextMessage(
          'before',
          'The sample build is ready. I changed the timeout to 30 seconds.',
        ),
        role: 'assistant',
      },
      contextMessage(
        target,
        'Still not fixed. The sample build fails at the same step. Please check the retry limit instead of changing the timeout again.',
      ),
      {
        ...contextMessage(
          'after',
          'I will inspect the retry limit and the failing step before making another change.',
        ),
        role: 'assistant',
      },
      contextMessage(
        'far-after',
        'This other unrelated message must also stay outside the excerpt.',
      ),
    ],
  };
}

export function multiSourceContextFixture(
  target: string,
): Extract<SessionSourceStatus, { state: 'loaded' }> {
  const status = contextFixture(target);
  status.records.push(
    contextMessage('subagent-before', 'Synthetic subagent question.'),
    contextMessage('subagent-target', 'Synthetic subagent correction.'),
    contextMessage('subagent-after', 'Synthetic subagent reply.'),
  );
  status.sources = 3;
  status.record_ranges = [
    { start: 0, end: 5 },
    { start: 5, end: 5 },
    { start: 5, end: 8 },
  ];
  return status;
}

/** Native excerpt answers for synthetic UI fixtures only. */
export function contextExcerpt(
  status: Extract<SessionSourceStatus, { state: 'loaded' }>,
  target: string,
  block: number | null = null,
  /**
   * The native correction marks for the target, stated by the fixture: the
   * UI never works them out (`SessionExcerptRecord.correction_marks`).
   */
  marks: import('../../data/generated/TextMark').TextMark[] = [],
  /**
   * The record the native side keeps ahead of the target's neighbours (an
   * admitted mistake's person's message), stated by the fixture as the
   * native rule keeps it: once, earlier in the same source, not already shown.
   */
  lead: string | null = null,
): import('../../data/generated/SessionExcerptStatus').SessionExcerptStatus {
  const matches = status.records.flatMap((record, index) => (record.id === target ? [index] : []));
  if (matches.length !== 1)
    return {
      state: 'unavailable',
      reason: { reason: matches.length ? 'target_ambiguous' : 'target_missing' },
    };
  const position = matches[0]!;
  const ranges = status.record_ranges.filter(
    (range) => range.start <= position && position < range.end,
  );
  if (ranges.length !== 1 || status.gaps.length)
    return { state: 'unavailable', reason: { reason: 'incomplete' } };
  const range = ranges[0]!;
  const start = Math.max(range.start, position - 1);
  const leads =
    lead === null || lead === target
      ? []
      : status.records.flatMap((record, index) => (record.id === lead ? [index] : []));
  const leadAt =
    leads.length === 1 && leads[0]! >= range.start && leads[0]! < start ? leads[0]! : null;
  const shown = [
    ...(leadAt === null ? [] : [leadAt]),
    ...Array.from({ length: Math.min(range.end, position + 2) - start }, (_, n) => start + n),
  ];
  return {
    state: 'loaded',
    generation: status.generation,
    records: shown.map((at) => {
      const source = status.records[at]!;
      const target = at === position;
      const isLead = at === leadAt;
      const blocks =
        target && block !== null
          ? source.blocks.filter((part) => part.index === block)
          : source.blocks.slice(0, 3);
      return {
        record: { ...source, blocks },
        target,
        omitted_blocks: blocks.length < source.blocks.length,
        only_tool_results:
          source.blocks.length > 0 && source.blocks.every((part) => part.kind === 'tool_result'),
        // Synthetic M-02 answers: the main conversation's user-role messages
        // are a person's; another source's (a sub-agent's) are not.
        person:
          source.role !== 'user'
            ? null
            : at >= status.record_ranges[0]!.start && at < status.record_ranges[0]!.end,
        correction_marks: target ? marks : [],
        lead: isLead,
        gap_after: isLead && at + 1 < start,
      };
    }),
  };
}
