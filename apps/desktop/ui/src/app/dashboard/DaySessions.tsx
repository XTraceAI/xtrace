import { useState, type CSSProperties } from 'react';
import { useQuery } from '@tanstack/react-query';
import { usePurgeEpoch } from '../../data/content-purges';
import { Link } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DashboardDaySession } from '../../data/generated/DashboardDaySession';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { queryKeys } from '../../data/query-client';
import { count } from '../../kit/format';
import type { TimeRange } from '../../kit/TopBar';
import { agentDuration, agentTime } from '../agent-duration';
import { displayTitle, shortId } from '../session-context';
import { GroupToggle, parentName, verifiedParent } from '../session-parent';
import { listAddress, sessionHref } from '../session-search';
import { useSessionTitles } from '../session-titles';
import { dayList, dayMs, type DayLane, type DayRow } from './day-sessions';
import {
  checkingHidden,
  checkingHiddenShort,
  creatorHidden,
  creatorHiddenShort,
  rowGroup,
  sessionKey,
  showsTotal,
} from './lanes';
import { plural } from './present';

/** The version a listed session's host title is read by; see `last_activity_at_ms`. */
const titleVersion = (session: DashboardDaySession) => session.last_activity_at_ms ?? 'unknown';

/**
 * A "human + agent" day's sessions box, under a heading that names the day:
 * the day's sessions with agent time, the most first, each with its time that
 * day and a link that opens it, then what the box leaves out, with its time.
 * The sessions and every number are the native side's answer for this day of
 * this report, so everything the box says adds up to the day's agent time.
 *
 * Which sessions are listed, and how sub-sessions appear, follow the rules
 * the Dashboard's Sessions section lists its sessions by ([`dayList`]): a
 * verified sub-session sits under the session that created it, collapsed
 * behind that row's disclosure, or under one row that names a parent that had
 * no agent time that day. A session still being checked, or a known
 * sub-session whose creator is not verified, is left out with every session
 * under it, and counted. A collapsed group shows its sessions' time that day
 * added up (Σ), as a collapsed group of lanes shows its sessions' cost; open,
 * each row shows its own. Groups start collapsed each time the box opens.
 *
 * Read when the box first opens (it is only mounted while it is open) and
 * kept for the same report: opening the same day again reads nothing,
 * a new report is a new read, and committed data re-reads it. While a new
 * report's answer is on its way the box keeps the last one, unless stored
 * content was deleted since, which drops every kept answer. A session is
 * named as the lanes name it: its host title when that has been read, else its
 * saved title or reviewer mark, else its short identifier. Host titles are
 * read only for the sessions drawn; a parent named only by a group's row
 * keeps the title the native side sent for it.
 */
