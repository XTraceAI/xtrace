import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { useData } from '../../data/DataProvider';
import type { CodexPageRequest } from '../../data/generated/CodexPageRequest';
import type { CodexPageReason } from '../../data/generated/CodexPageReason';
import type { MetricToolBlock } from '../../data/generated/MetricToolBlock';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import { Button } from '../../kit/Button';
import { SessionTranscript } from './SessionTranscript';
import type { ConversationMarker } from './ConversationActivity';
import { resolveJump } from './timeline-jump';
import { transcriptProps, unavailableNote } from './transcript-adapter';
import type { RevealOutcome } from './transcript-reveal';
import { useSourceRead } from './useSessionTranscript';

export interface CodexPageIntent {
  token: number;
  locator: MetricToolBlock;
  index: number;
  call: 'first' | 'repeated';
}
export interface CodexRepeatCandidate {
  index: number;
  locator: MetricToolBlock;
  tool: string;
  count: number;
  circling: boolean;
}
export interface CodexPageFeedback {
  intent: CodexPageIntent;
  target: { blockId: string; tool: string } | null;
  outcome: RevealOutcome | null;
  unavailable: string | null;
}
const FIRST: CodexPageRequest = { kind: 'first' };
export function codexPageReason(reason: CodexPageReason): string {
  switch (reason.reason) {
    case 'source':
      return unavailableNote(reason.detail);
    case 'changed':
      return 'This conversation changed while you were reading it. Load it again to continue.';
    case 'invalid_cursor':
      return 'This conversation portion can no longer be opened. Load it again.';
    case 'target_missing':
      return 'The saved tool call could not be found in this conversation.';
    case 'target_ambiguous':
      return 'More than one message has this saved message ID, so the exact call cannot be selected.';
    case 'block_missing':
      return 'The saved tool call could not be confirmed in its message.';
    case 'unsupported_history':
      return 'This view can read one original Codex rollout. Forked conversations, inherited history and conversations spread across several rollouts are not supported here.';
    case 'record_too_large':
      return 'One saved message is too large to show as a complete message in this view.';
  }
}

