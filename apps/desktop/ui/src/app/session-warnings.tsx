import {
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryFilters,
} from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { useData } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../data/generated/SessionWarningsAnswer';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { queryKeys } from '../data/query-client';

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
