import type { DashboardDaySession } from '../../data/generated/DashboardDaySession';
import type { DashboardDaySessions } from '../../data/generated/DashboardDaySessions';
import type { SessionParentLink } from '../../data/generated/SessionParentLink';
import { verifiedParent } from '../session-parent';
import {
  laneRows,
  listedLanes,
  rowGroup,
  sessionKey,
  stillChecking,
  unresolvedIn,
  type ChildContext,
  type LaneRow,
  type ListedSession,
} from './lanes';

/**
 * The most top-level rows a day's sessions box lists: a session of its own, a
 * group under its parent, or a group under a parent that is not one of the
 * day's sessions, each counts once, however many sessions it holds. The rest
 * are counted, with their time, as "and N more".
 *
 * The cap lives here, beside the box, because it counts rows the shared
 * grouping ([`laneRows`]) makes, and that grouping is the view's: a cap on
 * sessions, applied before grouping, could cut a group in two or count a
 * sub-session as a row of its own. The native read sends all daily sessions;
 * it decides no display.
 */
export const DAY_ROW_LIMIT = 8;

/** One of the day's sessions as the shared list rules read it. */
export interface DayLane extends ListedSession {
  session: DashboardDaySession;
}

export type DayRow = LaneRow<SessionParentLink, DayLane>;

/** Sessions the box does not list, and their time that day. */
export interface DayCount {
  sessions: number;
  ms: number;
}

/**
 * What a day's sessions box lists, decided by the rules the Dashboard's
 * Sessions section lists its sessions by, over the day's sessions in the
 * order they were sent (the most time first):
 *
 * - a session still being checked, or a known sub-session whose creator is
 *   not verified, is left out with every session under it ([`listedLanes`],
 *   [`unresolvedIn`]), judged from each session's own context and the
 *   context-only entries for parents that are not the day's sessions;
 * - every other session is placed once ([`laneRows`]): a verified sub-session
 *   under its parent, collapsed unless its group's key is in `open`, or under
 *   one `absent` row naming a parent that is not one of the day's sessions;
 * - the first [`DAY_ROW_LIMIT`] top-level rows are drawn, with the rows of
 *   any open group among them.
 *
 * Every session sent is counted exactly once: in a drawn row (a collapsed
 * group's members included), in `more` (top-level rows past the cap, with
 * every session under them),
 * in `checking` (left out while its own check is unfinished, as the lists
 * count it) or in `creator` (any other session left out). So the drawn rows'
 * time, `more`, `checking` and `creator` add up to the day's agent time.
 */
export function dayList(
  answer: DashboardDaySessions,
  open: ReadonlySet<string>,
): {
  rows: DayRow[];
  /** Left out by the cap: top-level `rows`, and the `sessions` in them, with their time. */
  more: DayCount & { rows: number };
  checking: DayCount;
  creator: DayCount;
  /** Whether any session was placed under another. */
  grouped: boolean;
} {
  const lanes: DayLane[] = answer.sessions.map((session) => ({
    key: sessionKey(session.host, session.session_id),
    sessionId: session.session_id,
    host: session.host,
    session,
  }));
  // A session's own context is its entry; a parent that is not one of the
  // day's sessions has a context-only entry. One host's is never another's.
  const context = new Map<string, ChildContext>([
    ...answer.referenced_parents.map(
      (entry) => [sessionKey(entry.host, entry.session_id), entry] as const,
    ),
    ...lanes.map((lane) => [lane.key, lane.session] as const),
  ]);
  const find = (id: string, host: string) => context.get(sessionKey(host, id));
  const parentOf = (lane: DayLane) =>
    verifiedParent({ id: lane.sessionId, parent: lane.session.parent });
  const listed = listedLanes(lanes, parentOf, unresolvedIn(find));
  const kept = new Set(listed.map((lane) => lane.key));
  const checking: DayCount = { sessions: 0, ms: 0 };
  const creator: DayCount = { sessions: 0, ms: 0 };
  for (const lane of lanes) {
    if (kept.has(lane.key)) continue;
    const bucket = stillChecking(find(lane.sessionId, lane.host), lane.host) ? checking : creator;
    bucket.sessions += 1;
    bucket.ms += lane.session.agent_ms;
  }
  const { rows: all, grouped } = laneRows(listed, parentOf, open);
  const rows: DayRow[] = [];
  const more = { rows: 0, sessions: 0, ms: 0 };
  let top = 0;
  for (const row of all) {
    if (row.depth === 0) top += 1;
    if (top <= DAY_ROW_LIMIT) {
      rows.push(row);
      continue;
    }
    if (row.depth > 0) continue;
    const group = rowGroup(row);
    more.rows += 1;
    more.sessions += group.length;
    more.ms += dayMs(group);
  }
  return { rows, more, checking, creator, grouped };
}

/** The given sessions' time that day, added up. */
export const dayMs = (lanes: readonly DayLane[]) =>
  lanes.reduce((sum, lane) => sum + lane.session.agent_ms, 0);
