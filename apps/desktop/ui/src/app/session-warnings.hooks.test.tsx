import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryObserver, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRef } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { DataSource } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../data/generated/SessionWarningsAnswer';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { events } from '../data/ipc-names';
import { queryKeys } from '../data/query-client';
import {
  invalidateSessionWarnings,
  pointsAt,
  useSessionWarning,
  useSessionWarnings,
  useWarningDismissals,
  warningsOf,
  type WarningsLookup,
} from './session-warnings';

const exported = fixture as unknown as FixtureExport;
afterEach(cleanup);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

// These are supplied DTOs for transport/lifecycle tests, not a warning detector.
function warning(id: string): SessionWarningItem {
  return {
    id,
    kind: 'correction',
    label: 'Correction',
    at_ms: 0,
    day: 'Jan 1',
    reason: id,
    compact_reason: id,
    name: id,
    record_uuids: [id],
    points_at: { record_uuid: id, block: null },
    returned: null,
  };
}
const first = warning('correction:first');
const second = warning('correction:second');
function answer(
  summary: string,
  warnings: SessionWarningItem[] = [],
  dismissed: SessionWarningItem[] = [],
): Extract<SessionWarningsAnswer, { state: 'read' }> {
  return {
    state: 'read',
    warnings,
    dismissed,
    count_label: null,
    summary,
    checks_summary: 'Synthetic checks',
    checks: [],
    incomplete_checks: 0,
  };
}
function source(read?: DataSource['sessionWarnings']) {
  const result = new FixtureDataSource(exported);
  const spy = vi.spyOn(result, 'sessionWarnings').mockImplementation(
    read ??
      (async (ids, timezone) => ({
        timezone,
        sessions: ids.map((session_id) => ({
          session_id,
          answer: answer(`${session_id} ${timezone}`),
        })),
      })),
  );
  return { source: result, read: spy };
}

type Bulk = { lookup: WarningsLookup; client: QueryClient };
function BulkProbe({
  ids,
  zone,
  capture,
}: {
  ids: string[];
  zone: string | null;
  capture: (value: Bulk) => void;
}) {
  const lookup = useSessionWarnings(ids, zone);
  const client = useQueryClient();
  capture({ lookup, client });
  return null;
}
function OneProbe({
  id = 's',
  zone = 'UTC',
  capture,
}: {
  id?: string;
  zone?: string;
  capture: (value: ReturnType<typeof useSessionWarning>) => void;
}) {
  const read = useSessionWarning(id, zone);
  capture(read);
  return null;
}

it('batches distinct IDs at the native limit and shares cached answers between list and detail readers', async () => {
  const data = source();
  const ids = Array.from({ length: SESSION_WARNINGS_TEXT.max_sessions + 1 }, (_, i) => `s${i}`);
  let bulk!: Bulk;
  let detail!: ReturnType<typeof useSessionWarning>;
  render(
    <DataProvider source={data.source}>
      <BulkProbe
        ids={[...ids, ids[0], ids[0]]}
        zone="UTC"
        capture={(v) => {
          bulk = v;
        }}
      />
      <OneProbe
        id={ids[0]}
        capture={(v) => {
          detail = v;
        }}
      />
    </DataProvider>,
  );
  await waitFor(() => expect(ids.every((id) => bulk?.lookup(id)?.state === 'read')).toBe(true));
  expect(data.read).toHaveBeenCalledTimes(2);
  expect(data.read.mock.calls.map(([named]) => named.length)).toEqual([
    SESSION_WARNINGS_TEXT.max_sessions,
    1,
  ]);
  expect(new Set(data.read.mock.calls.flatMap(([named]) => named)).size).toBe(ids.length);
  expect(detail.data).toBe(bulk.lookup(ids[0]));
});

it('keeps zones and source objects separate while sharing reads within each source and zone', async () => {
  const a = source();
  const b = source();
  let utc!: Bulk;
  let tokyo!: Bulk;
  let other!: Bulk;
  render(
    <>
      <DataProvider source={a.source}>
        <BulkProbe
          ids={['s']}
          zone="UTC"
          capture={(v) => {
            utc = v;
          }}
        />
        <BulkProbe
          ids={['s']}
          zone="Asia/Tokyo"
          capture={(v) => {
            tokyo = v;
          }}
        />
      </DataProvider>
      <DataProvider source={b.source}>
        <BulkProbe
          ids={['s']}
          zone="UTC"
          capture={(v) => {
            other = v;
          }}
        />
      </DataProvider>
    </>,
  );
  await waitFor(() => expect([utc, tokyo, other].every((v) => v?.lookup('s'))).toBe(true));
  expect(a.read.mock.calls.map(([, zone]) => zone).sort()).toEqual(['Asia/Tokyo', 'UTC']);
  expect(b.read).toHaveBeenCalledExactlyOnceWith(['s'], 'UTC');
  expect(utc.lookup('s')).not.toBe(tokyo.lookup('s'));
  expect(utc.client).not.toBe(other.client);
});

