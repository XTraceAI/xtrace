import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { clock } from '../../kit/clock';
import { HostGlyph } from '../../kit/HostGlyph';
import { hostName } from '../../kit/hosts';
import { ToolCallCard } from '../session-detail/ToolStrip';
import { TranscriptMarkdown } from '../session-detail/TranscriptMarkdown';
import { ROLE_LABEL } from '../session-detail/TranscriptRecordView';
import { CodeSurface, TranscriptText } from '../session-detail/TranscriptText';
import { generationNote } from '../session-detail/transcript-adapter';
import {
  recordSegments,
  type TranscriptBlock,
  type TranscriptRecord,
} from '../session-detail/transcript-view';
import { useWarningContext } from './useWarningContext';
import { warningExcerpt, type WarningContextRead } from './warning-context';
import { Spinner } from '../session-diagnosis';
import '../../styles/session-warnings.css';

/**
 * The messages around one warning, read from the original conversation and
 * drawn as a chat: what was saved under the user role on the right, in a
 * bubble under "You"; the agent's text on the left under "Agent"; each tool
 * call or result as a card. The message the warning points at is outlined
 * and tagged with the warning's kind. What each record holds, and which one
 * is the target, is the native excerpt's (`warningExcerpt`); this only lays
 * it out.
 */
export function WarningContext({
  sessionId,
  warning,
  ...props
}: {
  sessionId: string;
  warning: SessionWarningItem;
  timezone: string;
  host?: string;
  fullSession?: { href: string; open: () => void };
}) {
  const { read, retry } = useWarningContext(sessionId, warning, props.timezone);
  return <WarningContextView read={read} retry={retry} warning={warning} {...props} />;
}

/**
 * The conversation around a warning, from a read its caller owns (the
 * Dashboard's window reads it once and shows it here and in its reason).
 */
export function WarningContextView({
  read,
  retry,
  warning,
  timezone,
  host,
  fullSession,
}: {
  read: WarningContextRead;
  retry: () => void;
  warning: SessionWarningItem;
  timezone: string;
  /** The session's host: named in the header, and the agent's mark. */
  host?: string;
  fullSession?: { href: string; open: () => void };
}) {
  const excerpt = warningExcerpt(read);
  const generation = read.phase === 'read' ? generationNote(read.status) : null;
  let notice: string | null = null;
  if (read.phase === 'loading') notice = 'Loading the original conversation…';
  else if (read.phase === 'failed') notice = 'The conversation could not be read. Try again.';
  else if (read.status.state === 'unavailable')
    notice = excerpt.kind === 'unavailable' ? excerpt.reason : null;
  const tag = SESSION_WARNINGS_TEXT.message_tags[warning.kind];
  const when = [
    host ? hostName(host) : null,
    warning.day,
    clock(warning.at_ms, { timeZone: timezone }),
  ].filter(Boolean);
  return (
    <section
      className="xt-warning-context"
      aria-label={SESSION_WARNINGS_TEXT.conversation_heading}
      data-testid="warning-context"
    >
      <div className="xt-warning-context-head">
        <h3>{SESSION_WARNINGS_TEXT.conversation_heading}</h3>
        <span data-testid="warning-context-when">{when.join(' · ')}</span>
      </div>
      {notice ? (
        <p
          role="status"
          className="xt-warning-context-note"
          data-loading={read.phase === 'loading' || undefined}
        >
          {read.phase === 'loading' && <Spinner />}
          {notice}
        </p>
      ) : null}
      {read.phase === 'failed' && (
        <button type="button" className="xt-warning-action" onClick={retry}>
          Try again
        </button>
      )}
      {generation && (
        <p role="note" className="xt-warning-context-note">
          {generation}
        </p>
      )}
      {excerpt.kind === 'excerpt' && (
        <>
          {excerpt.state.kind === 'incomplete' && (
            <p role="note" className="xt-warning-context-note">
              {excerpt.state.note}
            </p>
          )}
          <div className="xt-chat">
            {excerpt.records.map(({ record, target, omitted, person }, index) => (
              <div
                className="xt-warning-context-message"
                data-trigger={target || undefined}
                key={`${index}:${record.id}`}
              >
                <ChatRecord
                  record={record}
                  person={person}
                  host={host}
                  flagged={target ? { tag, block: flaggedBlock(record, warning) } : null}
                />
                {omitted && (
                  <p className="xt-warning-context-note">
                    Other blocks in this message are not shown in this excerpt.
                  </p>
                )}
              </div>
            ))}
          </div>
          <p className="xt-warning-context-caption">
            {excerpt.neighborsAvailable
              ? 'Showing the triggering message and up to one message on either side from the same source.'
              : 'Nearby messages are unavailable because this message’s conversation source could not be identified.'}
          </p>
        </>
      )}
      {fullSession && (
        <div className="xt-warning-context-foot">
          <a
            href={fullSession.href}
            role="link"
            tabIndex={0}
            className="xt-warning-action"
            onClick={(event) => {
              event.preventDefault();
              fullSession.open();
            }}
          >
            View in full session
          </a>
        </div>
      )}
    </section>
  );
}

