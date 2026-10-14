import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryObserver, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import type { SessionPage } from '../data/generated/SessionPage';
import type { TokensByHost } from '../data/generated/TokensByHost';
import { events, type DataEvent } from '../data/ipc-names';
import { queryKeys } from '../data/query-client';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

const exported = fixture as FixtureExport;
const ready: NativeIndexStatus = {
  ...exported.native_index,
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
};
const scanning: NativeIndexStatus = { ...ready, phase: { phase: 'scanning' } };
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
};
const report = (days: number, fresh = true): DashboardMetrics => {
  const value = structuredClone(exported.dashboards.find((entry) => entry.window.days === days)!);
  value.tiles.sessions.value = fresh ? 22 : 11;
  value.tokens_by_host[0].tokens.counters.total_tokens = fresh ? 222 : 111;
  return value;
};
const tokens = (days: number, fresh = true): TokensByHost => {
  const value = report(days, fresh);
  return { window: value.window, hosts: value.tokens_by_host };
};
type Pending = 'dashboard' | 'tokens' | 'both';
/** Independent promises capture old snapshots at request start; later reads see new data. */
function harness(days = 7, pending: Pending = 'both', status = ready, holdIndex = false) {
  const oldReport = deferred<DashboardMetrics>();
  const oldTokens = deferred<TokensByHost>();
  const freshReport = deferred<DashboardMetrics>();
  const freshTokens = deferred<TokensByHost>();
  const firstIndex = deferred<NativeIndexStatus>();
  const listeners = new Map<DataEvent, Set<() => void>>();
  const calls: { kind: string; days?: number; read: number; snapshot?: 'old' | 'fresh' }[] = [];
  let currentStatus = status;
  let holdFresh = false;
  let reportReads = 0;
  let tokenReads = 0;
  const source = {
    kind: 'native' as DataSource['kind'],
    appInfo: vi.fn(async () => exported.app_info),
    dbCounts: vi.fn(async () => exported.db_counts),
    sessionsList: vi.fn(async (): Promise<SessionPage> => ({ rows: [], next: null })),
    dashboard: vi.fn(async (range: number) => {
      const read = range === days ? ++reportReads : 0;
      const old = read === 1;
      const captured = report(range, !old);
      calls.push({ kind: 'dashboard', days: range, read, snapshot: old ? 'old' : 'fresh' });
      if (old && pending !== 'tokens') return oldReport.promise;
      if (range === days && read === 2 && holdFresh) return freshReport.promise;
      return captured;
    }),
    tokensByHost: vi.fn(async (range: number) => {
      const read = range === days ? ++tokenReads : 0;
      const old = read === 1;
      const captured = tokens(range, !old);
      calls.push({ kind: 'tokens', days: range, read, snapshot: old ? 'old' : 'fresh' });
      if (old && pending !== 'dashboard') return oldTokens.promise;
      if (range === days && read === 2 && holdFresh) return freshTokens.promise;
      return captured;
    }),
    nativeIndexStatus: vi.fn(async () => {
      calls.push({ kind: 'index', read: calls.filter((call) => call.kind === 'index').length + 1 });
      return holdIndex ? firstIndex.promise : structuredClone(currentStatus);
    }),
    subscribe: vi.fn(async (event: DataEvent, callback: () => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(callback);
      listeners.set(event, set);
      return () => void set.delete(callback);
    }),
  } satisfies DataSource;
  return {
    source,
    calls,
    listeners,
    oldReport,
    oldTokens,
    freshReport,
    freshTokens,
    firstIndex,
    setStatus: (value: NativeIndexStatus) => {
      currentStatus = value;
    },
    holdFresh: () => {
      holdFresh = true;
    },
    emit: (event: DataEvent) => listeners.get(event)?.forEach((callback) => callback()),
    releaseOld: () => {
      oldReport.resolve(report(days, false));
      oldTokens.resolve(tokens(days, false));
    },
    releaseFresh: () => {
      freshReport.resolve(report(days));
      freshTokens.resolve(tokens(days));
    },
  };
}
function Capture({ onClient }: { onClient: (client: QueryClient) => void }) {
  onClient(useQueryClient());
  return null;
}
function TestApp({
  data,
  path,
  onClient,
}: {
  data: DataSource;
  path: string;
  onClient: (client: QueryClient) => void;
}) {
  return (
    <ThemeProvider>
      <DataProvider source={data}>
        <Capture onClient={onClient} />
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  );
}
function mount(data: DataSource, path = '/dashboard') {
  let client!: QueryClient;
  const onClient = (value: QueryClient) => {
    client = value;
  };
  const view = render(<TestApp data={data} path={path} onClient={onClient} />);
  return {
    ...view,
    client: () => client,
    replace: (next: DataSource) =>
      view.rerender(<TestApp data={next} path={path} onClient={onClient} />),
  };
}
const settle = (ms = 30) =>
  act(async () => void (await new Promise((done) => setTimeout(done, ms))));