it('does not expose the previous zone while a changed zone is still being read', async () => {
  const pending = deferred<Awaited<ReturnType<NonNullable<DataSource['sessionWarnings']>>>>();
  const data = source(async (ids, timezone) =>
    timezone === 'Asia/Tokyo'
      ? pending.promise
      : {
          timezone,
          sessions: ids.map((session_id) => ({ session_id, answer: answer('UTC answer') })),
        },
  );
  let current!: Bulk;
  const body = (zone: string) => (
    <DataProvider source={data.source}>
      <BulkProbe
        ids={['s']}
        zone={zone}
        capture={(v) => {
          current = v;
        }}
      />
    </DataProvider>
  );
  const view = render(body('UTC'));
  await waitFor(() => expect(current?.lookup('s')).toEqual(answer('UTC answer')));
  view.rerender(body('Asia/Tokyo'));
  expect(current.lookup('s')).toBeUndefined();
  await waitFor(() => expect(data.read).toHaveBeenCalledTimes(2));
  await act(async () =>
    pending.resolve({
      timezone: 'Asia/Tokyo',
      sessions: [{ session_id: 's', answer: answer('Tokyo answer') }],
    }),
  );
  await waitFor(() => expect(current.lookup('s')).toEqual(answer('Tokyo answer')));
  view.rerender(body('UTC'));
  expect(current.lookup('s')).toEqual(answer('UTC answer'));
  expect(data.read).toHaveBeenCalledTimes(2);
});

it('does not read without the optional capability or a list timezone', async () => {
  const data = source();
  Object.defineProperty(data.source, 'sessionWarnings', { value: undefined });
  let absent!: Bulk;
  render(
    <DataProvider source={data.source}>
      <BulkProbe
        ids={['s']}
        zone="UTC"
        capture={(v) => {
          absent = v;
        }}
      />
    </DataProvider>,
  );
  await waitFor(() => expect(absent).toBeDefined());
  expect(absent.lookup('s')).toBeUndefined();
  const noZone = source();
  let waiting!: Bulk;
  render(
    <DataProvider source={noZone.source}>
      <BulkProbe
        ids={['s']}
        zone={null}
        capture={(v) => {
          waiting = v;
        }}
      />
    </DataProvider>,
  );
  await waitFor(() => expect(waiting).toBeDefined());
  expect(waiting.lookup('s')).toBeUndefined();
  expect(data.read).not.toHaveBeenCalled();
  expect(noZone.read).not.toHaveBeenCalled();
});

it.each(['failed', 'omitted'] as const)(
  'keeps a %s warning read as an error instead of an empty healthy answer',
  async (mode) => {
    const error = new Error('synthetic read failure');
    const data = source(async (_ids, timezone) => {
      if (mode === 'failed') throw error;
      return { timezone, sessions: [] };
    });
    let result!: ReturnType<typeof useSessionWarning>;
    render(
      <DataProvider source={data.source}>
        <OneProbe
          capture={(v) => {
            result = v;
          }}
        />
      </DataProvider>,
    );
    await waitFor(() => expect(result?.isError).toBe(true));
    expect(result.data).toBeUndefined();
    expect(result.error?.message).toBe(
      mode === 'failed' ? error.message : 'The warnings read left this session out.',
    );
  },
);

it('replaces the query runtime with its source and discards a late answer from the previous source', async () => {
  const oldRead = deferred<Awaited<ReturnType<NonNullable<DataSource['sessionWarnings']>>>>();
  const old = source(() => oldRead.promise);
  const next = source(async (ids, timezone) => ({
    timezone,
    sessions: ids.map((session_id) => ({ session_id, answer: answer('new source') })),
  }));
  let current!: Bulk;
  const body = (data: DataSource) => (
    <DataProvider source={data}>
      <BulkProbe
        ids={['s']}
        zone="UTC"
        capture={(v) => {
          current = v;
        }}
      />
    </DataProvider>
  );
  const view = render(body(old.source));
  await waitFor(() => expect(old.read).toHaveBeenCalledTimes(1));
  const oldClient = current.client;
  const cancel = vi.spyOn(oldClient, 'cancelQueries');
  view.rerender(body(next.source));
  await waitFor(() => expect(current.lookup('s')).toEqual(answer('new source')));
  expect(current.client).not.toBe(oldClient);
  expect(cancel).toHaveBeenCalled();
  await act(async () =>
    oldRead.resolve({
      timezone: 'UTC',
      sessions: [{ session_id: 's', answer: answer('old source') }],
    }),
  );
  expect(current.lookup('s')).toEqual(answer('new source'));
  expect(oldClient.getQueryData(queryKeys.sessionWarnings('UTC', 's'))).toBeUndefined();
});

