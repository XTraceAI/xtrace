import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { clock } from '../../kit/clock';
import { SessionTranscript } from '../session-detail/SessionTranscript';
import { ToolCallCard } from '../session-detail/ToolStrip';
import { generationNote } from '../session-detail/transcript-adapter';
import { useWarningContext } from './useWarningContext';
import { warningExcerpt } from './warning-context';
import '../../styles/session-warnings.css';

export function WarningContext({
  sessionId,
  warning,
  timezone,
  fullSession,
}: {
  sessionId: string;
  warning: SessionWarningItem;
  timezone: string;
  fullSession?: { href: string; disabled: boolean; open: () => void };
}) {
  const { read, retry } = useWarningContext(sessionId, warning.points_at);
  const excerpt = warningExcerpt(read);
  const generation = read.phase === 'read' ? generationNote(read.status) : null;
  let notice: string | null = null;
  if (read.phase === 'loading') notice = 'Loading the original conversation…';
  else if (read.phase === 'failed') notice = 'The conversation could not be read. Try again.';
  else if (read.status.state === 'unavailable')
    notice = excerpt.kind === 'unavailable' ? excerpt.reason : null;
  return (
    <section
      className="xt-warning-context"
      aria-label="Conversation context"
      data-testid="warning-context"
    >
      <div className="xt-warning-context-head">
        <h3>Conversation context</h3>
        {fullSession ? (
          <a
            href={fullSession.disabled ? undefined : fullSession.href}
            role="link"
            tabIndex={0}
            className="xt-warning-action"
            aria-disabled={fullSession.disabled || undefined}
            title={
              fullSession.disabled
                ? 'Finish or stop the diagnosis before opening the full session.'
                : undefined
            }
            onClick={(event) => {
              event.preventDefault();
              if (!fullSession.disabled) fullSession.open();
            }}
          >
            View in full session
          </a>
        ) : (
          <span>Local conversation excerpt</span>
        )}
      </div>
      {notice ? (
        <p role="status" className="xt-warning-context-note">
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
          <p className="xt-warning-context-note">
            {excerpt.neighborsAvailable
              ? 'Showing the triggering message and up to one message on either side from the same source.'
              : 'Nearby messages are unavailable because this message’s conversation source could not be identified.'}
          </p>
          {excerpt.records.map(({ record, target, omitted }, index) => {
            const single = record.blocks.length === 1 ? record.blocks[0] : null;
            const tool =
              target &&
              warning.points_at.block !== null &&
              single &&
              (single.kind === 'tool_call' || single.kind === 'tool_result')
                ? single
                : null;
            const timed = {
              ...record,
              at: record.at
                ? {
                    ...record.at,
                    label: clock(new Date(record.at.iso).getTime(), { timeZone: timezone }),
                  }
                : null,
            };
            return (
              <div
                className="xt-warning-context-message"
                data-trigger={target || undefined}
                key={`${index}:${record.id}`}
              >
                {target && <span className="xt-warning-context-trigger">Issue message</span>}
                {tool ? (
                  <>
                    <div className="xt-warning-context-tool-time">{timed.at?.label}</div>
                    <ToolCallCard block={tool} highlighted />
                  </>
                ) : (
                  <SessionTranscript records={[timed]} />
                )}
                {omitted && (
                  <p className="xt-warning-context-note">
                    Other blocks in this message are not shown in this excerpt.
                  </p>
                )}
              </div>
            );
          })}
        </>
      )}
    </section>
  );
}
