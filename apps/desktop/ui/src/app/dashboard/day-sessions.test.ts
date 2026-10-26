import { expect, it } from 'vitest';
import type { DashboardDaySession } from '../../data/generated/DashboardDaySession';
import type { DashboardDaySessions } from '../../data/generated/DashboardDaySessions';
import type { SessionParentLink } from '../../data/generated/SessionParentLink';
import { DAY_ROW_LIMIT, dayList, dayMs, type DayRow } from './day-sessions';
import { absentKey, rowGroup, sessionKey, showsTotal } from './lanes';

const M = 60_000;
const link = (session_id: string, title: string | null = null): SessionParentLink => ({
  session_id,
  host: 'codex',
  title,
  evidence: 'native_spawn',
});
/** One of the day's sessions: checked and no child unless said otherwise. */
const session = (
  session_id: string,
  minutes: number,
  context: Partial<DashboardDaySession> = {},
): DashboardDaySession => ({
  session_id,
  host: 'codex',
  title: null,
  automated_review: false,
  agent_ms: minutes * M,
  last_activity_at_ms: 1,
  parent: null,
  known_child: false,
  child_check: 'checked',
  ...context,
});
const child = (id: string, minutes: number, parent: string) =>
  session(id, minutes, { parent: link(parent), known_child: true, child_check: 'child' });
const answer = (
  sessions: DashboardDaySession[],
  extra: Partial<DashboardDaySessions> = {},
): DashboardDaySessions => {
  return {
    date: '2026-09-07',
    start_ms: 0,
    end_ms: 86_400_000,
    agent_ms: sessions.reduce((sum, s) => sum + s.agent_ms, 0),
    sessions,
    referenced_parents: [],

    ...extra,
  };
};
/** What one drawn row shows: its key's session (or absent parent), depth and time. */
const shown = (row: DayRow) => ({
  id: row.kind === 'session' ? row.lane.sessionId : `absent ${row.parent.session_id}`,
  depth: row.depth,
  children: row.children,
  minutes:
    (showsTotal(row)
      ? dayMs(rowGroup(row))
      : row.kind === 'session'
        ? row.lane.session.agent_ms
        : 0) / M,
});
/** Everything the box says adds up to the day's agent time. */
const accounted = (day: DashboardDaySessions, open: ReadonlySet<string> = new Set()) => {
  const list = dayList(day, open);
  const drawn = list.rows.reduce(
    (sum, row) =>
      sum +
      (showsTotal(row)
        ? dayMs(rowGroup(row))
        : row.kind === 'session'
          ? row.lane.session.agent_ms
          : 0),
    0,
  );
  return drawn + list.more.ms + list.checking.ms + list.creator.ms;
};

// The day the unit and browser tests share in spirit: a parent with two
// sub-sessions, two sub-sessions of a parent with no time that day, a known
// sub-session whose creator is not verified (and one under it), a session
// still being checked, and seven ordinary sessions.
const DAY = answer(
  [
    session('p', 30),
    child('k1', 25, 'gone'),
    child('c1', 20, 'p'),
    session('u', 15, { known_child: true, child_check: 'child' }),
    session('x', 10, { child_check: 'checking' }),
    child('c2', 5, 'p'),
    child('w', 3, 'u'),
    child('k2', 2, 'gone'),
    ...Array.from({ length: 7 }, (_, n) => session(`o-${n + 1}`, 1)),
  ],
  {
    referenced_parents: [
      { session_id: 'gone', host: 'codex', known_child: false, child_check: 'checked' },
    ],
  },
);

it('lists a day’s sessions by the Sessions section’s rules: sub-sessions collapsed under their parent, an absent parent named, the rest counted', () => {
  const list = dayList(DAY, new Set());
  expect(list.rows.map(shown)).toEqual([
    // A collapsed group shows its sessions' time added up.
    { id: 'p', depth: 0, children: 2, minutes: 55 },
    { id: 'absent gone', depth: 0, children: 2, minutes: 27 },
    ...Array.from({ length: 6 }, (_, n) => ({
      id: `o-${n + 1}`,
      depth: 0,
      children: 0,
      minutes: 1,
    })),
  ]);
  expect(list.grouped).toBe(true);
  // Eight top-level rows; the ninth is counted with its time.
  expect(list.more).toEqual({ rows: 1, sessions: 1, ms: M });
  // The session still being checked, counted as the lists count it, and the
  // unverified sub-session with the one under it.
  expect(list.checking).toEqual({ sessions: 1, ms: 10 * M });
  expect(list.creator).toEqual({ sessions: 2, ms: 18 * M });
  expect(accounted(DAY)).toBe(DAY.agent_ms);
});

