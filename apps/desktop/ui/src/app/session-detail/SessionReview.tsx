import { useEffect, useRef, type ReactNode } from 'react';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../../data/generated/SessionWarningsAnswer';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { clock } from '../../kit/clock';
import {
  SessionWarningsDetail,
  WarningChecks,
  WarningRow,
  WarningSign,
  useWarningDismissals,
} from '../session-warnings';

/** One selected issue owns its actions; its compact neighbours remain reachable. */
export function SessionReview({
  sessionId,
  answer,
  pending,
  failed,
  selection,
  onSelect,
  onReadbackSelect,
  navigationGeneration,
  actions,
  below,
  children,
}: {
  sessionId: string;
  answer: SessionWarningsAnswer | undefined;
  pending: boolean;
  failed: boolean;
  selection: string | null;
  onSelect: (id: string) => void;
  onReadbackSelect: (id: string) => void;
  navigationGeneration: number;
  actions?: (warning: SessionWarningItem) => ReactNode;
  below?: (warning: SessionWarningItem) => ReactNode;
  children: ReactNode;
}) {
  const card = useRef<HTMLDivElement>(null);
  const choices = useRef<HTMLDivElement>(null);
  const shown = useRef<string | null>(null);
  const shownDismissed = useRef<boolean | null>(null);
  const read = answer?.state === 'read' ? answer : null;
  const selected =
    read && [...read.warnings, ...read.dismissed].find((warning) => warning.id === selection);
  const undo = Boolean(selected && read?.dismissed.some((warning) => warning.id === selected.id));
  const rowProps = useWarningDismissals(sessionId, read, card, {
    generation: navigationGeneration,
    onMoved: onReadbackSelect,
  });
  useEffect(() => {
    const context = JSON.stringify([sessionId, selection, selected?.points_at]);
    if (
      selection === null ||
      !read ||
      (shown.current === context && shownDismissed.current === undo)
    )
      return;
    const focusSelection = shown.current !== context;
    shown.current = context;
    shownDismissed.current = undo;
    const target =
      card.current?.querySelector<HTMLElement>('[data-warning-id]') ??
      card.current?.querySelector<HTMLElement>('.xt-warnings-summary');
    if (focusSelection) target?.focus({ preventScroll: true });
    const list = choices.current;
    const button = list?.querySelector<HTMLElement>('[aria-current="true"]');
    if (list && button) {
      const top = button.offsetTop;
      const bottom = top + button.offsetHeight;
      if (top < list.scrollTop || bottom > list.scrollTop + list.clientHeight) {
        const first = [...list.querySelectorAll<HTMLElement>('.xt-session-issue-choice')].find(
          (item) => item.offsetTop >= Math.max(0, bottom - list.clientHeight),
        );
        list.scrollTop = first?.offsetTop ?? top;
      }
    }
  }, [sessionId, selection, selected, read, undo]);
  const choice = (warning: SessionWarningItem, dismissed: boolean) => (
    <li key={warning.id}>
      <button
        type="button"
        className="xt-session-issue-choice"
        data-testid="warning-choice"
        data-dismissed={dismissed || undefined}
        aria-label={warning.name}
        aria-current={warning.id === selected?.id ? 'true' : undefined}
        onClick={() => onSelect(warning.id)}
      >
        <span className="xt-session-issue-time">
          {warning.day} · {clock(warning.at_ms)}
        </span>
        <strong>{warning.label}</strong>
        {warning.compact_reason && (
          <span className="xt-session-issue-fact">{warning.compact_reason}</span>
        )}
      </button>
    </li>
  );
  const hasIssues = Boolean(read && (read.warnings.length || read.dismissed.length));
  return (
    <div ref={card} className="xt-session-review" data-has-issues={hasIssues || undefined}>
      <header className="xt-session-review-heading">
        <div>
          <h2>Warnings</h2>
          <span>{SESSION_WARNINGS_TEXT.session_scope}</span>
        </div>
        {read && hasIssues ? (
          <>
            <p
              className="xt-warnings-summary"
              tabIndex={-1}
              data-testid="warnings-summary"
              data-incomplete={read.incomplete_checks > 0 || undefined}
            >
              {read.warnings.length > 0 && <WarningSign size={14} />}
              <span>{read.summary}</span>
            </p>
            <WarningChecks answer={read} />
          </>
        ) : (
          <SessionWarningsDetail
            sessionId={sessionId}
            answer={answer}
            pending={pending}
            failed={failed}
            highlight={selection}
          />
        )}
      </header>
      <div className="xt-session-review-layout">
        {hasIssues && read && (
          <aside className="xt-session-issue-nav" aria-label="Warnings">
            <div className="xt-session-issue-position">
              <span>{read.count_label ?? 'Warnings'}</span>
              <span>
                {undo
                  ? SESSION_WARNINGS_TEXT.dismissed_heading
                  : selected
                    ? `${read.warnings.findIndex((warning) => warning.id === selected.id) + 1} of ${read.warnings.length}`
                    : ''}
              </span>
            </div>
            <div ref={choices} className="xt-session-issue-scroll">
              {read.warnings.length > 0 && (
                <ol aria-label="Warnings" data-testid="warnings-list">
                  {read.warnings.map((warning) => choice(warning, false))}
                </ol>
              )}
              {read.dismissed.length > 0 && (
                <details className="xt-session-dismissed" open={undo || undefined}>
                  <summary>{SESSION_WARNINGS_TEXT.dismissed_heading}</summary>
                  <ol
                    aria-label={SESSION_WARNINGS_TEXT.dismissed_heading}
                    data-testid="warnings-dismissed"
                  >
                    {read.dismissed.map((warning) => choice(warning, true))}
                  </ol>
                </details>
              )}
            </div>
          </aside>
        )}
        <div className="xt-session-reading">
          {selected && (
            <ol
              className="xt-warnings-list xt-session-selected-issue"
              aria-label="Selected warning"
            >
              <WarningRow
                key={JSON.stringify([sessionId, selected.id, selected.points_at])}
                warning={selected}
                undo={undo}
                highlighted
                {...rowProps(selected, undo)}
                actions={undo ? undefined : actions?.(selected)}
                below={undo ? undefined : below?.(selected)}
                summary={
                  <div className="xt-session-selected-summary">
                    <span className="xt-session-issue-time">
                      {selected.day} · {clock(selected.at_ms)}
                    </span>
                    <h3>{selected.label}</h3>
                    <p>{selected.reason}</p>
                  </div>
                }
              />
            </ol>
          )}
          {children}
        </div>
      </div>
    </div>
  );
}