export function DaySessions({
  window,
  date,
  label,
  range,
  listRef,
}: {
  window: DashboardWindow;
  date: string;
  /** The day as its row writes it, for the heading. */
  label: string;
  /** The Dashboard's range, carried by each link as the lanes' links carry it. */
  range: TimeRange;
  /** Given the list of links once the sessions have been read. */
  listRef?: (list: HTMLUListElement | null) => void;
}) {
  const { source } = useData();
  const available = Boolean(source.daySessions);
  // Keyed by the purge epoch, so no saved title read before "Delete stored
  // content" is shown after it, not even while the next answer is on its way.
  const purges = usePurgeEpoch();
  const query = useQuery({
    queryKey: queryKeys.daySessions(purges, window.days, window.end_ms, window.timezone, date),
    queryFn: () => source.daySessions!(window.days, window.end_ms, window.timezone, date),
    enabled: available,
    staleTime: Infinity,
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === purges ? previous : undefined,
  });
  // Which groups are open, by row key: collapsed until asked, and kept while
  // the box's answer is read again.
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const toggle = (key: string) =>
    setOpen((previous) => {
      const next = new Set(previous);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  const list = query.data ? dayList(query.data, open) : null;
  const drawn = list?.rows.flatMap((row) => (row.kind === 'session' ? [row.lane] : [])) ?? [];
  const hostTitle = useSessionTitles(
    JSON.stringify(['effort-day', date]),
    // The session's latest activity, on any day, as Sessions shows it: every
    // new event reads its title again, even one after this day or one that
    // adds no time. A session with none known is read once.
    drawn.map((lane) => ({ id: lane.sessionId, version: titleVersion(lane.session) })),
    { keepResolved: true },
  );
  if (!available) return null;
  const titleOf = (lane: DayLane) =>
    displayTitle(
      hostTitle(lane.sessionId, titleVersion(lane.session)) ?? lane.session.title,
      lane.session.automated_review,
    );
  const nameOf = (lane: DayLane) => titleOf(lane) ?? `Session ${shortId(lane.sessionId)}`;
  // A verified parent is named by its own row when it is drawn here, as that
  // row reads now; otherwise by the name the native side sent for it.
  const drawnByKey = new Map(drawn.map((lane) => [lane.key, lane]));
  const createdBy = (lane: DayLane) => {
    const parent = verifiedParent({ id: lane.sessionId, parent: lane.session.parent });
    if (!parent) return null;
    const shown = drawnByKey.get(sessionKey(parent.host, parent.session_id));
    return { parent, name: parentName(parent, shown ? titleOf(shown) : undefined) };
  };
  const linkTo = (sessionId: string, host: string) =>
    sessionHref(sessionId, listAddress({ sessionId, host }, range));
  /** A collapsed group's total, as its row writes and says it; none otherwise. */
  const groupTotal = (row: DayRow) => {
    if (!showsTotal(row)) return null;
    const ms = dayMs(rowGroup(row));
    return {
      visible: `Σ ${agentTime(ms)}`,
      spoken: agentDuration(ms).spoken,
      members: plural(row.members.length, 'sub-session'),
    };
  };

  const item = (row: DayRow) => {
    const style = row.depth > 0 ? ({ '--depth': row.depth } as CSSProperties) : undefined;
    const total = groupTotal(row);
    if (row.kind === 'absent') {
      const name = parentName(row.parent);
      return (
        <li key={row.key} className="xt-effort-tip-item" data-group="absent" style={style}>
          <GroupToggle row={row} name={name} noun="sub-session" onToggle={toggle} />
          <Link
            className="xt-effort-tip-session"
            data-testid="day-session-parent"
            to={linkTo(row.parent.session_id, row.parent.host)}
            aria-label={`Open parent session ${name}, ${row.parent.session_id}, which had no agent time this day${
              total ? `; its ${total.members}: ${total.spoken} this day` : ''
            }`}
            title={`Sub-sessions of ${name} · ${row.parent.session_id}`}
          >
            <span>
              <span className="xt-effort-tip-group-words">Sub-sessions of </span>
              {name}
            </span>
            {total && <b>{total.visible}</b>}
          </Link>
        </li>
      );
    }
    const { lane } = row;
    const title = titleOf(lane);
    const name = nameOf(lane);
    const created = createdBy(lane);
    return (
      <li
        key={row.key}
        className="xt-effort-tip-item"
        data-subsession={created ? '' : undefined}
        style={style}
      >
        {row.children > 0 && (
          <GroupToggle row={row} name={name} noun="sub-session" onToggle={toggle} />
        )}
        <Link
          className="xt-effort-tip-session"
          data-testid="day-session"
          to={linkTo(lane.sessionId, lane.host)}
          // The visible name can be cut to fit, so the link names the
          // session in full, as a lane's link does.
          aria-label={`Open session ${title ? `${title}, ` : ''}${lane.sessionId}${
            created ? `, sub-session of ${created.name}` : ''
          }, ${agentDuration(lane.session.agent_ms).spoken} this day${
            total ? `; ${total.spoken} this day with its ${total.members}` : ''
          }`}
          title={`${name} · ${lane.sessionId}`}
        >
          <span>{name}</span>
          <b>{total ? total.visible : agentTime(lane.session.agent_ms)}</b>
        </Link>
      </li>
    );
  };
  /** One line counting sessions the box leaves out, with their time that day. */
  const leftOut = (testId: string, short: string, full: string, ms: number) => (
    <li className="xt-effort-tip-row" data-testid={testId} title={full}>
      <span aria-hidden="true">{short}</span>
      <span className="sr-only">{full}</span>
      <b>{agentTime(ms)}</b>
    </li>
  );

  const body = query.isError ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Sessions could not be read
    </span>
  ) : !query.data || !list ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      Reading sessions…
    </span>
  ) : query.data.sessions.length === 0 ? (
    <span className="xt-effort-tip-note" data-testid="day-sessions-state">
      No sessions
    </span>
  ) : (
    <ul ref={listRef} className="xt-effort-tip-sessions" role="list" aria-label="Sessions this day">
      {list.rows.map(item)}
      {list.more.rows > 0 && (
        <li className="xt-effort-tip-row" data-testid="day-sessions-more">
          <span>
            and{' '}
            {list.more.rows === list.more.sessions
              ? plural(list.more.rows, 'more session')
              : `${count(list.more.rows)} more (${plural(list.more.sessions, 'session')})`}
          </span>
          <b>{agentTime(list.more.ms)}</b>
        </li>
      )}
      {list.checking.sessions > 0 &&
        leftOut(
          'day-sessions-checking',
          checkingHiddenShort(list.checking.sessions),
          checkingHidden(list.checking.sessions),
          list.checking.ms,
        )}
      {list.creator.sessions > 0 &&
        leftOut(
          'day-sessions-creator',
          creatorHiddenShort(list.creator.sessions),
          creatorHidden(list.creator.sessions),
          list.creator.ms,
        )}
    </ul>
  );
  return (
    <>
      <span className="xt-effort-tip-head" data-testid="day-sessions-head">
        <span>Sessions on {label}</span>
      </span>
      {body}
    </>
  );
}
