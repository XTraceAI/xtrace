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
        /** The native excerpt left out records between this and the next. */
        gapAfter: boolean;
      }[];
      state: TranscriptState;
      neighborsAvailable: boolean;
      /** The native excerpt kept an earlier message ahead of the target's neighbours. */
      hasLead: boolean;
    };

/** One read of a warning's original messages. */
export type WarningContextRead = import('../session-detail/useSessionTranscript').SourceRead<
  import('../../data/generated/SessionExcerptStatus').SessionExcerptStatus
>;

/** A stretch of a correction's message: marked where its words made it one. */
export type QuotePart = { readonly text: string; readonly marked: boolean };

/**
 * A correction's whole message as the excerpt read it, split at the native
 * marks (`SessionExcerptRecord.correction_marks`, UTF-16 offsets into each
 * text block): its text blocks, in order, one per line. `null` until the
 * excerpt is read, and when it is unavailable or marks nothing, so the
 * warning's own reason is shown instead. Nothing here looks for words.
 */
export function correctionQuote(read: WarningContextRead): QuotePart[][] | null {
  if (read.phase !== 'read' || read.status.state !== 'loaded') return null;
  const target = read.status.records.find((entry) => entry.target);
  if (!target || target.correction_marks.length === 0) return null;
  const quote = target.record.blocks.flatMap((block) => {
    if (block.kind !== 'text') return [];
    const marks = target.correction_marks
      .filter((mark) => mark.block === block.index)
      .sort((a, b) => a.start - b.start);
    const parts: QuotePart[] = [];
    let at = 0;
    for (const mark of marks) {
      if (mark.start < at || mark.end > block.text.length || mark.end <= mark.start) continue;
      if (mark.start > at) parts.push({ text: block.text.slice(at, mark.start), marked: false });
      parts.push({ text: block.text.slice(mark.start, mark.end), marked: true });
      at = mark.end;
    }
    if (at < block.text.length) parts.push({ text: block.text.slice(at), marked: false });
    return [parts];
  });
  return quote.some((parts) => parts.some((part) => part.marked)) ? quote : null;
}

export function warningExcerpt(read: WarningContextRead): WarningContext {
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
    hasLead: read.status.records.some((entry) => entry.lead),
    records: read.status.records.map((entry) => ({
      record: excerptRecord(entry.record, entry.only_tool_results),
      target: entry.target,
      person: entry.person === true,
      omitted: entry.omitted_blocks,
      gapAfter: entry.gap_after,
    })),
  };
}