it('cancels the runtime cache on unmount without pretending the native promise stopped', async () => {
  const pending = deferred<Awaited<ReturnType<NonNullable<DataSource['sessionWarnings']>>>>();
  const data = source(() => pending.promise);
  let current!: Bulk;
  const view = render(
    <DataProvider source={data.source}>
      <BulkProbe
        ids={['s']}
        zone="UTC"
        capture={(v) => {
          current = v;
        }}
      />
    </DataProvider>,
  );
  await waitFor(() => expect(data.read).toHaveBeenCalledTimes(1));
  const cancel = vi.spyOn(current.client, 'cancelQueries');
  view.unmount();
  expect(cancel).toHaveBeenCalled();
  await act(async () =>
    pending.resolve({ timezone: 'UTC', sessions: [{ session_id: 's', answer: answer('late') }] }),
  );
  expect(current.client.getQueryData(queryKeys.sessionWarnings('UTC', 's'))).toBeUndefined();
});

it('leaves a shared read available to the cache when only its last screen unmounts', async () => {
  const pending = deferred<Awaited<ReturnType<NonNullable<DataSource['sessionWarnings']>>>>();
  const data = source(() => pending.promise);
  let current!: Bulk;
  const body = (shown: boolean) => (
    <DataProvider source={data.source}>
      {shown && (
        <BulkProbe
          ids={['s']}
          zone="UTC"
          capture={(v) => {
            current = v;
          }}
        />
      )}
    </DataProvider>
  );
  const view = render(body(true));
  await waitFor(() => expect(data.read).toHaveBeenCalledTimes(1));
  view.rerender(body(false));
  await act(async () =>
    pending.resolve({ timezone: 'UTC', sessions: [{ session_id: 's', answer: answer('kept') }] }),
  );
  await waitFor(() =>
    expect(current.client.getQueryData(queryKeys.sessionWarnings('UTC', 's'))).toEqual(
      answer('kept'),
    ),
  );
});

it('re-reads session and window warning answers after committed import and relationship changes', async () => {
  const data = source();
  const windowRead = vi.spyOn(data.source, 'windowWarnings');
  const entry = exported.dashboard_warnings[0];
  let current!: Bulk;
  function WindowProbe() {
    useQuery({
      queryKey: queryKeys.windowWarnings(
        entry.window_days,
        entry.warnings.start_ms,
        entry.warnings.end_ms,
        entry.timezone,
      ),
      queryFn: () =>
        data.source.windowWarnings(entry.window_days, entry.window_end_ms, entry.timezone),
      staleTime: Infinity,
    });
    return null;
  }
  render(
    <DataProvider source={data.source}>
      <BulkProbe
        ids={['s']}
        zone="UTC"
        capture={(v) => {
          current = v;
        }}
      />
      <WindowProbe />
    </DataProvider>,
  );
  await waitFor(() => expect(current?.lookup('s')).toBeDefined());
  await waitFor(() => expect(windowRead).toHaveBeenCalledTimes(1));
  const initial = data.read.mock.calls.length;
  act(() => data.source.emit(events.importReceived));
  await waitFor(() => expect(data.read).toHaveBeenCalledTimes(initial + 1));
  await waitFor(() => expect(windowRead).toHaveBeenCalledTimes(2));
  act(() => data.source.emit(events.sessionCreationsChanged));
  await waitFor(() => expect(data.read).toHaveBeenCalledTimes(initial + 2));
  await waitFor(() => expect(windowRead).toHaveBeenCalledTimes(3));
});

it('invalidates only this session in all zones and every warning window, including inactive queries', async () => {
  const reads: unknown[] = [];
  const client = new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: Infinity,
        queryFn: ({ queryKey }) => {
          reads.push(queryKey);
          return {};
        },
      },
    },
  });
  const keys = [
    queryKeys.sessionWarnings('UTC', 's'),
    queryKeys.sessionWarnings('Asia/Tokyo', 's'),
    queryKeys.sessionWarnings('UTC', 'other'),
    queryKeys.windowWarnings(7, 0, 1, 'UTC'),
    queryKeys.windowWarnings(30, 0, 1, 'UTC'),
    queryKeys.dashboard(7),
  ];
  for (const key of keys) client.setQueryData(key, {});
  await invalidateSessionWarnings(client, 's');
  expect(reads).toEqual([keys[0], keys[1], keys[3], keys[4]]);
});

