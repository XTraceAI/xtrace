import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { queryKeys } from '../../data/query-client';
import { agentDuration, agentTime } from '../agent-duration';
import { displayTitle, shortId } from '../session-context';
import { listAddress, sessionHref } from '../session-search';
import { useSessionTitles } from '../session-titles';
import { plural, rangeOf } from './present';

/**
 * The "Sessions" part of a "human + agent" day card: every session with agent
 * time that day, the most first, each with its time that day and a link that
 * opens it, then how many more there were and their time when the report
 * capped the list. The list and every number are the native side's answer for
 * this day of this report, so they add up to the card's agent time.
 *
 * Read when the card first opens (this part is only mounted while it is open)
 * and kept for the same report: opening the same day again reads nothing,
 * a new report is a new read, and committed data re-reads it. While a new
 * report's answer is on its way the card keeps the last one. A session is
 * named as the lanes name it: its host title when that has been read, else its
 * saved title or reviewer mark, else its short identifier.
 */
export function DaySessions({ window, date }: { window: DashboardWindow; date: string }) {
  const { source } = useData();
  const available = Boolean(source.daySessions);
  const query = useQuery({
    queryKey: queryKeys.daySessions(window.days, window.end_ms, date),
    queryFn: () => source.daySessions!(window.days, window.end_ms, date),
    enabled: available,
    staleTime: Infinity,
    placeholderData: keepPreviousData,
  });
  const sessions = query.data?.sessions ?? [];
  const hostTitle = useSessionTitles(
    JSON.stringify(['effort-day', date]),
    sessions.map((session) => ({ id: session.session_id, version: session.agent_ms })),
    { keepResolved: true },
  );
  const range = rangeOf(window.days) ?? '7d';
  if (!available) return null;
  const body = query.isError ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Sessions could not be read
    </span>
  ) : !query.data ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Reading sessions…
    </span>
  ) : sessions.length === 0 ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      No sessions
    </span>
  ) : (
    <ul className="xt-effort-tip-sessions" role="list" aria-label="Sessions this day">
      {sessions.map((session) => {
        const title = displayTitle(
          hostTitle(session.session_id, session.agent_ms) ?? session.title,
          session.automated_review,
        );
        const name = title ?? `Session ${shortId(session.session_id)}`;
        return (
          <li key={`${session.host}\u0000${session.session_id}`}>
            <Link
              className="xt-effort-tip-session"
              data-testid="day-session"
              to={sessionHref(
                session.session_id,
                listAddress({ sessionId: session.session_id, host: session.host }, range),
              )}
              // The visible name can be cut to fit, so the link names the
              // session in full, as a lane's link does.
              aria-label={`Open session ${title ? `${title}, ` : ''}${session.session_id}, ${agentDuration(session.agent_ms).spoken} this day`}
              title={`${name} · ${session.session_id}`}
            >
              <span>{name}</span>
              <b>{agentTime(session.agent_ms)}</b>
            </Link>
          </li>
        );
      })}
      {query.data.more_sessions > 0 && (
        <li className="xt-effort-tip-row" data-testid="day-sessions-more">
          <span>and {plural(query.data.more_sessions, 'more session')}</span>
          <b>{agentTime(query.data.more_ms)}</b>
        </li>
      )}
    </ul>
  );
  return (
    <>
      <span className="xt-effort-tip-divider" />
      <span className="xt-effort-tip-row" data-testid="day-sessions-head">
        <span>Sessions</span>
      </span>
      {body}
    </>
  );
}
