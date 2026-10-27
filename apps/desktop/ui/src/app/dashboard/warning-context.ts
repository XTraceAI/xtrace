import { excerptRecord, unavailableNote } from '../session-detail/transcript-adapter';
import type { TranscriptRecord, TranscriptState } from '../session-detail/transcript-view';
export type WarningContext =
  | { kind: 'unavailable'; reason: string }
  | {
      kind: 'excerpt';
      records: {
        record: TranscriptRecord;
        target: boolean;
        omitted: boolean;
        /** M-02: a person's message (`true`), or not known to be one. */
        person: boolean;
      }[];
      state: TranscriptState;
      neighborsAvailable: boolean;
    };

export function warningExcerpt(
  read: import('../session-detail/useSessionTranscript').SourceRead<
    import('../../data/generated/SessionExcerptStatus').SessionExcerptStatus
  >,
): WarningContext {
  if (read.phase !== 'read') return { kind: 'unavailable', reason: '' };
  if (read.status.state === 'unavailable') {
    const reason = read.status.reason;
    const note =
      reason.reason === 'source'
        ? unavailableNote(reason.source)
        : reason.reason === 'target_ambiguous'
          ? 'More than one message matches this issue. The original context cannot be identified safely.'
          : reason.reason === 'target_missing' || reason.reason === 'block_missing'
            ? 'The message or tool block saved for this issue is not in the original conversation.'
            : reason.reason === 'deadline'
              ? 'Reading this excerpt took too long. Try again.'
              : reason.reason === 'incomplete'
                ? 'Part of this conversation could not be checked, so the triggering message cannot be identified safely.'
                : reason.limit === 'line_bytes'
                  ? 'One saved message is too large to check safely for this excerpt.'
                  : reason.limit === 'output_bytes'
                    ? 'These messages are too large to show in this excerpt.'
                    : 'This conversation exceeds the amount that can be checked safely for one excerpt.';
    return { kind: 'unavailable', reason: note };
  }
  return {
    kind: 'excerpt',
    state: { kind: 'ready' },
    neighborsAvailable: true,
    records: read.status.records.map((entry) => ({
      record: excerptRecord(entry.record, entry.only_tool_results),
      target: entry.target,
      person: entry.person === true,
      omitted: entry.omitted_blocks,
    })),
  };
}
