import { Tooltip } from '@base-ui/react/tooltip';
import { Link } from 'react-router';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { WARNING_PARAM, sessionHref } from './session-search';
import '../styles/session-warnings.css';
import {
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryFilters,
} from '@tanstack/react-query';
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { useData } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../data/generated/SessionWarningsAnswer';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { queryKeys } from '../data/query-client';
import { DiagnosisStatusLine, useDiagnosisStatus, withDiagnosisStatus } from './diagnosis-status';

/** One session's cached answers, in every zone. */
function sessionFilter(sessionId: string): QueryFilters {
  const [scope, kind] = queryKeys.sessionWarnings('', '');
  return {
    predicate: ({ queryKey }) =>
      queryKey[0] === scope && queryKey[1] === kind && queryKey[3] === sessionId,
  };
}

/** Readbacks that concurrent warning writes may cancel and replace. */
const warningReadbacks = new WeakMap<QueryClient, Set<Promise<void>>>();

/**
 * After a dismissal or its undo (stored or refused), the answers that show
 * the session's warnings are read again: that session's in every zone, and
 * the Effort card's days. Nothing else is touched.
 *
 * A read already under way began before the change was stored, so it is
 * cancelled first and its answer never lands. Every such answer is then read
 * again, drawn now or not: they are kept until committed data changes
 * (`staleTime: Infinity`), so one left unread would show the old answer when
 * its screen comes back.
 */
export async function invalidateSessionWarnings(client: QueryClient, sessionId: string) {
  const [metrics, days] = queryKeys.windowWarnings(7, 0, 0, '');
  const filters = [sessionFilter(sessionId), { queryKey: [metrics, days] }];
  const pending = warningReadbacks.get(client) ?? new Set<Promise<void>>();
  warningReadbacks.set(client, pending);
  const readback = (async () => {
    await Promise.all(filters.map((filter) => client.cancelQueries(filter)));
    await Promise.all(
      filters.map((filter) => client.invalidateQueries({ ...filter, refetchType: 'all' })),
    );
  })();
  pending.add(readback);
  try {
    await readback;
  } finally {
    pending.delete(readback);
  }
  // Cancellation can settle this read before another press's replacement
  // answer arrives. Wait for those readbacks before inspecting the cache.
  while (pending.size > 0) await Promise.all(pending);
}

/**
 * Whether the session's answers, as cached now, list `warningId` among the
 * dismissed ones (`dismissed`) or among those not dismissed: every zone's
 * answer lists the same warnings.
 */
function listedAs(client: QueryClient, sessionId: string, warningId: string, dismissed: boolean) {
  return client
    .getQueriesData<SessionWarningsAnswer>(sessionFilter(sessionId))
    .some(
      ([, answer]) =>
        answer?.state === 'read' &&
        (dismissed ? answer.dismissed : answer.warnings).some((one) => one.id === warningId),
    );
}

/** Distinct sessions one native read names at most: the native limit. */
const BATCH = SESSION_WARNINGS_TEXT.max_sessions;

type Waiting = {
  resolve: (answer: SessionWarningsAnswer) => void;
  reject: (error: unknown) => void;
};

/**
 * Reads asked for in the same moment share one native read per zone: every
 * row of a table asks for its own session, and its answer is cached on its
 * own, so a scroll or a new page only reads the sessions not yet read.
 */
const batches = new WeakMap<DataSource, Map<string, Map<string, Waiting[]>>>();

function readWarnings(source: DataSource, timezone: string, id: string) {
  return new Promise<SessionWarningsAnswer>((resolve, reject) => {
    let zones = batches.get(source);
    if (!zones) {
      zones = new Map();
      batches.set(source, zones);
    }
    let waiting = zones.get(timezone);
    if (!waiting) {
      waiting = new Map();
      zones.set(timezone, waiting);
      const named = waiting;
      const all = zones;
      setTimeout(() => {
        all.delete(timezone);
        const ids = [...named.keys()];
        for (let at = 0; at < ids.length; at += BATCH) {
          const chunk = ids.slice(at, at + BATCH);
          const settle = (answer: (id: string) => SessionWarningsAnswer | undefined) => {
            for (const id of chunk) {
              const found = answer(id);
              for (const one of named.get(id) ?? [])
                if (found) one.resolve(found);
                else one.reject(new Error('The warnings read left this session out.'));
            }
          };
          source.sessionWarnings!(chunk, timezone).then(
            (list) => {
              const byId = new Map(list.sessions.map((entry) => [entry.session_id, entry.answer]));
              settle((id) => byId.get(id));
            },
            (error: unknown) => {
              for (const id of chunk) for (const one of named.get(id) ?? []) one.reject(error);
            },
          );
        }
      }, 0);
    }
    const list = waiting.get(id) ?? [];
    list.push({ resolve, reject });
    waiting.set(id, list);
  });
}

