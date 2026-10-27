import { useContext, useLayoutEffect, useRef } from 'react';
import { RevealContext } from './transcript-reveal';
import type { ConversationMarker } from './ConversationActivity';
import { ToolActivityStrip } from './ToolStrip';
import { TranscriptMarkdown } from './TranscriptMarkdown';
import { CodeSurface, TranscriptText } from './TranscriptText';
import {
  type TranscriptBlock,
  type TranscriptRecord,
  type TranscriptRole,
  recordSegments,
} from './transcript-view';
import './transcript.css';

/**
 * What the header calls each side.
 *
 * Fixed wording, and no name: who a session belonged to is attribution, which this view does
 * not do and is not given the data for. `User` is the role the record was saved under, not a
 * claim that a person typed it — another agent's prompt is saved under the same role.
 */
const ROLE_LABEL: Record<TranscriptRole, string> = {
  user: 'User',
  assistant: 'Agent',
  system: 'System',
  tool: 'Tool',
};

/**
 * One non-tool block, drawn as itself.
 *
 * Only the agent's text is formatted. It is Markdown because the model wrote Markdown; a
 * person's prompt, a system line, thinking and code are shown as the characters they are —
 * see `TranscriptMarkdown` for why the line falls there.
 */
function TranscriptBlockView({
  block,
  role,
}: {
  block: Exclude<TranscriptBlock, { kind: 'tool_call' | 'tool_result' }>;
  role: TranscriptRole;
}) {
  return (
    <div data-block-id={block.id} data-block={block.kind}>
      {block.kind === 'text' || block.kind === 'thinking' ? (
        <TranscriptText
          text={block.text}
          kind={block.kind}
          formatted={
            role === 'assistant' && block.kind === 'text' ? (
              <TranscriptMarkdown text={block.text} />
            ) : undefined
          }
        />
      ) : block.kind === 'code' ? (
        <CodeSurface code={block.text} label={block.label ?? undefined} />
      ) : (
        // The honest fallback. A block this view cannot draw is named, not guessed at and not
        // dumped as JSON — and a media block is named rather than loaded, because this view
        // fetches nothing.
        <p className="xt-tx-unsupported">
          {block.label?.trim()
            ? `A block of type “${block.label.trim()}” in this turn is not shown here.`
            : 'A block in this turn is not shown here.'}
        </p>
      )}
    </div>
  );
}

/** One saved record; conversation mode changes its presentation, never its identity. */
export function TranscriptRecordView({
  record,
  conversation = false,
  markers = [],
}: {
  record: TranscriptRecord;
  conversation?: boolean;
  markers?: readonly ConversationMarker[];
}) {
  const segments = recordSegments(record);
  const { request, report, scroll } = useContext(RevealContext);
  const element = useRef<HTMLElement>(null);
  const answered = useRef<number | null>(null);
  const highlighted = request?.kind === 'record' && request.recordId === record.id;
  useLayoutEffect(() => {
    if (!highlighted || !request || answered.current === request.token) return;
    answered.current = request.token;
    if (!element.current?.isConnected) {
      report(request.token, 'not_found');
      return;
    }
    scroll(element.current);
    report(request.token, 'revealed');
  }, [highlighted, request, report, scroll]);

  return (
    <article
      ref={element}
      data-highlighted={highlighted || undefined}
      className={`xt-turn${conversation ? ' xt-conversation-turn' : ''}`}
      data-role={record.role}
      data-record-id={record.id}
    >
      <header className="xt-turn-head">
        <span>{ROLE_LABEL[record.role]}</span>
        {typeof record.ordinal === 'number' ? <span>turn {record.ordinal}</span> : null}
        {record.at ? (
          <time className="xt-turn-time" dateTime={record.at.iso}>
            {record.at.label}
          </time>
        ) : null}
      </header>

      <div className="xt-turn-body">
        {segments.length === 0 ? (
          // The turn happened; what it carried is not in this view. Dropping the record
          // instead would quietly shorten the session.
          <p className="xt-tx-unsupported">No content from this turn is shown here.</p>
        ) : (
          segments.map((segment) =>
            segment.kind === 'tools' ? (
              <ToolActivityStrip
                key={segment.key}
                blocks={segment.blocks}
                ordered={conversation}
                markers={markers}
              />
            ) : (
              <TranscriptBlockView key={segment.key} block={segment.block} role={record.role} />
            ),
          )
        )}
      </div>
    </article>
  );
}