/**
 * What on the warning's own record is outlined: its one tool card when the
 * warning points at a tool block and the excerpt holds just that block,
 * otherwise the whole message.
 */
function flaggedBlock(record: TranscriptRecord, warning: SessionWarningItem): 'tool' | null {
  const single = record.blocks.length === 1 ? record.blocks[0] : null;
  return warning.points_at.block !== null &&
    (single?.kind === 'tool_call' || single?.kind === 'tool_result')
    ? 'tool'
    : null;
}

/** The flagged message's tag, a dot and the kind's singular name. */
function FlagTag({ tag }: { tag: string }) {
  return (
    <span className="xt-chat-flag" data-testid="warning-context-flag">
      <span className="xt-chat-flag-dot" aria-hidden="true" />
      {tag}
    </span>
  );
}

/**
 * One saved record as chat. `flagged` is set on the warning's own record:
 * for a tool block the excerpt holds only that block, and its card is
 * outlined; otherwise the whole message is.
 */
function ChatRecord({
  record,
  person,
  host,
  flagged,
}: {
  record: TranscriptRecord;
  /** M-02 counts it as a person's message: the only "You". */
  person: boolean;
  host?: string;
  flagged: { tag: string; block: 'tool' | null } | null;
}) {
  const segments = recordSegments(record);
  // Only a message M-02 counts as a person's is "You", on the right. Any
  // other record (an agent's, a sub-agent's prompt, a reminder, tool work)
  // keeps the transcript's neutral caption for its saved role, on the left.
  const own = person && segments.some((segment) => segment.kind === 'block');
  const who = own ? SESSION_WARNINGS_TEXT.you : ROLE_LABEL[record.role];
  const flaggedMessage = flagged !== null && flagged.block === null;
  const body =
    segments.length === 0 ? (
      <p className="xt-tx-unsupported">No content from this turn is shown here.</p>
    ) : (
      segments.map((segment) =>
        segment.kind === 'tools' ? (
          <div className="xt-chat-tools" key={segment.key}>
            {segment.blocks.map((block, index) => {
              const flaggedTool = flagged?.block === 'tool';
              return (
                <div
                  className="xt-chat-tool"
                  key={`${index}:${block.id}`}
                  data-flagged={flaggedTool || undefined}
                >
                  <ToolCallCard
                    block={block}
                    highlighted={flaggedTool}
                    excerpt
                    tag={flaggedTool ? <FlagTag tag={flagged.tag} /> : undefined}
                  />
                </div>
              );
            })}
          </div>
        ) : (
          <ChatBlock key={segment.key} block={segment.block} own={own} role={record.role} />
        ),
      )
    );
  return (
    <article
      className="xt-chat-turn"
      data-role={record.role}
      data-own={own || undefined}
      data-record-id={record.id}
      data-flagged={flaggedMessage || undefined}
      data-highlighted={flaggedMessage || undefined}
    >
      <header className="xt-chat-who">
        {!own &&
          (record.role === 'assistant' && host ? (
            <HostGlyph host={host} />
          ) : (
            <span className="xt-chat-avatar" aria-hidden="true">
              ⌘
            </span>
          ))}
        <span>{who}</span>
        {flaggedMessage && <FlagTag tag={flagged.tag} />}
      </header>
      <div className="xt-chat-body">{body}</div>
    </article>
  );
}

/** One non-tool block: a bubble for your own text, plain text for the agent's. */
function ChatBlock({
  block,
  own,
  role,
}: {
  block: Exclude<TranscriptBlock, { kind: 'tool_call' | 'tool_result' }>;
  own: boolean;
  role: TranscriptRecord['role'];
}) {
  const content =
    block.kind === 'text' || block.kind === 'thinking' ? (
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
      <p className="xt-tx-unsupported">
        {block.label?.trim()
          ? `A block of type “${block.label.trim()}” in this turn is not shown here.`
          : 'A block in this turn is not shown here.'}
      </p>
    );
  return (
    <div
      className={own ? 'xt-chat-bubble' : 'xt-chat-text'}
      data-block-id={block.id}
      data-block={block.kind}
    >
      {content}
    </div>
  );
}