/** One session's answer, if read, and whether it is being read or failed. */
export type WarningsLookup = (id: string) => SessionWarningsAnswer | undefined;

/**
 * Each named session's warnings, read once per session and zone and kept
 * until committed data changes: the key is under `sessions`, so every ingest
 * event, a sub-session pass and a content purge read them again. A session
 * whose answer is not in yet, or whose read failed, has no answer, and a row
 * shows no marker for it.
 */
export function useSessionWarnings(
  ids: readonly string[],
  timezone: string | null | undefined,
): WarningsLookup {
  const { source } = useData();
  const enabled = Boolean(source.sessionWarnings) && Boolean(timezone);
  const request = JSON.stringify([...new Set(ids)].sort());
  const named = useMemo<string[]>(() => JSON.parse(request), [request]);
  // One map of the answers in, rebuilt only when one of them changes.
  const combine = useCallback(
    (results: { data?: SessionWarningsAnswer }[]) =>
      new Map(
        named.flatMap((id, index) => {
          const data = results[index]?.data;
          return data ? [[id, data] as const] : [];
        }),
      ),
    [named],
  );
  const found = useQueries({
    queries: named.map((id) => ({
      queryKey: queryKeys.sessionWarnings(timezone ?? '', id),
      queryFn: () => readWarnings(source, timezone!, id),
      enabled,
      staleTime: Infinity,
    })),
    combine,
  });
  return useCallback((id: string) => found.get(id), [found]);
}

/**
 * One session's answer for its own page, with whether it is being read or
 * failed: the same cache entry the lists' rows use for that session.
 */
export function useSessionWarning(
  id: string | undefined,
  timezone: string,
  { refetchOnMount }: { refetchOnMount?: 'always' } = {},
) {
  const { source } = useData();
  return useQuery({
    queryKey: queryKeys.sessionWarnings(timezone, id ?? ''),
    queryFn: () => readWarnings(source, timezone, id!),
    enabled: Boolean(source.sessionWarnings) && Boolean(id),
    staleTime: Infinity,
    refetchOnMount,
  });
}

/**
 * The message a warning points at, as one string: its saved record, and for
 * a tool result `#` and that result's block (the native `points_at`, the
 * message Diagnose flags).
 */
export const pointsAt = ({ points_at: { record_uuid, block } }: SessionWarningItem) =>
  block === null ? record_uuid : `${record_uuid}#${block}`;

/** The warnings of a read answer; none for anything else. */
export const warningsOf = (answer: SessionWarningsAnswer | undefined): SessionWarningItem[] =>
  answer?.state === 'read' ? answer.warnings : [];

/** A press of Dismiss (`undo: false`) or Undo (`undo: true`) on one warning. */
type Press = { id: string; undo: boolean };
const pressKey = ({ id, undo }: Press) => `${undo ? 'restore' : 'dismiss'} ${id}`;

/**
 * Dismissing one warning, or undoing it, for one session's page: the native
 * side stores it, and the answers that show the session's warnings are read
 * again, whether it was stored or refused, before the press is over, so the
 * page never shows a guess. Each press settles on its own, so presses on two
 * warnings in quick succession each get their own answer.
 *
 * Resolves to whether the answer read after it lists the warning where the
 * press moved it (dismissed, or back); rejects when the change was refused.
 */
function useDismissal(sessionId: string) {
  const { source } = useData();
  const client = useQueryClient();
  const controls = source.warningDismissals;
  return useMemo(
    () =>
      controls
        ? async ({ id, undo }: Press) => {
            try {
              await (undo ? controls.restore(sessionId, id) : controls.dismiss(sessionId, id));
            } finally {
              await invalidateSessionWarnings(client, sessionId);
            }
            return listedAs(client, sessionId, id, !undo);
          }
        : null,
    [client, controls, sessionId],
  );
}