it('discards pre-dismissal reads before reading back session and window answers', async () => {
  const client = new QueryClient();
  let stored = false;
  const keys = [queryKeys.windowWarnings(7, 0, 1, 'UTC'), queryKeys.sessionWarnings('UTC', 's')];
  const waiting = keys.map(() => [] as (() => void)[]);
  const stale: (() => void)[] = [];
  for (const [i, key] of keys.entries()) {
    const observer = new QueryObserver(client, {
      queryKey: key,
      staleTime: Infinity,
      queryFn: () =>
        new Promise<string>((resolve) => {
          const value = stored ? 'after' : 'before';
          waiting[i].push(() => resolve(value));
        }),
    });
    const stop = observer.subscribe(() => {});
    await waitFor(() => expect(waiting[i]).toHaveLength(1));
    waiting[i].shift()!();
    await waitFor(() => expect(client.getQueryData(key)).toBe('before'));
    void client.invalidateQueries({ queryKey: key });
    await waitFor(() => expect(waiting[i]).toHaveLength(1));
    stale.push(waiting[i].shift()!);
    stop();
  }
  stored = true;
  const done = invalidateSessionWarnings(client, 's');
  for (const land of stale) land();
  await waitFor(() => expect(waiting.map((queue) => queue.length)).toEqual([1, 1]));
  for (const queue of waiting) queue.shift()!();
  await done;
  for (const key of keys) expect(client.getQueryData(key)).toBe('after');
});

function Actions({
  id = 's',
  generation = 0,
  onMoved = () => {},
  capture,
}: {
  id?: string;
  generation?: number;
  onMoved?: (id: string) => void;
  capture?: (client: QueryClient) => void;
}) {
  const query = useSessionWarning(id, 'UTC');
  const client = useQueryClient();
  capture?.(client);
  const read = query.data?.state === 'read' ? query.data : null;
  const card = useRef<HTMLDivElement>(null);
  const props = useWarningDismissals(id, read, card, { generation, onMoved });
  return (
    <div ref={card}>
      {read &&
        [
          ...read.warnings.map((item) => ({ item, undo: false })),
          ...read.dismissed.map((item) => ({ item, undo: true })),
        ].map(({ item, undo }) => {
          const state = props(item, undo);
          return (
            <div key={item.id} data-warning-id={item.id}>
              <button
                data-action={undo ? 'restore' : 'dismiss'}
                aria-busy={state.busy}
                onClick={state.onPress}
              >
                {undo ? 'restore' : 'dismiss'} {item.id}
              </button>
              {state.refused && <output>refused {item.id}</output>}
            </div>
          );
        })}
    </div>
  );
}
function controlled() {
  const gone = new Set<string>();
  let failRead = false;
  const data = source(async (ids, timezone) => {
    if (failRead) throw new Error('readback failed');
    return {
      timezone,
      sessions: ids.map((session_id) => ({
        session_id,
        answer: answer(
          'supplied answer',
          [first, second].filter((w) => !gone.has(w.id)),
          [first, second].filter((w) => gone.has(w.id)),
        ),
      })),
    };
  });
  const pending = new Map<string, ReturnType<typeof deferred<unknown>>>();
  const controls = {
    dismiss: vi.fn((_session: string, id: string) => {
      const press = deferred<unknown>();
      pending.set(id, press);
      return press.promise;
    }),
    restore: vi.fn(async (session_id: string, warning_id: string) => {
      gone.delete(warning_id);
      return { session_id, warning_id, dismissed: false };
    }),
  };
  Object.defineProperty(data.source, 'warningDismissals', { value: controls });
  return {
    ...data,
    controls,
    refuseRead: () => {
      failRead = true;
    },
    finish: (id: string, refused = false) => {
      const press = pending.get(id)!;
      if (refused) press.reject(new Error('write refused'));
      else {
        gone.add(id);
        press.resolve({ session_id: 's', warning_id: id, dismissed: true });
      }
    },
  };
}