/** Each owned read replaces the last; no transcript cache or accumulation. */
export function CodexPagedTranscript({
  sessionId,
  intent,
  repeatCandidates,
  onFeedback,
  onReset,
  revealOwner,
}: {
  sessionId: string;
  intent: CodexPageIntent | null;
  repeatCandidates: readonly CodexRepeatCandidate[];
  onFeedback: (feedback: CodexPageFeedback | null) => void;
  onReset: () => void;
  revealOwner: RefObject<HTMLElement | null>;
}) {
  const { source } = useData();
  const digest = useRef<string | null>(null);
  const [held, setHeld] = useState<{ token: number | null; selection: CodexPageRequest }>({
    token: null,
    selection: FIRST,
  });
  const [restart, setRestart] = useState(0);
  // Losing a stale highlight must not replace its read or dismiss a changed-source refusal.
  if (intent && held.token !== intent.token) {
    setHeld({
      token: intent.token,
      selection: {
        kind: 'tool_call',
        record_uuid: intent.locator.record_uuid,
        block: intent.locator.block_index,
        generation: digest.current,
      },
    });
  }
  const selection = held.selection;
  const identity = JSON.stringify([sessionId, held.token, selection, restart]);
  const load = useCallback(
    (id: string) => {
      if (!source.codexTranscriptPage) return Promise.reject(new Error('Page reader unavailable'));
      return source.codexTranscriptPage(sessionId, selection, id);
    },
    [source, sessionId, selection],
  );
  const cancel = useCallback((id: string) => source.cancelSessionTranscript(id), [source]);
  const { read, retry } = useSourceRead(identity, load, cancel);
  const page = read.phase === 'read' && read.status.state === 'loaded' ? read.status : null;
  useEffect(() => {
    if (page) digest.current = page.digest;
  }, [page]);
  const view = useMemo(() => {
    if (!page) return null;
    const status: SessionSourceStatus = {
      state: 'loaded',
      generation: { generation: 'unrecorded' },
      sources: 1,
      records: page.records,
      gaps: [],
      dropped_records: 0,
    };
    return { status, transcript: transcriptProps(status, false) };
  }, [page]);
  const target = useMemo(() => {
    if (
      !page ||
      !view ||
      page.target === null ||
      selection.kind !== 'tool_call' ||
      !intent ||
      held.token !== intent.token
    )
      return null;
    if (page.records[page.target]?.id !== intent.locator.record_uuid) return null;
    const jump = resolveJump(intent.locator, { phase: 'read', status: view.status });
    return jump.kind === 'target' ? { blockId: jump.blockId, tool: jump.tool } : null;
  }, [page, view, selection, intent, held.token]);
  const reveal = useMemo(
    () => (target && intent ? { token: intent.token, blockId: target.blockId } : null),
    [target, intent],
  );
  const markers = useMemo<ConversationMarker[]>(() => {
    if (!view) return [];
    return repeatCandidates.flatMap((candidate) => {
      const jump = resolveJump(candidate.locator, { phase: 'read', status: view.status });
      return jump.kind === 'target'
        ? [
            {
              index: candidate.index,
              blockId: jump.blockId,
              tool: candidate.tool,
              count: candidate.count,
              circling: candidate.circling,
              selected: intent?.call === 'repeated' && intent.index === candidate.index,
            },
          ]
        : [];
    });
  }, [repeatCandidates, view, intent]);
  const [result, setResult] = useState<{ token: number; outcome: RevealOutcome } | null>(null);
  const onReveal = useCallback(
    (token: number, outcome: RevealOutcome) => setResult({ token, outcome }),
    [],
  );
  const navigate = (cursor: string | null | undefined) => {
    if (cursor) {
      onFeedback(null);
      setHeld((current) => ({ ...current, selection: { kind: 'cursor', cursor } }));
    }
  };
  const reload = () => {
    onReset();
    digest.current = null;
    setHeld((current) => ({ ...current, selection: FIRST }));
    setRestart((n) => n + 1);
  };
  const note =
    read.phase === 'failed'
      ? 'This conversation portion could not be read just now.'
      : read.phase === 'read' && read.status.state === 'unavailable'
        ? codexPageReason(read.status.reason)
        : null;
  const feedback = useMemo<CodexPageFeedback | null>(
    () =>
      intent && held.token === intent.token && selection.kind === 'tool_call'
        ? {
            intent,
            target,
            outcome: result?.token === intent.token ? result.outcome : null,
            unavailable: note,
          }
        : null,
    [intent, held.token, selection, target, result, note],
  );
  useEffect(() => {
    onFeedback(feedback);
  }, [feedback, onFeedback]);
  return (
    <>
      <p className="xt-session-detail-notice" role="note">
        This conversation is read in bounded portions. It may have changed since the saved
        measurements were taken. Results for tool calls are not checked across the whole
        conversation.
      </p>
      <nav aria-label="Conversation portions">
        <Button
          variant="outline"
          height={28}
          aria-disabled={!page?.previous}
          onClick={() => navigate(page?.previous)}
        >
          Earlier messages
        </Button>{' '}
        <Button
          variant="outline"
          height={28}
          aria-disabled={!page?.next}
          onClick={() => navigate(page?.next)}
        >
          Later messages
        </Button>{' '}
        <Button variant="outline" height={28} onClick={reload}>
          Load conversation again
        </Button>
      </nav>
      {page && (
        <p role="status">
          {page.total === 0
            ? 'No saved messages were found.'
            : `Showing messages ${page.start + 1}–${page.end} of ${page.total}.`}
        </p>
      )}
      {read.phase === 'loading' ? (
        <SessionTranscript conversation records={[]} state={{ kind: 'loading' }} />
      ) : view ? (
        <SessionTranscript
          conversation
          revealOwner={revealOwner}
          groupAdjacentActivity
          markers={markers}
          records={view.transcript.records}
          state={view.transcript.state}
          reveal={reveal}
          onReveal={onReveal}
        />
      ) : (
        <>
          <SessionTranscript
            conversation
            records={[]}
            state={{
              kind: 'unavailable',
              note: note ?? 'This conversation portion is unavailable.',
            }}
          />
          {read.phase === 'failed' && (
            <Button variant="outline" height={28} onClick={retry}>
              Try again
            </Button>
          )}
        </>
      )}
    </>
  );
}