const reads = (h: ReturnType<typeof harness>, days: number) => ({
  dashboard: h.source.dashboard.mock.calls.filter(([range]) => range === days).length,
  tokens: h.source.tokensByHost.mock.calls.filter(([range]) => range === days).length,
});
const sidebar = () => within(screen.getByRole('region', { name: 'Tokens by host' }));
const retryTokens = () =>
  screen.getByRole<HTMLButtonElement>('button', { name: 'Retry recorded tokens' });
async function fresh(view: ReturnType<typeof mount>, days: number) {
  await waitFor(() =>
    expect(screen.getByTestId('dashboard-summary').textContent).toContain('22 sessions'),
  );
  await waitFor(() => expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy());
  await settle();
  for (const key of [queryKeys.dashboard(days), queryKeys.tokensByHost(days)]) {
    const state = view.client().getQueryState(key)!;
    expect(state.status).toBe('success');
    expect(state.fetchStatus).toBe('idle');
    expect(state.isInvalidated).toBe(false);
  }
}
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

it('recovers independently held first tokens after Dashboard-first ready recovery', async () => {
  const h = harness();
  const view = mount(h.source);
  await settle();
  expect(reads(h, 7)).toEqual({ dashboard: 1, tokens: 1 });
  await act(async () => h.oldReport.resolve(report(7, false)));
  await waitFor(() => expect(reads(h, 7).dashboard).toBe(2));
  await act(async () => h.oldTokens.resolve(tokens(7, false)));
  await fresh(view, 7);
  expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
});
it('does not claim an empty token result while the first read is pending', async () => {
  const h = harness(7, 'tokens', ready, true);
  mount(h.source);
  await screen.findByTestId('dashboard-summary');
  expect.soft(sidebar().queryByText('Reading recorded tokens…')).toBeTruthy();
  expect.soft(sidebar().queryByText('No host measurements')).toBeNull();
});
it('offers a safe token Retry without claiming an empty result after an initial error', async () => {
  const h = harness(7, 'tokens', ready, true);
  mount(h.source);
  await screen.findByTestId('dashboard-summary');
  await act(async () => h.oldTokens.reject(new Error('private backend detail')));
  await screen.findByText('Recorded tokens unavailable');
  expect.soft(screen.queryByText('private backend detail')).toBeNull();
  expect.soft(sidebar().queryByText('No host measurements')).toBeNull();
  expect.soft(screen.queryByRole('button', { name: 'Retry recorded tokens' })).toBeTruthy();
});

// The first regression and existing Dashboard together controls complement these three orders.
for (const phase of ['ready', 'disabled'] as const)
  for (const order of ['dashboard-first', 'tokens-first'] as const) {
    if (phase === 'ready' && order === 'dashboard-first') continue;
    it(`recovers initial ${phase} metrics in ${order} order`, async () => {
      const h = harness(7, 'both', phase === 'ready' ? ready : exported.native_index);
      const view = mount(h.source);
      await settle();
      expect(reads(h, 7)).toEqual({ dashboard: 1, tokens: 1 });
      expect(h.source.nativeIndexStatus).not.toHaveBeenCalled();
      if (order === 'dashboard-first') {
        await act(async () => h.oldReport.resolve(report(7, false)));
        await waitFor(() => expect(reads(h, 7).dashboard).toBe(2));
        await act(async () => h.oldTokens.resolve(tokens(7, false)));
      } else {
        await act(async () => h.oldTokens.resolve(tokens(7, false)));
        await settle();
        expect(screen.queryByTestId('dashboard-summary')).toBeNull();
        expect(h.source.nativeIndexStatus).not.toHaveBeenCalled();
        await act(async () => h.oldReport.resolve(report(7, false)));
      }
      await fresh(view, 7);
      expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
      expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
      await settle();
      expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
    });
  }