it('keeps quick presses independent, prevents a duplicate press, reads back refusals and supports Undo', async () => {
  const data = controlled();
  const moved = vi.fn();
  render(
    <DataProvider source={data.source}>
      <Actions onMoved={moved} />
    </DataProvider>,
  );
  const one = await screen.findByRole('button', { name: `dismiss ${first.id}` });
  const two = screen.getByRole('button', { name: `dismiss ${second.id}` });
  fireEvent.click(one);
  fireEvent.click(one);
  fireEvent.click(two);
  expect(data.controls.dismiss).toHaveBeenCalledTimes(2);
  expect(one.getAttribute('aria-busy')).toBe('true');
  expect(two.getAttribute('aria-busy')).toBe('true');
  await act(async () => data.finish(second.id));
  const restore = await screen.findByRole('button', { name: `restore ${second.id}` });
  await waitFor(() => expect(document.activeElement).toBe(restore));
  expect(one.getAttribute('aria-busy')).toBe('true');
  const before = data.read.mock.calls.length;
  await act(async () => data.finish(first.id, true));
  await screen.findByText(`refused ${first.id}`);
  expect(data.read.mock.calls.length).toBeGreaterThan(before);
  expect(one.getAttribute('aria-busy')).toBe('false');
  fireEvent.click(restore);
  await waitFor(() =>
    expect(data.controls.restore).toHaveBeenCalledExactlyOnceWith('s', second.id),
  );
  const dismiss = await screen.findByRole('button', { name: `dismiss ${second.id}` });
  await waitFor(() => expect(document.activeElement).toBe(dismiss));
});

it('reads back a refused Undo and clears only that action state', async () => {
  const data = controlled();
  const moved = vi.fn();
  render(
    <DataProvider source={data.source}>
      <Actions onMoved={moved} />
    </DataProvider>,
  );
  fireEvent.click(await screen.findByRole('button', { name: `dismiss ${first.id}` }));
  await act(async () => data.finish(first.id));
  const restore = await screen.findByRole('button', { name: `restore ${first.id}` });
  await waitFor(() => expect(moved).toHaveBeenCalledTimes(1));
  data.controls.restore.mockRejectedValueOnce(new Error('Undo refused'));
  const reads = data.read.mock.calls.length;
  fireEvent.click(restore);
  await screen.findByText(`refused ${first.id}`);
  expect(restore.getAttribute('aria-busy')).toBe('false');
  expect(data.read.mock.calls.length).toBeGreaterThan(reads);
  expect(moved).toHaveBeenCalledTimes(1);
});

it('does not move focus when the readback fails and clears its action busy state', async () => {
  const data = controlled();
  const moved = vi.fn();
  render(
    <DataProvider source={data.source}>
      <Actions onMoved={moved} />
    </DataProvider>,
  );
  const button = await screen.findByRole('button', { name: `dismiss ${first.id}` });
  act(() => button.focus());
  fireEvent.click(button);
  data.refuseRead();
  await act(async () => data.finish(first.id));
  await waitFor(() => expect(button.getAttribute('aria-busy')).toBe('false'));
  expect(moved).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(button);
});

it.each(['selection', 'session', 'unmount'] as const)(
  'suppresses late action focus after %s changes',
  async (change) => {
    const data = controlled();
    const moved = vi.fn();
    const body = (id = 's', generation = 0) => (
      <DataProvider source={data.source}>
        <Actions id={id} generation={generation} onMoved={moved} />
      </DataProvider>
    );
    const view = render(body());
    const button = await screen.findByRole('button', { name: `dismiss ${first.id}` });
    fireEvent.click(button);
    if (change === 'unmount') view.unmount();
    else view.rerender(body(change === 'session' ? 'other' : 's', 1));
    await act(async () => data.finish(first.id));
    await waitFor(() => expect(data.read.mock.calls.length).toBeGreaterThanOrEqual(2));
    expect(moved).not.toHaveBeenCalled();
  },
);

it('offers no mutation handler without the optional dismissal capability and uses supplied point fields', async () => {
  const data = source(async (ids, timezone) => ({
    timezone,
    sessions: ids.map((session_id) => ({ session_id, answer: answer('unchanged', [first]) })),
  }));
  render(
    <DataProvider source={data.source}>
      <Actions />
    </DataProvider>,
  );
  const button = await screen.findByRole('button', { name: `dismiss ${first.id}` });
  fireEvent.click(button);
  expect(button.getAttribute('aria-busy')).toBe('false');
  expect(data.read).toHaveBeenCalledTimes(1);
  expect(pointsAt(first)).toBe(first.id);
  expect(pointsAt({ ...first, points_at: { record_uuid: 'record', block: 0 } })).toBe('record#0');
  expect(warningsOf(undefined)).toEqual([]);
  expect(warningsOf(answer('one', [first]))).toEqual([first]);
});