/** Dismiss/Undo state and read-back focus, shared by the page and selected issue modal. */
export function useWarningDismissals(
  sessionId: string,
  read: Extract<SessionWarningsAnswer, { state: 'read' }> | null,
  card: RefObject<HTMLElement | null>,
  selection?: { generation: number; onMoved: (warningId: string) => void },
) {
  const dismissal = useDismissal(sessionId);
  const owner = useMemo(() => ({ live: true, sessionId }), [sessionId]);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const navigation = useRef(selection?.generation ?? 0);
  navigation.current = selection?.generation ?? 0;
  useEffect(() => {
    owner.live = true;
    return () => {
      owner.live = false;
    };
  }, [owner]);
  const drawn = useRef(read);
  drawn.current = read;
  /**
   * The warning just moved between the lists, and which way: its button in
   * the other list takes focus once the new answer has drawn it there. `from`
   * is the answer drawn when the move was known; once a newer one is drawn
   * without that button, nothing takes focus.
   */
  const [moved, setMoved] = useState<(Press & { from: typeof read; generation: number }) | null>(
    null,
  );
  /** The presses still being stored and read back: their buttons wait. */
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  /** The presses refused, each said on its own warning until pressed again. */
  const [refused, setRefused] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    if (
      !read ||
      moved === null ||
      !card.current ||
      !owner.live ||
      currentOwner.current !== owner ||
      moved.generation !== navigation.current
    )
      return;
    const wanted = moved.undo ? 'dismiss' : 'restore';
    const button = [
      ...card.current.querySelectorAll<HTMLElement>(`[data-action="${wanted}"]`),
    ].find(
      (item) => item.closest<HTMLElement>('[data-warning-id]')?.dataset.warningId === moved.id,
    );
    if (button) {
      setMoved(null);
      button.focus();
    } else if (read !== moved.from) setMoved(null);
  }, [read, moved, card, owner]);
  const change = (set: typeof setBusy, key: string, on: boolean) =>
    set((before) => {
      if (before.has(key) === on) return before;
      const after = new Set(before);
      if (on) after.add(key);
      else after.delete(key);
      return after;
    });
  /** The same presses, for a second press before the page has redrawn. */
  const inFlight = useRef(new Set<string>());
  const press = (one: Press) => {
    const key = pressKey(one);
    if (!dismissal || inFlight.current.has(key)) return;
    const generation = navigation.current;
    const ownsSession = () => owner.live && currentOwner.current === owner;
    const ownsSelection = () => ownsSession() && navigation.current === generation;
    inFlight.current.add(key);
    change(setRefused, key, false);
    change(setBusy, key, true);
    dismissal(one)
      .then(
        (listed) => {
          // Moved only if the answer read after it says so; a failed read
          // leaves the warning where it was, and focus where it is.
          if (listed && ownsSelection()) {
            selection?.onMoved(one.id);
            setMoved({ ...one, from: drawn.current, generation });
          }
        },
        () => {
          if (ownsSession()) change(setRefused, key, true);
        },
      )
      .finally(() => {
        inFlight.current.delete(key);
        if (ownsSession()) change(setBusy, key, false);
      });
  };
  return (warning: SessionWarningItem, undo: boolean) => ({
    busy: busy.has(pressKey({ id: warning.id, undo })),
    refused: refused.has(pressKey({ id: warning.id, undo })),
    onPress: dismissal ? () => press({ id: warning.id, undo }) : undefined,
  });
}

/**
 * Where a mark opens: its session's page, carrying the list state that finds
 * the session (as the row's own link does) and the warning to show, or the
 * whole card when `warningId` is empty.
 */
export function warningHref(sessionId: string, warningId: string, carry?: URLSearchParams | null) {
  const base = sessionHref(sessionId, carry);
  const warning = new URLSearchParams({ [WARNING_PARAM]: warningId });
  return `${base}${base.includes('?') ? '&' : '?'}${warning}`;
}

/**
 * What a Dashboard session row's marker for `count` warnings before its
 * `hours`-hour activity axis is named: the native template, filled in.
 */
export const beforeAxisLabel = (count: number, hours: number) =>
  (count === 1 ? SESSION_WARNINGS_TEXT.before_axis_one : SESSION_WARNINGS_TEXT.before_axis_many)
    .replace('{count}', String(count))
    .replace('{hours}', String(hours));

/** The zone this Mac shows, as the session page's own times are written. */
export const localZone = () => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
};