for (const phase of ['ready', 'disabled'] as const)
  it(`recovers initial ${phase} metrics when status arrives after both old results`, async () => {
    const status = phase === 'ready' ? ready : exported.native_index;
    const h = harness(7, 'both', status, true);
    const view = mount(h.source);
    await act(async () => h.releaseOld());
    await screen.findByTestId('dashboard-summary');
    await settle();
    expect(sidebar().queryByLabelText('Claude Code tokens: 111')).toBeTruthy();
    expect(reads(h, 7)).toEqual({ dashboard: 1, tokens: 1 });
    await act(async () => h.firstIndex.resolve(status));
    await fresh(view, 7);
    expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
    expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  });
for (const phase of ['ready', 'disabled'] as const)
  for (const pending of ['dashboard', 'tokens', 'both'] as const)
    it(`refreshes uncached 14d ${pending} pending across a poll settling ${phase}`, async () => {
      const h = harness(14, pending, scanning);
      const view = mount(h.source);
      await screen.findByTestId('dashboard-summary');
      await waitFor(() => expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(1));
      fireEvent.click(screen.getByRole('radio', { name: '14d' }));
      await settle();
      expect(reads(h, 14)).toEqual({ dashboard: 1, tokens: 1 });
      const before7 = reads(h, 7);
      h.setStatus(phase === 'ready' ? ready : exported.native_index);
      // Actual production polling interval; no event and no fake QueryClient behavior.
      await waitFor(() => expect(reads(h, 14)).toEqual({ dashboard: 2, tokens: 2 }), {
        timeout: 2000,
      });
      await act(async () => h.releaseOld());
      await fresh(view, 14);
      expect(reads(h, 7)).toEqual(before7);
      const indexReads = h.source.nativeIndexStatus.mock.calls.length;
      await settle(1100);
      expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(indexReads);
      expect(reads(h, 14)).toEqual({ dashboard: 2, tokens: 2 });
    });
for (const pending of ['dashboard', 'tokens', 'both'] as const)
  it(`refreshes uncached 14d ${pending} pending when the first status finally arrives`, async () => {
    const h = harness(14, pending, ready, true);
    const view = mount(h.source);
    await screen.findByTestId('dashboard-summary');
    await settle();
    expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('radio', { name: '14d' }));
    await settle();
    expect(reads(h, 14)).toEqual({ dashboard: 1, tokens: 1 });
    await act(async () => h.firstIndex.resolve(ready));
    await waitFor(() => expect(reads(h, 14)).toEqual({ dashboard: 2, tokens: 2 }));
    await act(async () => h.releaseOld());
    await fresh(view, 14);
    expect(h.source.nativeIndexStatus).toHaveBeenCalledTimes(1);
  });
for (const [event, pending] of [
  [events.nativeIndexStatus, 'tokens'],
  [events.importReceived, 'dashboard'],
  [events.turnCompleted, 'both'],
] as const)
  it(`refreshes uncached 30d ${pending} pending after one batched ${event} event`, async () => {
    const h = harness(30, pending);
    const view = mount(h.source);
    await fresh(view, 7);
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await settle();
    expect(reads(h, 30)).toEqual({ dashboard: 1, tokens: 1 });
    const before7 = reads(h, 7);
    await act(async () => h.emit(event));
    await settle(450);
    expect(reads(h, 30)).toEqual({ dashboard: 1, tokens: 1 });
    await waitFor(() => expect(reads(h, 30)).toEqual({ dashboard: 2, tokens: 2 }));
    await act(async () => h.releaseOld());
    await fresh(view, 30);
    expect(reads(h, 7)).toEqual(before7);
    // The synthetic reports preserve the same fixture session IDs: turn completion is enrichment.
    expect(report(30).lanes.map((lane) => lane.session_id)).toEqual(
      report(30, false).lanes.map((lane) => lane.session_id),
    );
  });

