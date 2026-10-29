import { Popover } from '@base-ui/react/popover';
import { useQuery } from '@tanstack/react-query';
import { Tooltip } from '@base-ui/react/tooltip';
import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
} from 'react';
import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { DashboardWindow } from '../../data/generated/DashboardWindow';
import { UNMEASURED } from '../../kit/format';
import { Icon } from '../../kit/icons';
import { RulePopover } from '../../kit/RulePopover';
import type { TimeRange } from '../../kit/TopBar';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import { agentTime } from '../agent-duration';
import { leverageText } from '../metric-format';
import { DaySessions } from './DaySessions';
import { useData } from '../../data/DataProvider';
import type { DashboardDayWarning } from '../../data/generated/DashboardDayWarning';
import type { DashboardWarningDay } from '../../data/generated/DashboardWarningDay';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { queryKeys } from '../../data/query-client';
import { displayTitle, shortId } from '../session-context';
import { HostGlyph } from '../../kit/HostGlyph';
import { DiagnosisStatusLine, useDiagnosisStatus, withDiagnosisStatus } from '../diagnosis-status';
import { WarningDialog } from './WarningDialog';
import { useSessionTitles } from '../session-titles';
import '../../styles/session-warnings.css';
import {
  clockHours,
  QUARTERS,
  LINE_HEIGHT,
  LINE_WIDTH,
  AT_OR_BELOW_EVEN,
  agentLines,
  timelineRows,
  type AgentLine,
  type TimelineRow,
} from './overview';
import { timelineRowHeight } from './timeline-rows';
import '../../styles/overview.css';

/** The totals columns whose widths follow their widest text. */
const TOTAL_COLUMNS = ['you', 'agents', 'leverage'] as const;
const HOURS = ['00', '06', '12', '18', '24'];
/**
 * How far from a warning dot's centre, in pixels and in any direction, the
 * pointer is over it: the dot's radius and a little more, so a 7px dot is
 * easy to point at and dots drawn over one another are all under the pointer
 * together. A pointer elsewhere on the track, above or below a dot, is not.
 */
const DOT_REACH_PX = 5;

/**
 * A report's session warnings for "human + agent": each day's native answer
 * by its date, and whether the read failed, so a failed read is never drawn
 * as days without warnings.
 */
export type DayWarnings = {
  byDate: ReadonlyMap<string, DashboardWarningDay>;
  failed: boolean;
  /** What names a warning's session, as the session rows and day card name it. */
  nameOf: (entry: DashboardDayWarning) => string;
};

/**
 * "human + agent": the timeline with the agents' line and each day's session
 * warnings, read for this report.
 */
export function PairedHumanTimeline({
  report,
  range,
}: {
  report: DashboardMetrics;
  range: TimeRange;
}) {
  const warnings = useDayWarnings(report);
  return <HumanTimeline report={report} range={range} withAgents warnings={warnings} />;
}

/**
 * The warnings inside the report's whole days, by day, for "human + agent":
 * the native answer for this report's whole days in its zone, each session
 * read over its whole history, so a warning here is the same warning the
 * session's rows and page show. Read only while that view is drawn, and kept
 * until committed data changes (`metrics`).
 *
 * The answer covers whole local days, so it is keyed by those days' bounds,
 * not by the report's end: a live report after each finished turn has a new
 * end but, until the day changes, the same days and the same answer. Ingest
 * events re-read it.
 */
export function useDayWarnings(report: DashboardMetrics, enabled = true) {
  const { source } = useData();
  const { window } = report;
  const { start_ms: firstStart, end_ms: lastEnd } = report.human_hours.current;
  const query = useQuery({
    queryKey: queryKeys.windowWarnings(window.days, firstStart, lastEnd, window.timezone),
    // The command names the report by its range, end and zone, as a day's
    // sessions are named; any end inside these days names these days.
    queryFn: () => source.windowWarnings!(window.days, window.end_ms, window.timezone),
    enabled: enabled && Boolean(source.windowWarnings),
    staleTime: Infinity,
    // When the days move on (a new day), the dots stay where the same range's
    // last answer in the same zone put them until the new answer is in. A
    // reason's clock times are written in its zone, so another zone's answer
    // is never shown in their place.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === window.days && previousQuery.queryKey[5] === window.timezone
        ? previous
        : undefined,
  });
  // Each warned session is named as its lane and the day card name it: the
  // host's own title read for its latest record, else the saved title, else
  // the Guardian hint, else "Session" and its short identifier.
  const titleRows = useMemo(() => {
    const rows = new Map<string, number>();
    for (const day of query.data?.days ?? [])
      for (const entry of day.warnings) rows.set(entry.session_id, entry.last_ms);
    return [...rows].map(([id, version]) => ({ id, version }));
  }, [query.data]);
  const hostTitle = useSessionTitles(
    JSON.stringify(['effort-warnings', window.days, window.timezone]),
    titleRows,
    { keepResolved: true },
  );
  return useMemo<DayWarnings>(
    () => ({
      byDate: new Map((query.data?.days ?? []).map((day) => [day.date, day])),
      failed: query.isError && !query.data,
      nameOf: (entry) =>
        displayTitle(
          hostTitle(entry.session_id, entry.last_ms) ?? entry.title,
          entry.automated_review,
        ) ?? `Session ${shortId(entry.session_id)}`,
    }),
    [query.data, query.isError, hostTitle],
  );
}