/** The warning sign: a small triangle in the danger ink. */
export function WarningSign({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M10.3 3.9 1.8 18.4A2 2 0 0 0 3.5 21.4h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
      <path d="M12 9.5v4.5M12 17.5v.01" />
    </svg>
  );
}

/**
 * One warning's line in a tooltip or list: its day, its label, then its
 * reason, whose clock times carry no date of their own.
 */
export function WarningLine({ warning, id }: { warning: SessionWarningItem; id?: string }) {
  return (
    <span className="xt-warning-line" data-kind={warning.kind} id={id}>
      <span className="xt-warning-day">{warning.day}</span> <b>{warning.label}</b> {warning.reason}
    </span>
  );
}

/**
 * One warning drawn on an activity track at `left` (0–1): a small round dot
 * in the danger ink, ringed with the surface colour so it reads over a span
 * bar. It is a focusable, named control, like the compaction ticks beside it are
 * focusable marks; hovering or focusing it shows the warning's label and
 * reason. `to` opens its session page; `onOpen` opens Dashboard detail.
 * While the warning is being diagnosed an arc circles the dot, and its hint
 * and name say so (`useDiagnosisStatus`).
 */
export function WarningDot({
  sessionId,
  warning,
  left,
  to,
  onOpen,
  suppressTooltip = false,
}: {
  sessionId: string;
  warning: SessionWarningItem;
  left: number;
  suppressTooltip?: boolean;
} & (
  { to: string; onOpen?: never } | { to?: never; onOpen: (button: HTMLButtonElement) => void }
)) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const theme = useSurfaceTheme();
  const status = useDiagnosisStatus(sessionId, warning.id);
  return (
    <Tooltip.Root disableHoverablePopup open={open && !suppressTooltip} onOpenChange={setOpen}>
      <Tooltip.Trigger
        render={
          onOpen ? (
            <button
              type="button"
              className="xt-warning-dot"
              data-kind={warning.kind}
              data-testid="warning-dot"
              data-warning-id={warning.id}
              data-diagnosis={status?.kind}
              style={{ left: `${left * 100}%` }}
              tabIndex={0}
              aria-haspopup="dialog"
              onClick={(event) => onOpen(event.currentTarget)}
            />
          ) : (
            <Link
              to={to}
              className="xt-warning-dot"
              data-kind={warning.kind}
              data-testid="warning-dot"
              data-warning-id={warning.id}
              data-diagnosis={status?.kind}
              style={{ left: `${left * 100}%` }}
              // WebKit leaves links out of the Tab order unless told.
              tabIndex={0}
            />
          )
        }
        // The dot has no text for a name to label.
        aria-label={withDiagnosisStatus(warning.name, status)}
        aria-describedby={id}
        delay={0}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="top"
          align="center"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup
            id={id}
            role="tooltip"
            className="xt-rule-popover xt-warning-tip"
            data-testid="warning-tip"
          >
            <WarningLine warning={warning} />
            <DiagnosisStatusLine status={status} />
            {!onOpen && (
              <span className="xt-warning-tip-hint">{SESSION_WARNINGS_TEXT.open_hint}</span>
            )}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/**
 * A row's compact warning marker: the sign and a count, one tab stop, whose
 * tooltip lists each warning's label and reason. `label` is its accessible
 * name; `heading` heads the list. It is a link to `to`, the session's page
 * at its Warnings card. Nothing is drawn without a warning.
 */
export function WarningMarker({
  warnings,
  label,
  heading,
  count,
  to,
  testId = 'warning-marker',
}: {
  warnings: readonly SessionWarningItem[];
  label: string;
  heading?: string;
  /** What the marker shows beside the sign; the number of warnings by default. */
  count?: string;
  to: string;
  testId?: string;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  if (warnings.length === 0) return null;
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        // WebKit leaves links out of the Tab order unless told.
        render={<Link to={to} className="xt-warning-marker" data-testid={testId} tabIndex={0} />}
        aria-label={label}
        aria-describedby={id}
        delay={0}
      >
        <WarningSign size={11} />
        <span aria-hidden="true">{count ?? warnings.length.toLocaleString('en-US')}</span>
      </Tooltip.Trigger>
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="bottom"
          align="start"
          sideOffset={6}
          collisionPadding={8}
        >
          <Tooltip.Popup
            id={id}
            role="tooltip"
            className="xt-rule-popover xt-warning-tip"
            data-testid={`${testId}-tip`}
          >
            {heading && <span className="xt-warning-tip-head">{heading}</span>}
            {warnings.map((warning) => (
              <WarningLine key={warning.id} warning={warning} />
            ))}
            <span className="xt-warning-tip-hint">{SESSION_WARNINGS_TEXT.open_hint}</span>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/**
 * A listed session's marker: its count and every warning's reason, from the
 * native answer. A session with no warning, or not read yet, shows nothing:
 * a row never marks healthy or unread data, and its page says what the
 * checks could not read.
 */
export function SessionWarningMarker({
  answer,
  to,
}: {
  answer: SessionWarningsAnswer | undefined;
  /** The session's page at its Warnings card. */
  to: string;
}) {
  if (answer?.state !== 'read' || !answer.count_label) return null;
  return (
    <WarningMarker
      warnings={answer.warnings}
      label={answer.count_label}
      heading={answer.count_label}
      to={to}
    />
  );
}

export function WarningChecks({
  answer,
}: {
  answer: Extract<SessionWarningsAnswer, { state: 'read' }>;
}) {
  return (
    <details className="xt-warnings-check-details">
      <summary tabIndex={0}>{answer.checks_summary}</summary>
      <ul className="xt-warnings-checks" aria-label="Checks" data-testid="warnings-checks">
        {answer.checks.map((check) => (
          <li key={check.kind} data-complete={check.complete || undefined}>
            <span className="xt-warnings-check-mark" aria-hidden="true">
              {check.complete ? '✓' : '–'}
            </span>
            <span>{check.text}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

/**
 * A session's own warnings, for its page: the native summary, each warning
 * not dismissed with a Dismiss button, the dismissed ones apart with an Undo,
 * and each check's line, so a session some checks could not fully read says
 * so instead of reading as one with nothing wrong.
 *
 * `highlight` is the warning a mark opened the page at (its `id`), or empty
 * for the card as a whole: once the answer is in, the card scrolls into view
 * and focus moves to that warning, marked, or to the card. `actions` is the
 * place for more buttons on each warning not dismissed, before Dismiss, and
 * `below` for content supplied under the warning.
 */
export function SessionWarningsDetail({
  sessionId,
  answer,
  pending,
  failed,
  highlight = null,
  actions,
  below,
}: {
  sessionId: string;
  answer: SessionWarningsAnswer | undefined;
  pending: boolean;
  failed: boolean;
  highlight?: string | null;
  actions?: (warning: SessionWarningItem) => ReactNode;
  below?: (warning: SessionWarningItem) => ReactNode;
}) {
  const card = useRef<HTMLDivElement>(null);
  const shown = useRef<string | null>(null);
  const read = answer?.state === 'read' ? answer : null;
  const rowProps = useWarningDismissals(sessionId, read, card);
  useEffect(() => {
    if (!read || highlight === null || shown.current === highlight) return;
    shown.current = highlight;
    const element = card.current;
    if (!element) return;
    const target =
      (highlight &&
        [...element.querySelectorAll<HTMLElement>('[data-warning-id]')].find(
          (item) => item.dataset.warningId === highlight,
        )) ||
      element.querySelector<HTMLElement>('.xt-warnings-summary') ||
      element;
    // Not every engine a test runs in scrolls.
    target.scrollIntoView?.({ block: 'center' });
    target.focus({ preventScroll: true });
  }, [read, highlight]);
  if (failed && !answer)
    return (
      <p className="xt-session-detail-notice" role="status" data-testid="warnings-state">
        {SESSION_WARNINGS_TEXT.session_unread}
      </p>
    );
  if (!answer)
    return (
      <p className="xt-session-detail-notice" role="status" data-testid="warnings-state">
        {pending
          ? SESSION_WARNINGS_TEXT.session_reading
          : SESSION_WARNINGS_TEXT.session_unavailable}
      </p>
    );
  if (answer.state === 'missing')
    return (
      <p className="xt-session-detail-notice" role="status" data-testid="warnings-state">
        {SESSION_WARNINGS_TEXT.session_missing}
      </p>
    );
  const item = (warning: SessionWarningItem, undo: boolean) => (
    <WarningRow
      key={warning.id}
      warning={warning}
      undo={undo}
      highlighted={highlight !== null && highlight !== '' && highlight === warning.id}
      {...rowProps(warning, undo)}
      actions={undo ? undefined : actions?.(warning)}
      below={undo ? undefined : below?.(warning)}
    />
  );
  return (
    <div ref={card} className="xt-warnings" data-testid="session-warnings">
      <p
        tabIndex={-1}
        className="xt-warnings-summary"
        data-warned={answer.warnings.length > 0 || undefined}
        data-incomplete={answer.incomplete_checks > 0 || undefined}
        data-testid="warnings-summary"
      >
        {answer.warnings.length > 0 && <WarningSign size={13} />}
        <span>{answer.summary}</span>
      </p>
      {answer.warnings.length > 0 && (
        <ol className="xt-warnings-list" aria-label="Warnings" data-testid="warnings-list">
          {answer.warnings.map((warning) => item(warning, false))}
        </ol>
      )}
      {answer.dismissed.length > 0 && (
        <section className="xt-warnings-dismissed" aria-labelledby={`${sessionId}-dismissed`}>
          <h3 id={`${sessionId}-dismissed`} className="xt-warnings-dismissed-head">
            {SESSION_WARNINGS_TEXT.dismissed_heading}
          </h3>
          <ol
            className="xt-warnings-list"
            data-dismissed=""
            aria-label={SESSION_WARNINGS_TEXT.dismissed_heading}
            data-testid="warnings-dismissed"
          >
            {answer.dismissed.map((warning) => item(warning, true))}
          </ol>
        </section>
      )}
      <WarningChecks answer={answer} />
    </div>
  );
}

/**
 * Under a warning: that it was shown again after growing past its
 * dismissal, and that the last Dismiss or Undo press was refused.
 */
export function WarningNotes({
  warning,
  undo,
  refused,
}: {
  warning: SessionWarningItem;
  undo: boolean;
  refused: boolean;
}) {
  return (
    <>
      {warning.returned && (
        <span className="xt-warning-returned" data-testid="warning-returned">
          {warning.returned}
        </span>
      )}
      {refused && (
        <span className="xt-warning-refused" role="alert" data-testid="warning-refused">
          {undo ? SESSION_WARNINGS_TEXT.restore_failed : SESSION_WARNINGS_TEXT.dismiss_failed}
        </span>
      )}
    </>
  );
}

/**
 * A warning's Dismiss button, or Undo for a dismissed one: it waits while
 * its press is stored and read back.
 */
export function DismissButton({
  undo,
  busy,
  onPress,
  describedBy,
}: {
  undo: boolean;
  busy: boolean;
  onPress: () => void;
  describedBy?: string;
}) {
  return (
    <button
      type="button"
      className="xt-warning-action"
      data-action={undo ? 'restore' : 'dismiss'}
      aria-describedby={describedBy}
      // WebKit leaves buttons out of the Tab order unless told.
      tabIndex={0}
      aria-disabled={busy || undefined}
      onClick={busy ? undefined : onPress}
    >
      {undo ? SESSION_WARNINGS_TEXT.restore : SESSION_WARNINGS_TEXT.dismiss}
    </button>
  );
}

/**
 * One warning on its session's page: its line and, where dismissals are
 * stored, its button (Dismiss, or Undo for a dismissed one), described by
 * the line. `actions` go before the button.
 */
export function WarningRow({
  warning,
  undo,
  highlighted,
  busy,
  refused,
  onPress,
  actions,
  below,
  summary,
}: {
  warning: SessionWarningItem;
  undo: boolean;
  highlighted: boolean;
  busy: boolean;
  refused: boolean;
  onPress?: () => void;
  actions?: ReactNode;
  below?: ReactNode;
  /** What stands for the warning's line (the session page's selected warning). */
  summary?: ReactNode;
}) {
  const line = useId();
  return (
    <li
      data-kind={warning.kind}
      data-warning-id={warning.id}
      // The saved message the native answer points at.
      data-points-at={pointsAt(warning)}
      data-highlighted={highlighted || undefined}
      data-testid={undo ? 'dismissed-warning' : 'warning-item'}
      tabIndex={-1}
    >
      {summary ? <div id={line}>{summary}</div> : <WarningLine warning={warning} id={line} />}
      {(onPress || actions) && (
        <span className="xt-warning-actions">
          {actions}
          {onPress && (
            <DismissButton undo={undo} busy={busy} onPress={onPress} describedBy={line} />
          )}
        </span>
      )}
      <WarningNotes warning={warning} undo={undo} refused={refused} />
      {below}
    </li>
  );
}