it('retries only selected-range tokens, coalesces pending clicks, and restores their caption', async () => {
  const h = harness(14, 'tokens', ready, true);
  const view = mount(h.source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await settle();
  await act(async () => h.oldTokens.reject(new Error('private token failure')));
  await screen.findByText('Recorded tokens unavailable');
  const dashboardReads = h.source.dashboard.mock.calls.length;
  const beforePeriod = screen.getByTestId('report-period').textContent;
  h.holdFresh();
  const retry = retryTokens();
  retry.focus();
  expect(document.activeElement).toBe(retry);
  fireEvent.click(retry);
  await settle();
  expect(reads(h, 14).tokens).toBe(2);
  expect(retryTokens().disabled).toBe(true);
  expect(retryTokens().getAttribute('aria-busy')).toBe('true');
  expect(sidebar().queryByText('Reading recorded tokens…')).toBeTruthy();
  expect(sidebar().queryByText('No host measurements')).toBeNull();
  fireEvent.click(retryTokens());
  fireEvent.click(retryTokens());
  expect(reads(h, 14).tokens).toBe(2);
  await act(async () => h.freshTokens.resolve(tokens(14)));
  await waitFor(() => expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy());
  expect(screen.getByText('Recorded tokens · 14d')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Retry recorded tokens' })).toBeNull();
  expect(h.source.dashboard).toHaveBeenCalledTimes(dashboardReads);
  expect(screen.getByTestId('report-period').textContent).toBe(beforePeriod);
  expect(view.client().getQueryState(queryKeys.tokensByHost(14))?.status).toBe('success');
});
it('keeps a safe Retry after a selected-range token retry fails again', async () => {
  const h = harness(14, 'tokens', ready, true);
  mount(h.source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await settle();
  await act(async () => h.oldTokens.reject(new Error('first private error')));
  await screen.findByText('Recorded tokens unavailable');
  h.holdFresh();
  fireEvent.click(retryTokens());
  await settle();
  await act(async () => h.freshTokens.reject(new Error('second private error')));
  await screen.findByText('Recorded tokens unavailable');
  expect(retryTokens().disabled).toBe(false);
  expect(sidebar().queryByText('No host measurements')).toBeNull();
  expect(screen.queryByText(/private error/)).toBeNull();
  expect(reads(h, 14)).toEqual({ dashboard: 1, tokens: 2 });
});
it('retains cached host rows on a failed background refresh and retries tokens alone', async () => {
  const h = harness(14, 'tokens', ready, true);
  const view = mount(h.source);
  await screen.findByTestId('dashboard-summary');
  // A previously successful query remains mounted in Shell on other routes as well.
  const failed = deferred<TokensByHost>();
  h.source.tokensByHost.mockImplementationOnce(() => failed.promise);
  await act(async () => {
    void view.client().invalidateQueries({ queryKey: queryKeys.tokensByHost(7) });
  });
  await act(async () => failed.reject(new Error('private background error')));
  await screen.findByText('Recorded tokens refresh failed · 7d');
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
  expect(sidebar().queryByText('No host measurements')).toBeNull();
  const dashboardReads = h.source.dashboard.mock.calls.length;
  const next = deferred<TokensByHost>();
  h.source.tokensByHost.mockImplementationOnce(() => next.promise);
  fireEvent.click(retryTokens());
  await settle();
  expect(retryTokens().disabled).toBe(true);
  await act(async () => next.resolve(tokens(7)));
  await waitFor(() => expect(screen.queryByText('Recorded tokens refresh failed · 7d')).toBeNull());
  expect(h.source.dashboard).toHaveBeenCalledTimes(dashboardReads);
  expect(view.client().getQueryState(queryKeys.tokensByHost(7))?.status).toBe('success');
});

it('isolates source replacement while both old metric promises are held', async () => {
  const h = harness();
  const view = mount(h.source);
  await settle();
  const oldClient = view.client();
  const oldReads = reads(h, 7);
  const next = harness(14);
  view.replace(next.source);
  await fresh(view, 7);
  await act(async () => h.releaseOld());
  await settle();
  expect(view.client()).not.toBe(oldClient);
  expect(reads(h, 7)).toEqual(oldReads);
  expect(oldClient.getQueryData(queryKeys.dashboard(7))).toBeUndefined();
  expect(oldClient.getQueryData(queryKeys.tokensByHost(7))).toBeUndefined();
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
});
it('disposes after refresh begins while original and fresh metric promises remain held', async () => {
  const h = harness();
  h.holdFresh();
  const view = mount(h.source);
  await settle();
  await act(async () => h.oldReport.resolve(report(7, false)));
  await waitFor(() => expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 }));
  const client = view.client();
  view.unmount();
  const states = client
    .getQueryCache()
    .getAll()
    .map((query) => query.state);
  await act(async () => {
    h.releaseOld();
    h.releaseFresh();
  });
  await settle();
  expect(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.state),
  ).toEqual(states);
  expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
  expect([...h.listeners.values()].every((set) => set.size === 0)).toBe(true);
});
it('ignores late status and captured event callbacks after provider cleanup', async () => {
  const h = harness(7, 'tokens', ready, true);
  const view = mount(h.source);
  await screen.findByTestId('dashboard-summary');
  await settle();
  const callbacks = [...h.listeners.values()].flatMap((set) => [...set]);
  await act(async () => h.emit(events.importReceived));
  const client = view.client();
  view.unmount();
  const before = h.calls.length;
  const states = client
    .getQueryCache()
    .getAll()
    .map((query) => query.state);
  await act(async () => {
    h.firstIndex.resolve(ready);
    h.oldTokens.resolve(tokens(7, false));
    callbacks.forEach((callback) => callback());
  });
  await settle(560);
  expect(h.calls).toHaveLength(before);
  expect(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.state),
  ).toEqual(states);
  expect([...h.listeners.values()].every((set) => set.size === 0)).toBe(true);
});
it('keeps preview unavailable with no Retry or cold data/index reads', async () => {
  const h = harness();
  h.source.kind = 'preview';
  mount(h.source);
  await settle();
  expect(h.source.dashboard).not.toHaveBeenCalled();
  expect(h.source.tokensByHost).not.toHaveBeenCalled();
  expect(h.source.nativeIndexStatus).not.toHaveBeenCalled();
  expect(screen.getByText('Recorded tokens unavailable')).toBeTruthy();
  expect(sidebar().queryByText('Reading recorded tokens…')).toBeNull();
  expect(sidebar().queryByText('No host measurements')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Retry recorded tokens' })).toBeNull();
});

it('refreshes Settings counts whose first read is pending at first settled recovery', async () => {
  const h = harness(14);
  const old = deferred<typeof exported.db_counts>();
  const oldCounts = { ...exported.db_counts, sessions: 11 };
  const freshCounts = { ...exported.db_counts, sessions: 22 };
  h.source.dbCounts.mockImplementationOnce(() => old.promise).mockResolvedValue(freshCounts);
  const view = mount(h.source, '/settings');
  await waitFor(() => expect(h.source.dbCounts).toHaveBeenCalledTimes(2));
  await act(async () => old.resolve(oldCounts));
  await settle();
  expect(view.client().getQueryData(queryKeys.dbCounts)).toEqual(freshCounts);
  expect(screen.getByText('22', { selector: '.xt-database-summary dd span' })).toBeTruthy();
  expect(view.client().getQueryState(queryKeys.dbCounts)?.fetchStatus).toBe('idle');
  expect(view.client().getQueryState(queryKeys.dbCounts)?.isInvalidated).toBe(false);
});
it('refreshes the actual Sessions infinite query whose first read is pending on import', async () => {
  const h = harness(14, 'tokens', scanning);
  const row = exported.sessions.rows[0];
  const old = deferred<{ rows: typeof exported.sessions.rows; next: null }>();
  h.source.sessionsList
    .mockImplementationOnce(() => old.promise)
    .mockResolvedValue({ rows: [{ ...row, record_count: 222 }], next: null });
  const view = mount(h.source, '/sessions');
  await settle();
  expect(h.source.sessionsList).toHaveBeenCalledTimes(1);
  await act(async () => h.emit(events.importReceived));
  await waitFor(() => expect(h.source.sessionsList).toHaveBeenCalledTimes(2));
  await act(async () => old.resolve({ rows: [{ ...row, record_count: 111 }], next: null }));
  const table = within(screen.getByRole('table', { name: 'Indexed sessions' }));
  await table.findByRole('cell', { name: '222' });
  expect(table.queryByRole('cell', { name: '111' })).toBeNull();
  const query = view
    .client()
    .getQueryCache()
    .find({ queryKey: [...queryKeys.sessions, '', null] })!;
  expect(query.state.status).toBe('success');
  expect(query.state.fetchStatus).toBe('idle');
  expect(query.state.isInvalidated).toBe(false);
  expect(
    (query.state.data as { pages: { rows: typeof exported.sessions.rows }[] }).pages[0].rows[0]
      .record_count,
  ).toBe(222);
});
it('preserves a disabled matching observer and an unrelated metrics-other query', async () => {
  const h = harness();
  const view = mount(h.source);
  await settle();
  let disabledReads = 0;
  let unrelatedReads = 0;
  const disabled = new QueryObserver(view.client(), {
    queryKey: ['metrics', 'disabled', 14],
    queryFn: async () => ++disabledReads,
    enabled: false,
  });
  const unrelated = new QueryObserver(view.client(), {
    queryKey: ['metrics-other', 'one'],
    queryFn: async () => ++unrelatedReads,
  });
  const stopDisabled = disabled.subscribe(() => {});
  const stopUnrelated = unrelated.subscribe(() => {});
  try {
    await settle();
    await act(async () => h.releaseOld());
    await fresh(view, 7);
    expect(disabledReads).toBe(0);
    expect(unrelatedReads).toBe(1);
    expect(view.client().getQueryState(['metrics-other', 'one'])?.isInvalidated).toBe(false);
  } finally {
    stopDisabled();
    stopUnrelated();
  }
});
it('cancels an inactive cold range without eager fetching and reads fresh on rapid return', async () => {
  const h = harness(14, 'tokens');
  const view = mount(h.source);
  await fresh(view, 7);
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await settle();
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await settle();
  await act(async () => h.emit(events.importReceived));
  await waitFor(() => expect(reads(h, 30).tokens).toBe(2));
  expect(reads(h, 14)).toEqual({ dashboard: 1, tokens: 1 });
  const state = view.client().getQueryState(queryKeys.tokensByHost(14))!;
  expect(state.data).toBeUndefined();
  expect(state.fetchStatus).toBe('idle');
  expect(state.isInvalidated).toBe(true);
  await act(async () => h.oldTokens.resolve(tokens(14, false)));
  await settle();
  expect(view.client().getQueryData(queryKeys.tokensByHost(14))).toBeUndefined();
  expect(view.client().getQueryState(queryKeys.tokensByHost(14))?.isInvalidated).toBe(true);
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await fresh(view, 14);
  expect(reads(h, 14)).toEqual({ dashboard: 2, tokens: 2 });
});

it('ignores a late original rejection after recovery accepts a fresh token result', async () => {
  const h = harness();
  const view = mount(h.source);
  await settle();
  await act(async () => h.oldReport.resolve(report(7, false)));
  await fresh(view, 7);
  await act(async () => h.oldTokens.reject(new Error('late private original error')));
  await settle();
  expect(view.client().getQueryState(queryKeys.tokensByHost(7))?.status).toBe('success');
  expect(screen.queryByText('Recorded tokens unavailable')).toBeNull();
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
});
it('replaces the source while fresh replacement metric requests also remain held', async () => {
  const h = harness();
  h.holdFresh();
  const view = mount(h.source);
  await settle();
  await act(async () => h.oldReport.resolve(report(7, false)));
  await waitFor(() => expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 }));
  const oldClient = view.client();
  const next = harness(14);
  view.replace(next.source);
  await fresh(view, 7);
  const states = oldClient
    .getQueryCache()
    .getAll()
    .map((query) => query.state);
  await act(async () => {
    h.releaseOld();
    h.freshReport.reject(new Error('late replaced report error'));
    h.freshTokens.resolve(tokens(7, false));
  });
  await settle();
  expect(
    oldClient
      .getQueryCache()
      .getAll()
      .map((query) => query.state),
  ).toEqual(states);
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
  expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 });
});
it('shows a successful empty token result only after a successful read', async () => {
  const h = harness(14, 'tokens', ready, true);
  h.source.tokensByHost.mockResolvedValue({ window: tokens(7).window, hosts: [] });
  mount(h.source);
  await screen.findByTestId('dashboard-summary');
  await settle();
  expect(sidebar().queryByText('No host measurements')).toBeTruthy();
  expect(sidebar().queryByText('Reading recorded tokens…')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Retry recorded tokens' })).toBeNull();
});
it('retries Shell tokens off Dashboard without a Dashboard read or range control', async () => {
  const h = harness(7, 'tokens', ready, true);
  mount(h.source, '/prs');
  await settle();
  await act(async () => h.oldTokens.reject(new Error('private off-route error')));
  await screen.findByText('Recorded tokens unavailable');
  h.holdFresh();
  fireEvent.click(retryTokens());
  await settle();
  expect(retryTokens().disabled).toBe(true);
  expect(h.source.dashboard).not.toHaveBeenCalled();
  expect(screen.queryByRole('radio', { name: '14d' })).toBeNull();
  await act(async () => h.freshTokens.resolve(tokens(7)));
  await settle();
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
  expect(h.source.dashboard).not.toHaveBeenCalled();
});
it('keeps a held token retry from replacing the newly selected range', async () => {
  const h = harness(14, 'tokens', ready, true);
  mount(h.source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await settle();
  await act(async () => h.oldTokens.reject(new Error('private old range error')));
  await screen.findByText('Recorded tokens unavailable');
  h.holdFresh();
  fireEvent.click(retryTokens());
  await settle();
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await settle();
  expect(screen.getByText('Recorded tokens · 30d')).toBeTruthy();
  await act(async () => h.freshTokens.resolve(tokens(14, false)));
  await settle();
  expect(screen.getByText('Recorded tokens · 30d')).toBeTruthy();
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
  expect(sidebar().queryByLabelText('Claude Code tokens: 111')).toBeNull();
});
it('preserves a recoverable fresh-request failure despite late canceled success', async () => {
  const h = harness();
  h.holdFresh();
  const view = mount(h.source);
  await settle();
  await act(async () => h.oldReport.resolve(report(7, false)));
  await waitFor(() => expect(reads(h, 7)).toEqual({ dashboard: 2, tokens: 2 }));
  await act(async () => {
    h.freshReport.resolve(report(7));
    h.freshTokens.reject(new Error('private fresh error'));
  });
  await screen.findByText('Recorded tokens unavailable');
  await act(async () => h.oldTokens.resolve(tokens(7, false)));
  await settle();
  expect(view.client().getQueryState(queryKeys.tokensByHost(7))?.status).toBe('error');
  expect(view.client().getQueryData(queryKeys.tokensByHost(7))).toBeUndefined();
  expect(screen.queryByText('private fresh error')).toBeNull();
  const dashboardReads = h.source.dashboard.mock.calls.length;
  fireEvent.click(retryTokens());
  await fresh(view, 7);
  expect(h.source.dashboard).toHaveBeenCalledTimes(dashboardReads);
});
it('isolates a held manual token retry when the source is replaced', async () => {
  const h = harness(14, 'tokens', ready, true);
  const view = mount(h.source);
  await screen.findByTestId('dashboard-summary');
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await settle();
  await act(async () => h.oldTokens.reject(new Error('private old-source token error')));
  await screen.findByText('Recorded tokens unavailable');
  h.holdFresh();
  fireEvent.click(retryTokens());
  await settle();
  const oldClient = view.client();
  const next = harness(14);
  view.replace(next.source);
  await fresh(view, 7);
  const states = oldClient
    .getQueryCache()
    .getAll()
    .map((query) => query.state);
  await act(async () => h.freshTokens.resolve(tokens(14, false)));
  await settle();
  expect(
    oldClient
      .getQueryCache()
      .getAll()
      .map((query) => query.state),
  ).toEqual(states);
  expect(screen.getByText('Recorded tokens · 7d')).toBeTruthy();
  expect(sidebar().queryByLabelText('Claude Code tokens: 222')).toBeTruthy();
  expect(sidebar().queryByLabelText('Claude Code tokens: 111')).toBeNull();
});