/**
 * Human time as a 24-hour timeline: one row per reported local day, oldest
 * first, each a whole day with its stretches drawn across its hours and its
 * total at the right. Rows keep the height eight rows have in the plot, so a
 * 14- or 30-day range scrolls instead of shrinking; the hour axis stays above
 * the rows.
 *
 * It opens on today, at the bottom, when it first appears (switching to
 * "human h" mounts it) and when the range changes. A refresh of the same range
 * (live updates send a new report after every finished turn) leaves the
 * scroll position alone. When the rows change height with the card, a view
 * that was at the bottom stays there and any other keeps the same rows in view.
 *
 * The timeline is one tab stop: one day row at a time (today at first). Up and
 * Down move to the day before or after, Home and End to the first and last;
 * a focused day opens its card, as hovering does. In "human + agent" a day's
 * date also opens a box of its sessions, each a link: hovering the date opens
 * it, and Enter on the focused day moves focus into it; Escape returns to the
 * day.
 */
export function HumanTimeline({
  report,
  range,
  withAgents = false,
  warnings: dayWarnings,
}: {
  report: DashboardMetrics;
  /** The Dashboard's range, which a day card's session links carry. */
  range: TimeRange;
  /**
   * The "human + agent" view: each day's agent time as a thin line across
   * its track, as high as the agents running on average in each wall-clock
   * quarter-hour on the report's one scale, over soft bands where you were
   * working.
   */
  withAgents?: boolean;
  /** "human + agent" only: each day's session warnings, by date. */
  warnings?: DayWarnings;
}) {
  const human = report.human_hours.current;
  const rows = useMemo(() => timelineRows(human.by_day, report.window), [human, report.window]);
  const lines = useMemo(
    () =>
      withAgents ? agentLines(report.leverage.by_day, report.leverage.agents_scale_top) : null,
    [withAgents, report.leverage],
  );
  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<{
    entry: DashboardDayWarning;
    name: string;
    date: string;
    timezone: string;
  } | null>(null);
  const warningFocus = useRef<HTMLElement | null>(null);
  // A refreshed answer can remove the pressed dot while its detail stays open.
  // Return to that day, or the remaining current day, when the dot has gone.
  useLayoutEffect(() => {
    if (!selected || warningFocus.current?.isConnected) return;
    const days = [
      ...(body.current?.querySelectorAll<HTMLElement>('[data-testid="human-day"]') ?? []),
    ];
    warningFocus.current =
      days.find((day) => day.dataset.date === selected.date) ??
      days.find((day) => day.tabIndex === 0) ??
      root.current;
  });
  // No total is capped: a busy day's agent hours or a tiny day's leverage
  // can be far wider than usual. Each totals column is as wide as its
  // widest text in any row, never narrower than its usual width (in CSS);
  // the axis and every row read the same widths, so they stay aligned.
  useLayoutEffect(() => {
    const element = root.current;
    if (!withAgents || !element) return;
    let live = true;
    const fit = () => {
      if (!live) return;
      for (const key of TOTAL_COLUMNS) {
        const widths = [
          ...element.querySelectorAll<HTMLElement>(
            `.xt-human-total[data-key="${key}"] > .xt-human-total-text`,
          ),
        ].map((text) => text.getBoundingClientRect().width);
        element.style.setProperty(`--${key}-col`, `${Math.ceil(Math.max(0, ...widths))}px`);
      }
    };
    fit();
    // Measured again once the mono font has loaded.
    void document.fonts?.ready.then(fit);
    return () => {
      live = false;
    };
  }, [withAgents, rows, lines]);
  const [rowHeight, setRowHeight] = useState<number | null>(null);
  const [above, setAbove] = useState(false);
  /** The range the view last opened on today for; `null` before the first. */
  const openedFor = useRef<number | null>(null);
  const drawnRowHeight = useRef<number | null>(null);
  const atBottom = useRef(true);
  const days = report.window.days;
  /** The day that takes the timeline's tab stop; today until another is chosen. */
  const [chosen, setChosen] = useState<string | null>(null);
  const found = rows.findIndex((row) => row.date === chosen);
  const current = found >= 0 ? found : rows.length - 1;
  /**
   * The one day whose sessions box is open, in "human + agent". Opening a
   * day's box closes any other at once, without waiting for the pointer to
   * move: a hover popup only notices the pointer has left when it moves.
   */
  const [sessionsDay, setSessionsDay] = useState<string | null>(null);
  // A box whose day is no longer drawn (a refresh moved the range on, or the
  // view left "human + agent") is closed, never reopened when that day comes
  // back with no pointer or focus on it.
  if (sessionsDay !== null && (!withAgents || !rows.some((row) => row.date === sessionsDay)))
    setSessionsDay(null);
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    // Keys pressed inside a day's sessions box (it is portalled, but its
    // events still pass through here) are the box's own.
    if (!(event.target instanceof HTMLElement) || !event.target.classList.contains('xt-human-row'))
      return;
    const next = {
      ArrowUp: current - 1,
      ArrowDown: current + 1,
      Home: 0,
      End: rows.length - 1,
    }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    const index = Math.max(0, Math.min(rows.length - 1, next));
    setChosen(rows[index]!.date);
    body.current?.querySelectorAll<HTMLElement>('[data-testid="human-day"]')[index]?.focus();
  };
  // Size the rows from the space eight of them have, and follow the card.
  useLayoutEffect(() => {
    const element = body.current;
    if (!element) return;
    const fit = () => setRowHeight(timelineRowHeight(element.clientHeight, withAgents));
    fit();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [withAgents]);
  // Open on today, the last row, for each new range; follow a change of row
  // height. Nothing here runs for a new report of the same range.
  useLayoutEffect(() => {
    const element = body.current;
    if (!element || rowHeight === null) return;
    const previous = drawnRowHeight.current;
    if (openedFor.current !== days) {
      openedFor.current = days;
      element.scrollTop = element.scrollHeight;
    } else if (previous !== null && previous !== rowHeight) {
      element.scrollTop = atBottom.current
        ? element.scrollHeight
        : Math.round((element.scrollTop * rowHeight) / previous);
    }
    drawnRowHeight.current = rowHeight;
    atBottom.current = isAtBottom(element);
    setAbove(element.scrollTop > 2);
  }, [days, rowHeight]);
  const unknown = human.active_ms === null;
  return (
    <div
      ref={root}
      tabIndex={-1}
      className="xt-effort xt-human"
      data-testid="human-timeline"
      data-variant={withAgents ? 'paired' : undefined}
    >
      {withAgents ? (
        <PairedHeadline report={report} warningsFailed={dayWarnings?.failed ?? false} />
      ) : (
        <p className="xt-effort-headline" data-testid="effort-headline">
          <strong data-testid="effort-total">
            {unknown ? 'human time unknown' : `${agentTime(human.active_ms!)} human`}
          </strong>
          {unknown && <small>Could not tell which messages a person sent.</small>}
          <small className="xt-effort-headline-end">
            break threshold: {human.break_minutes} min
          </small>
        </p>
      )}
      <div className="xt-human-plot">
        <div className="xt-human-axis">
          <span />
          <span className="xt-human-hours" aria-hidden="true">
            {HOURS.map((hour) => (
              <span key={hour}>{hour}</span>
            ))}
          </span>
          {withAgents ? (
            <>
              <RulePopover ruleId="M-08" text="Human hrs">
                <span
                  className="xt-human-total-head"
                  data-testid="human-total-head"
                  role="img"
                  aria-label="Human hrs"
                >
                  <Icon name="person" size={13} />
                </span>
              </RulePopover>
              <RulePopover ruleId="M-05" text="Agent hrs">
                <span
                  className="xt-human-total-head"
                  data-key="agents"
                  data-testid="agent-total-head"
                  role="img"
                  aria-label="Agent hrs"
                >
                  <Icon name="bot" size={13} />
                </span>
              </RulePopover>
              <RulePopover ruleId="M-08" text="Leverage">
                <span
                  className="xt-human-total-head"
                  data-key="leverage"
                  data-testid="leverage-total-head"
                  role="img"
                  aria-label="Leverage"
                >
                  <Icon name="bolt" size={13} />
                </span>
              </RulePopover>
            </>
          ) : (
            <span />
          )}
        </div>
        <div
          ref={body}
          className="xt-human-body"
          role="list"
          aria-label={`${withAgents ? 'Human and agent time' : 'Human time'} by day, ${rows.length} days, each from 00:00 to 24:00`}
          data-more-above={above || undefined}
          data-testid="human-timeline-body"
          onKeyDown={move}
          onScroll={(event) => {
            atBottom.current = isAtBottom(event.currentTarget);
            setAbove(event.currentTarget.scrollTop > 2);
          }}
          style={
            rowHeight === null ? undefined : ({ '--row-h': `${rowHeight}px` } as CSSProperties)
          }
        >
          {rows.map((row, index) => (
            <Row
              key={row.date}
              row={row}
              line={lines?.get(row.date) ?? (lines ? null : undefined)}
              warnings={withAgents ? dayWarnings?.byDate.get(row.date) : undefined}
              nameOf={dayWarnings?.nameOf}
              warningOpen={selected !== null}
              onWarning={(entry, button, name) => {
                warningFocus.current = button;
                setSessionsDay(null);
                setSelected({ entry, name, date: row.date, timezone: report.window.timezone });
              }}
              lower={index >= rows.length / 2}
              current={index === current}
              onFocus={() => setChosen(row.date)}
              sessionsOpen={sessionsDay === row.date}
              onSessions={(open) =>
                setSessionsDay((day) => (open ? row.date : day === row.date ? null : day))
              }
              window={report.window}
              range={range}
            />
          ))}
        </div>
      </div>
      <WarningDialog
        selected={
          selected && {
            warning: selected.entry.warning,
            name: selected.name,
            sessionId: selected.entry.session_id,
            host: selected.entry.host,
            timezone: selected.timezone,
          }
        }
        onClose={() => setSelected(null)}
        returnFocusRef={warningFocus}
      />
    </div>
  );
}