it('opens a group under its parent, each row then showing its own time, and still adds up', () => {
  const p = sessionKey('codex', 'p');
  const gone = absentKey(sessionKey('codex', 'gone'));
  const list = dayList(DAY, new Set([p, gone]));
  expect(list.rows.slice(0, 6).map(shown)).toEqual([
    { id: 'p', depth: 0, children: 2, minutes: 30 },
    { id: 'c1', depth: 1, children: 0, minutes: 20 },
    { id: 'c2', depth: 1, children: 0, minutes: 5 },
    // An open absent row has no time of its own.
    { id: 'absent gone', depth: 0, children: 2, minutes: 0 },
    { id: 'k1', depth: 1, children: 0, minutes: 25 },
    { id: 'k2', depth: 1, children: 0, minutes: 2 },
  ]);
  // Opening a group never changes what the cap counts: still eight
  // top-level rows, the ninth left out.
  expect(list.rows.filter((row) => row.depth === 0)).toHaveLength(DAY_ROW_LIMIT);
  expect(list.more).toEqual({ rows: 1, sessions: 1, ms: M });
  expect(accounted(DAY, new Set([p, gone]))).toBe(DAY.agent_ms);
});

it('caps top-level rows, not sessions: a group counts once, and a group past the cap is counted with every session in it', () => {
  // Nine parents, each with two sub-sessions: 27 sessions, nine rows.
  const day = answer(
    Array.from({ length: 9 }, (_, n) => [
      session(`p-${n}`, 30 - n),
      child(`p-${n}-a`, 2, `p-${n}`),
      child(`p-${n}-b`, 1, `p-${n}`),
    ]).flat(),
  );
  const list = dayList(day, new Set());
  expect(list.rows).toHaveLength(DAY_ROW_LIMIT);
  expect(list.rows.every((row) => row.children === 2 && row.depth === 0)).toBe(true);
  // The ninth complete group is one more row containing three sessions.
  expect(list.more).toEqual({ rows: 1, sessions: 3, ms: (22 + 3) * M });
  expect(accounted(day)).toBe(day.agent_ms);
});

it('leaves out a session with no context, and the sub-sessions of a parent that is itself an unverified sub-session', () => {
  const day = answer(
    [
      session('a', 9, { child_check: null, known_child: null }),
      child('b', 4, 'hidden-parent'),
      session('c', 2),
    ],
    {
      // The parent had no time that day; its own context says it is a known
      // sub-session with no verified creator.
      referenced_parents: [
        { session_id: 'hidden-parent', host: 'codex', known_child: true, child_check: 'child' },
      ],
    },
  );
  const list = dayList(day, new Set());
  expect(list.rows.map(shown)).toEqual([{ id: 'c', depth: 0, children: 0, minutes: 2 }]);
  expect(list.checking).toEqual({ sessions: 1, ms: 9 * M });
  expect(list.creator).toEqual({ sessions: 1, ms: 4 * M });
  expect(list.grouped).toBe(false);
  expect(accounted(day)).toBe(day.agent_ms);
});

it('reads a parent’s context by exact host: another host’s session of the same ID is not it', () => {
  const day = answer([
    // The Claude session shares the parent's ID but is not it, so the
    // Codex child sits under an absent parent with no context: left out.
    { ...session('same', 8), host: 'claude' },
    child('kid', 3, 'same'),
  ]);
  const list = dayList(day, new Set());
  expect(list.rows.map(shown)).toEqual([{ id: 'same', depth: 0, children: 0, minutes: 8 }]);
  expect(list.checking).toEqual({ sessions: 0, ms: 0 });
  expect(list.creator).toEqual({ sessions: 1, ms: 3 * M });
});

it('groups more than 1,000 children with a lower-time parent before capping complete groups', () => {
  // The parent's own time sorts after 1,001 children. It must still be a
  // measured parent row, and every child must contribute to its total.
  const children = Array.from({ length: 1_001 }, (_, n) => child(`c-${n}`, 2, 'late-parent'));
  const day = answer([
    ...children,
    ...Array.from({ length: 8 }, (_, n) => session(`other-${n}`, 2)),
    session('late-parent', 1),
  ]);
  const collapsed = dayList(day, new Set());
  expect(collapsed.rows).toHaveLength(DAY_ROW_LIMIT);
  expect(collapsed.rows[0]!.kind).toBe('session');
  expect(shown(collapsed.rows[0]!)).toEqual({
    id: 'late-parent',
    depth: 0,
    children: 1_001,
    minutes: 2_003,
  });
  expect(collapsed.more).toEqual({ rows: 1, sessions: 1, ms: 2 * M });
  const open = new Set([sessionKey('codex', 'late-parent')]);
  const expanded = dayList(day, open);
  expect(expanded.rows.filter((row) => row.depth === 1)).toHaveLength(1_001);
  expect(expanded.more).toEqual(collapsed.more);
  expect(accounted(day)).toBe(day.agent_ms);
  expect(accounted(day, open)).toBe(day.agent_ms);
});