/**
 * The "human + agent" headline: your time and the agents' over the same
 * whole days, each behind a key drawn like its line, then the leverage tile's
 * own value. Unknown human time says so, as the human h headline does.
 */
function PairedHeadline({
  report,
  warningsFailed,
}: {
  report: DashboardMetrics;
  /** The report's warnings could not be read: said, so no day reads as warning-free. */
  warningsFailed: boolean;
}) {
  const human = report.human_hours.current;
  const unknown = human.active_ms === null;
  const leverage = report.tiles.leverage.value;
  return (
    <p className="xt-effort-headline xt-paired-headline" data-testid="effort-headline">
      <span className="xt-paired-stat">
        <span className="xt-paired-key" data-key="you" role="img" aria-label="Your time">
          <Icon name="person" size={16} />
          <span className="xt-paired-swatch" data-key="band" />
        </span>
        <strong data-testid="effort-total">
          {unknown ? 'human time unknown' : agentTime(human.active_ms!)}
        </strong>
      </span>
      <span className="xt-paired-stat">
        <span className="xt-paired-key" data-key="agents" role="img" aria-label="Agent time">
          <Icon name="bot" size={16} />
          <svg className="xt-paired-swatch" data-key="line" viewBox="0 0 22 12" focusable="false">
            <path d="M0,10 L5,9 L8,4 L12,6 L16,2 L22,7" />
          </svg>
        </span>
        <strong data-testid="effort-agent-total">{agentTime(report.leverage.agent_ms)}</strong>
      </span>
      {unknown && <small>Could not tell which messages a person sent.</small>}
      <span className="xt-paired-stat" data-stat="leverage">
        <strong data-testid="effort-leverage">
          {leverage === null ? UNMEASURED : leverageText(leverage)}
        </strong>
        <small>agent time per hour of yours</small>
      </span>
      {warningsFailed && (
        <small className="xt-warnings-unread" role="status" data-testid="effort-warnings-unread">
          {SESSION_WARNINGS_TEXT.window_unread}
        </small>
      )}
    </p>
  );
}

/** Within two pixels of the end: the newest row is in view. */
const isAtBottom = (element: HTMLElement) =>
  element.scrollTop + element.clientHeight >= element.scrollHeight - 2;

/**
 * One day: a list item whose name lists every stretch, and whose card opens
 * on hover or focus. Only the current day is in the tab order; arrow keys
 * move between days. Each bar also names its own times.
 *
 * In "human + agent" the date also opens the day's sessions box, a list of
 * links, on hover; the pointer can move from the date into it. Enter or
 * Space on the focused day opens it with focus on its first session, and
 * Escape closes it and returns focus to the day. A click on the date does
 * nothing, so it never leaves a box pinned open. Only one of the day's two
 * boxes shows at a time: the card waits until the sessions box has gone.
 *
 * Each warning dot shows its own warning: pointing at it shows the session's
 * name, the warning's day, label and reason (every dot under the pointer when
 * they overlap) in place of the day's card, and clicking opens the first of
 * them in a detail modal on the Dashboard. From the focused day, Right arrow moves focus
 * to its first dot, Left and Right between its dots, and Left from the first
 * or Escape back to the day; Enter on a dot opens it.
 *
 * Whether the box is open is the timeline's, which keeps one open at most.
 */
function Row({
  row,
  line,
  warnings: warned,
  nameOf = (entry) => `Session ${shortId(entry.session_id)}`,
  warningOpen,
  onWarning,
  lower,
  current,
  onFocus,
  sessionsOpen,
  onSessions,
  window,
  range,
}: {
  row: TimelineRow;
  /** The day's agents line in the "human + agent" view; `null` there when the report has none. */
  line?: AgentLine | null;
  /** The day's session warnings in the "human + agent" view: the native answer for that date. */
  warnings?: DashboardWarningDay;
  /** What names a warning's session; see {@link DayWarnings}. */
  nameOf?: (entry: DashboardDayWarning) => string;
  warningOpen: boolean;
  onWarning: (entry: DashboardDayWarning, button: HTMLButtonElement, name: string) => void;
  lower: boolean;
  current: boolean;
  onFocus: () => void;
  /** The day's sessions box is open: the timeline's one open box. */
  sessionsOpen: boolean;
  /** Ask to open or close the day's sessions box. */
  onSessions: (open: boolean) => void;
  /** The report's window, which names the day's sessions read. */
  window: DashboardWindow;
  /** The Dashboard's range, which the day's session links carry. */
  range: TimeRange;
}) {
  const theme = useSurfaceTheme();
  const paired = line !== undefined;
  /** The quarter under the pointer, and where along the track it is, in pixels. */
  const [pointer, setPointer] = useState<{ quarter: number; x: number } | null>(null);
  /** The pointer is on the track: the hairline shows. */
  const [onTrack, setOnTrack] = useState(false);
  /** Hover or focus asks for the day's card. */
  const [card, setCard] = useState(false);
  /** The sessions box is open or still closing; the card waits until it has gone. */
  const [sessionsShown, setSessionsShown] = useState(false);
  /** The warning dots under the pointer, by their place in the day's list. */
  const [pointed, setPointed] = useState<readonly number[]>([]);
  /** The warning dot that has focus, by its place in the day's list. */
  const [focusedDot, setFocusedDot] = useState<number | null>(null);
  const dots = useRef<(HTMLButtonElement | null)[]>([]);
  const dotAt = (entry: DashboardDayWarning) => clockHours(entry.warning.at_ms, window) / 24;
  /** The dots whose warnings show in place of the day's card. */
  const shownDots = warningOpen ? [] : focusedDot !== null ? [focusedDot] : pointed;
  const focusDot = (index: number) => dots.current[index]?.focus();
  const day = useRef<HTMLElement | null>(null);
  const dayRef = useCallback((element: HTMLElement | null) => {
    day.current = element;
  }, []);
  const popup = useRef<HTMLDivElement | null>(null);
  /** Enter asked for focus on the first session, which may not be read yet. */
  const entering = useRef(false);
  const enter = (box: HTMLElement) => {
    const link = box.querySelector<HTMLElement>('a[href]');
    if (link) {
      entering.current = false;
      link.focus();
    } else box.focus();
  };
  const popupRef = useCallback((element: HTMLDivElement | null) => {
    popup.current = element;
    if (element && entering.current) enter(element);
  }, []);
  // The sessions arrive after the box opened from the keyboard: focus moves
  // from the box onto the first of them.
  const listRef = useCallback((list: HTMLElement | null) => {
    if (list && entering.current && document.activeElement === popup.current) enter(list);
  }, []);
  // Another day's box opening closes this one: focus inside it goes back to
  // the day rather than being lost with the box.
  useLayoutEffect(() => {
    if (sessionsOpen) return;
    entering.current = false;
    if (popup.current?.contains(document.activeElement)) day.current?.focus();
  }, [sessionsOpen]);
  const track = (
    <span
      className="xt-human-track"
      data-fold={row.fold || undefined}
      data-on-warning={pointed.length > 0 || undefined}
      onClick={(event) => {
        // A press on the dots under the pointer opens the first of them,
        // through its own button; a press on the button is handled there.
        const first = dots.current[pointed[0] ?? -1];
        if (!first || (event.target instanceof Element && event.target.closest('button'))) return;
        first.click();
      }}
      onPointerMove={
        paired
          ? (event) => {
              const box = event.currentTarget.getBoundingClientRect();
              if (box.width <= 0) return;
              const x = Math.max(0, Math.min(box.width - 1, event.clientX - box.left));
              setPointer({ quarter: Math.floor((x / box.width) * QUARTERS), x });
              setOnTrack(true);
              // Every dot is centred on the track's middle line.
              const dy = event.clientY - (box.top + box.height / 2);
              const under = (warned?.warnings ?? []).flatMap((entry, index) =>
                Math.hypot(dotAt(entry) * box.width - (event.clientX - box.left), dy) <=
                DOT_REACH_PX
                  ? [index]
                  : [],
              );
              setPointed((before) => (before.join() === under.join() ? before : under));
            }
          : undefined
      }
      onPointerLeave={
        paired
          ? (event) => {
              setOnTrack(false);
              setPointed([]);
              // Onto the day's totals the card stays open and drops the
              // quarter now. Off the day, or onto its date, the card closes
              // and keeps it until it has gone: a card whose content changed
              // as it closed would resize under the positioner's own size
              // watcher (a ResizeObserver loop in WebKit).
              const to = event.relatedTarget;
              if (
                to instanceof Element &&
                event.currentTarget.parentElement?.contains(to) &&
                !to.closest('.xt-human-day')
              )
                setPointer(null);
            }
          : undefined
      }
    >
      {row.bars.map((bar, index) => (
        <span
          key={index}
          role="img"
          aria-label={`${row.label}, ${bar.tick ? `one message at ${bar.text}` : bar.text}`}
          className={bar.pieces ? 'xt-human-stretch' : 'xt-human-bar'}
          data-tick={bar.tick || undefined}
          data-testid="human-stretch"
          style={
            bar.pieces
              ? undefined
              : {
                  left: `${bar.left * 100}%`,
                  width: bar.tick ? undefined : `${bar.width * 100}%`,
                }
          }
        >
          {bar.pieces?.map((piece, part) => (
            <span
              key={part}
              aria-hidden="true"
              className="xt-human-bar"
              data-lane={piece.lane}
              data-tick={bar.tick || undefined}
              style={{
                left: `${piece.left * 100}%`,
                width: bar.tick ? undefined : `${piece.width * 100}%`,
              }}
            />
          ))}
        </span>
      ))}
      {paired && line?.path && (
        <svg
          className="xt-agent-line"
          data-testid="agent-line"
          viewBox={`0 0 ${LINE_WIDTH} ${LINE_HEIGHT}`}
          preserveAspectRatio="none"
          aria-hidden="true"
          focusable="false"
        >
          <path d={line.path} vectorEffect="non-scaling-stroke" />
        </svg>
      )}
      {warned?.warnings.map((entry, index) => (
        // The day is the tab stop; Right arrow reaches each warning button.
        <DayWarningDot
          key={`${entry.session_id}:${entry.warning.id}`}
          entry={entry}
          name={nameOf(entry)}
          buttonRef={(element) => {
            dots.current[index] = element;
          }}
          onClick={(event) => onWarning(entry, event.currentTarget, nameOf(entry))}
          pointed={shownDots.includes(index)}
          left={dotAt(entry)}
          onFocus={() => setFocusedDot(index)}
          onBlur={() => setFocusedDot((current) => (current === index ? null : current))}
          onKeyDown={(event) => {
            if (warningOpen) return;
            const next = {
              ArrowRight: index + 1,
              ArrowLeft: index - 1,
              Escape: -1,
            }[event.key];
            if (next === undefined) return;
            event.preventDefault();
            event.stopPropagation();
            if (next >= 0 && next < (warned?.warnings.length ?? 0)) focusDot(next);
            else if (next < 0) day.current?.focus();
          }}
        />
      ))}
      {pointer && onTrack && (
        <span
          className="xt-human-hair"
          data-testid="agent-hairline"
          aria-hidden="true"
          style={{ left: `${pointer.x}px` }}
        />
      )}
    </span>
  );
  // The day's warnings in its name: the native count, each listed warning's
  // session name (named here, as the rows name sessions) and its native
  // words, then the native "and N more". Only the separators are written here.
  const listed = warned?.warnings.slice(0, warned.listed) ?? [];
  const warnedName = warned
    ? `; ${warned.count_label}: ${listed.map((entry) => `${nameOf(entry)}, ${entry.text}`).join('; ')}${
        warned.more_label ? `; ${warned.more_label}` : ''
      }`
    : '';
  const name = paired
    ? `${row.name}; agents: ${line ? `${line.spoken}${line.busiest ? `, busiest 15 minutes at ${line.busiest}` : ''}` : 'unknown'}; agent time per hour of yours: ${line?.leverageWords ?? 'unknown'}${warnedName}; Enter opens the day’s sessions${warned ? '; Right arrow moves to its warnings' : ''}`
    : row.name;
  const sessionsName = `Sessions on ${row.label}`;
  const dayHead = (
    <span className="xt-effort-tip-head">
      <span>{row.label}</span>
      <b>{row.total ?? 'unknown'}</b>
    </span>
  );
  /**
   * The whole day's lines: the same wherever the pointer is on it. With
   * agents the times you worked are left to the bands; without, they are the
   * card.
   */
  const daySummary = (
    <>
      {dayHead}
      {row.bars.length === 0 && (
        <span className="xt-effort-tip-note">
          {row.total === null ? 'Could not tell which messages a person sent' : 'No messages'}
        </span>
      )}
      {!paired &&
        row.bars.map((bar, index) => (
          <span key={index} className="xt-effort-tip-row">
            <span>{bar.text}</span>
            <em>{bar.tick ? 'one message' : ''}</em>
          </span>
        ))}
      {paired && (
        <>
          {row.bars.length === 0 && <span className="xt-effort-tip-divider" />}
          <span className="xt-effort-tip-row" data-agent="">
            <span>Agent time</span>
            <b data-testid="agent-day-total">{line ? line.total : 'unknown'}</b>
          </span>
          {line?.busiest && (
            <span className="xt-effort-tip-row">
              <span>Busiest 15 min</span>
              <em>{line.busiest}</em>
            </span>
          )}
          <span
            className="xt-effort-tip-row"
            data-low={line?.atOrBelowEven ? '' : undefined}
            data-testid="leverage-day"
          >
            <span>Agent time per hour of yours</span>
            <b>{line?.leverage ?? line?.leverageWords ?? 'unknown'}</b>
            {line?.atOrBelowEven && <em>{AT_OR_BELOW_EVEN}</em>}
          </span>
        </>
      )}
    </>
  );
  const dayCard = (
    <Tooltip.Root
      disableHoverablePopup
      open={card && !warningOpen && !sessionsShown && shownDots.length === 0}
      onOpenChange={(open) => setCard(open)}
      onOpenChangeComplete={(open) => {
        if (!open) setPointer(null);
      }}
    >
      <Tooltip.Trigger
        ref={dayRef}
        delay={0}
        closeOnClick={false}
        render={
          <div
            role="listitem"
            tabIndex={current ? 0 : -1}
            onFocus={onFocus}
            onKeyDown={
              paired
                ? (event) => {
                    if (event.target !== event.currentTarget) return;
                    if (event.key === 'ArrowRight' && dots.current[0]) {
                      event.preventDefault();
                      focusDot(0);
                      return;
                    }
                    if (event.key !== 'Enter' && event.key !== ' ') return;
                    event.preventDefault();
                    entering.current = true;
                    // An open box takes focus now; a closed (or closing) one
                    // opens and takes it once shown.
                    if (sessionsOpen && popup.current) enter(popup.current);
                    else {
                      setSessionsShown(true);
                      onSessions(true);
                    }
                  }
                : undefined
            }
            aria-label={name}
            className="xt-human-row"
            data-testid="human-day"
            data-date={row.date}
          />
        }
      >
        {paired ? (
          <Popover.Trigger
            nativeButton={false}
            openOnHover
            delay={0}
            // Not a tab stop: the day is, and Enter on it opens this box.
            tabIndex={-1}
            aria-label={sessionsName}
            // Focus that lands on the date (a click, or Shift+Tab out of the
            // box) belongs to the day.
            onFocus={() => day.current?.focus()}
            render={<span className="xt-human-day" data-weekend={row.weekend || undefined} />}
          >
            {row.label}
          </Popover.Trigger>
        ) : (
          <span className="xt-human-day" data-weekend={row.weekend || undefined}>
            {row.label}
          </span>
        )}
        {track}
        <span
          className="xt-human-total"
          data-key={paired ? 'you' : undefined}
          data-unknown={paired && row.total === null ? '' : undefined}
        >
          {paired ? (
            <span className="xt-human-total-text">{line?.yourHours ?? UNMEASURED}</span>
          ) : (
            (row.total ?? '—')
          )}
        </span>
        {paired && (
          <span className="xt-human-total" data-key="agents" data-testid="agent-row-total">
            <span className="xt-human-total-text">{line ? line.hours : UNMEASURED}</span>
          </span>
        )}
        {paired && (
          <span
            className="xt-human-total"
            data-key="leverage"
            data-unknown={line?.leverage ? undefined : ''}
            data-low={line?.atOrBelowEven ? '' : undefined}
            data-testid="leverage-row-total"
          >
            <span className="xt-human-total-text">{line?.leverage ?? UNMEASURED}</span>
          </span>
        )}
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side={lower ? 'top' : 'bottom'}
          align="center"
          sideOffset={4}
          collisionPadding={8}
        >
          <Tooltip.Popup
            className="xt-rule-popover xt-effort-tip"
            aria-hidden="true"
            data-testid="day-card"
          >
            {/* With agents the day's own lines are in its date's box; this
                card names the day and the 15 minutes under the pointer. */}
            {paired ? dayHead : daySummary}
            {paired && line && pointer && (
              <span className="xt-effort-tip-row" data-pointer="" data-testid="agent-quarter">
                <span>{line.quarter(pointer.quarter).span}</span>
                <em>{line.quarter(pointer.quarter).agents}</em>
              </span>
            )}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
  const pointedEntries = shownDots.flatMap((index) => warned?.warnings[index] ?? []);
  // The warnings under the pointer, or the focused dot's, by the first of
  // their dots, in place of the day's card.
  const dotTip = (
    <Tooltip.Root open={pointedEntries.length > 0} disableHoverablePopup>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          anchor={() => dots.current[shownDots[0] ?? -1] ?? null}
          side={lower ? 'top' : 'bottom'}
          align="center"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup
            className="xt-rule-popover xt-warning-tip xt-day-warning-tip"
            aria-hidden="true"
            data-testid="day-warning-tip"
          >
            {pointedEntries.map((entry) => (
              <DayWarningTipLine
                key={`${entry.session_id}:${entry.warning.id}`}
                entry={entry}
                name={nameOf(entry)}
              />
            ))}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
  if (!paired) return dayCard;
  return (
    <Popover.Root
      open={sessionsOpen && !warningOpen}
      onOpenChange={(open, details) => {
        // A click (or tap) on the date neither opens, pins nor closes the
        // box: a pinned box would stay open beside the next day's. Hover
        // opens it, and Enter or Space on the day for the keyboard.
        if (details.reason === 'trigger-press') {
          details.cancel();
          return;
        }
        if (open && warningOpen) return;
        if (open) setSessionsShown(true);
        // Escape from inside the box hands focus back to the day.
        else if (details.reason === 'escape-key' && popup.current?.contains(document.activeElement))
          day.current?.focus();
        onSessions(open);
      }}
      onOpenChangeComplete={(open) => {
        // A box reopened before it had gone keeps its element: focus moves
        // in here instead of when the element appears.
        if (open && entering.current && popup.current) enter(popup.current);
        if (!open) setSessionsShown(false);
      }}
    >
      {dayCard}
      {dotTip}
      <Popover.Portal data-theme={theme}>
        {/* Beside the date, over the track, and never flipped back over the
            dates: a box above or below its date would cover the next date
            and keep the pointer moving down the dates from reaching it. It
            grows away from the nearer end of the list and shifts only up or
            down to fit. */}
        <Popover.Positioner
          className="xt-rule-positioner xt-day-sessions-positioner"
          positionMethod="fixed"
          side="inline-end"
          align={lower ? 'end' : 'start'}
          sideOffset={4}
          collisionPadding={8}
          collisionAvoidance={{ side: 'none', align: 'shift', fallbackAxisSide: 'none' }}
        >
          <Popover.Popup
            ref={popupRef}
            className="xt-rule-popover xt-effort-tip"
            aria-label={sessionsName}
            initialFocus={false}
            finalFocus={false}
          >
            <div className="xt-day-summary" data-testid="day-summary">
              {daySummary}
            </div>
            <DaySessions
              window={window}
              date={row.date}
              label={row.label}
              range={range}
              listRef={listRef}
            />
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}

/**
 * One warning's dot on a day's track: a button in the danger ink, circled by
 * an arc while its warning is being diagnosed, and named with that state.
 */
function DayWarningDot({
  entry,
  name,
  buttonRef,
  pointed,
  left,
  ...handlers
}: {
  entry: DashboardDayWarning;
  name: string;
  buttonRef: (element: HTMLButtonElement | null) => void;
  pointed: boolean;
  left: number;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  onFocus: () => void;
  onBlur: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
}) {
  const status = useDiagnosisStatus(entry.session_id, entry.warning.id);
  return (
    <button
      ref={buttonRef}
      type="button"
      aria-haspopup="dialog"
      tabIndex={-1}
      className="xt-human-warning"
      data-testid="day-warning-dot"
      data-kind={entry.warning.kind}
      data-pointed={pointed || undefined}
      data-diagnosis={status?.kind}
      aria-label={withDiagnosisStatus(`${name}, ${entry.warning.name}`, status)}
      style={{ left: `${left * 100}%` }}
      {...handlers}
    />
  );
}

/**
 * One pointed warning in a day's hint: its session (the host's mark and the
 * session's name), the warning's label, its short reason when the native side
 * gives one (none for an admitted mistake, whose label says it) and, while one
 * is running or kept, its diagnosis.
 */
function DayWarningTipLine({ entry, name }: { entry: DashboardDayWarning; name: string }) {
  const status = useDiagnosisStatus(entry.session_id, entry.warning.id);
  return (
    <span
      className="xt-warning-pointed"
      data-kind={entry.warning.kind}
      data-testid="day-warning-tip-line"
    >
      <span className="xt-warning-tip-session">
        <HostGlyph host={entry.host} />
        <b className="xt-warning-session">{name}</b>
      </span>
      <span className="xt-warning-tip-kind">
        <span className="xt-warning-tip-dot" aria-hidden="true" />
        <b>{entry.warning.label}</b>
      </span>
      {entry.warning.compact_reason && (
        <span className="xt-warning-tip-reason">{entry.warning.compact_reason}</span>
      )}
      <DiagnosisStatusLine status={status} />
    </span>
  );
}
