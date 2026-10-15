import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource } from '../data/DataSource';
import type { DashboardMetrics } from '../data/generated/DashboardMetrics';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import type { TokensByHost } from '../data/generated/TokensByHost';
import { queryKeys } from '../data/query-client';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';
import { windowLabel } from './dashboard/present';

const exported = fixture as FixtureExport;
const report = (days = 7) =>
  structuredClone(exported.dashboards.find((entry) => entry.window.days === days)!);
const hosts = (days = 7): TokensByHost => {
  const current = report(days);
  return { window: current.window, hosts: current.tokens_by_host };
};
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
};

/** Independent requests: neither metrics method calls the other's callback or promise. */
function source() {
  const index = deferred<NativeIndexStatus>();
  return {
    kind: 'native' as const,
    dashboard: vi.fn(async (days: number) => report(days)),
    tokensByHost: vi.fn(async (days: number) => hosts(days)),
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    sessionsList: async () => ({ rows: [], next: null }),
    // Keep startup recovery pending here to isolate the period's two report observers.
    // Existing Dashboard/NativeIndex cases verify actual first-settled recovery separately.
    nativeIndexStatus: vi.fn(() => index.promise),
    subscribe: async () => () => {},
  } satisfies DataSource;
}

function CaptureClient({ onClient }: { onClient: (client: QueryClient) => void }) {
  onClient(useQueryClient());
  return null;
}
function TestApp({
  data,
  path = '/dashboard',
  onClient,
}: {
  data: DataSource;
  path?: string;
  onClient: (client: QueryClient) => void;
}) {
  return (
    <ThemeProvider>
      <DataProvider source={data}>
        <CaptureClient onClient={onClient} />
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  );
}
function mount(data: DataSource, path = '/dashboard') {
  let client!: QueryClient;
  const onClient = (current: QueryClient) => (client = current);
  const view = render(<TestApp data={data} path={path} onClient={onClient} />);
  return {
    client: () => client,
    replace: (next: DataSource) =>
      view.rerender(<TestApp data={next} path={path} onClient={onClient} />),
  };
}
const loaded = () => screen.findByTestId('dashboard-summary');
const settle = () => act(async () => void (await new Promise((done) => setTimeout(done, 30))));
const periodText = () => screen.queryByTestId('report-period')?.textContent;
const expectedPeriod = (current: DashboardMetrics) =>
  `${windowLabel(current.window)}, time zone ${current.window.timezone}`;
const shiftWindow = <T extends DashboardMetrics['window']>(window: T, ms: number): T => ({
  ...window,
  start_ms: window.start_ms + ms,
  end_ms: window.end_ms + ms,
});

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

it('hides fast host-token dates while the independent Dashboard is pending or failed, then uses its retry', async () => {
  const data = source();
  const held = deferred<DashboardMetrics>();
  const current = report();
  const host = hosts();
  host.window = shiftWindow(host.window, 86_400_000);
  data.dashboard.mockImplementationOnce(() => held.promise).mockResolvedValue(current);
  data.tokensByHost.mockResolvedValue(host);
  mount(data);
  await screen.findByLabelText('Claude Code tokens: 1100');
  expect.soft(screen.queryByTestId('report-period')).toBeNull();
  expect(screen.queryByTestId('dashboard-summary')).toBeNull();
  await act(async () => held.reject(new Error('synthetic Dashboard failure')));
  await screen.findByText('Dashboard metrics could not be loaded.', { exact: false });
  expect.soft(screen.queryByTestId('report-period')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await loaded();
  await settle();
  expect(periodText()).toBe(expectedPeriod(current));
  expect(data.dashboard).toHaveBeenCalledTimes(2);
  expect(data.tokensByHost).toHaveBeenCalledTimes(1);
});

it('shows Dashboard dates while host tokens are independently pending and then fail', async () => {
  const data = source();
  const held = deferred<TokensByHost>();
  data.tokensByHost.mockImplementation(() => held.promise);
  mount(data);
  await loaded();
  await settle();
  expect.soft(periodText()).toBe(expectedPeriod(report()));
  await act(async () => held.reject(new Error('synthetic token failure')));
  await screen.findByText('Recorded tokens unavailable');
  expect(periodText()).toBe(expectedPeriod(report()));
  expect(data.dashboard).toHaveBeenCalledTimes(1);
});

it('uses the exact Dashboard window and UTC zone when the two clock samples straddle midnight by 1ms', async () => {
  const data = source();
  const current = report();
  const host = hosts();
  host.window = {
    ...host.window,
    start_ms: Date.parse('2026-09-01T00:00:00.000Z'),
    end_ms: Date.parse('2026-09-08T00:00:00.000Z'),
    timezone: 'UTC',
  };
  current.window = shiftWindow(host.window, 1);
  data.dashboard.mockResolvedValue(current);
  data.tokensByHost.mockResolvedValue(host);
  mount(data);
  await loaded();
  await screen.findByLabelText('Claude Code tokens: 1100');
  await settle();
  expect(periodText()).toMatch(/^Sep 1\s–\s8, 2026, time zone UTC$/);
  expect(screen.getByTestId('report-period').getAttribute('title')).toMatch(
    /^Sep 1\s–\s8, 2026 · UTC$/,
  );
  expect(windowLabel(host.window)).toMatch(/^Sep 1\s–\s7, 2026$/);
});

it('hides retained cached dates after a Dashboard background failure and restores the successful retry', async () => {
  const data = source();
  const view = mount(data);
  await loaded();
  await settle();
  data.dashboard.mockRejectedValueOnce(new Error('synthetic background failure'));
  await act(async () => {
    await view.client().invalidateQueries({ queryKey: queryKeys.dashboard(7) });
  });
  await screen.findByText('Dashboard metrics could not be loaded.', { exact: false });
  expect(view.client().getQueryData(queryKeys.dashboard(7))).toBeTruthy();
  expect(view.client().getQueryState(queryKeys.dashboard(7))?.status).toBe('error');
  expect.soft(screen.queryByTestId('report-period')).toBeNull();
  const fresh = report();
  fresh.window = shiftWindow(fresh.window, 86_400_000);
  data.dashboard.mockResolvedValue(fresh);
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await loaded();
  await settle();
  expect(periodText()).toBe(expectedPeriod(fresh));
  expect(data.tokensByHost).toHaveBeenCalledTimes(1);
});

it('keeps the newest Dashboard range after older report and host-token responses complete last', async () => {
  const data = source();
  const older = deferred<DashboardMetrics>();
  const olderHosts = deferred<TokensByHost>();
  const monthHosts = deferred<TokensByHost>();
  const month = report(30);
  month.tiles.sessions.value = 330;
  data.dashboard.mockImplementation(async (days) =>
    days === 14 ? older.promise : days === 30 ? month : report(days),
  );
  data.tokensByHost.mockImplementation(async (days) =>
    days === 14 ? olderHosts.promise : days === 30 ? monthHosts.promise : hosts(days),
  );
  mount(data);
  await loaded();
  fireEvent.click(screen.getByRole('radio', { name: '14d' }));
  await waitFor(() => expect(data.dashboard).toHaveBeenCalledWith(14));
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() =>
    expect(screen.getByTestId('dashboard-summary').textContent).toContain('330 sessions'),
  );
  await settle();
  expect.soft(periodText()).toBe(expectedPeriod(month));
  await act(async () => {
    older.resolve(report(14));
    olderHosts.resolve(hosts(14));
  });
  const independentMonthHosts = hosts(30);
  independentMonthHosts.window = shiftWindow(independentMonthHosts.window, 86_400_000);
  await act(async () => monthHosts.resolve(independentMonthHosts));
  await settle();
  expect(periodText()).toBe(expectedPeriod(month));
  expect(screen.getByTestId('dashboard-summary').textContent).toContain('330 sessions');
  expect(screen.getByRole('radio', { name: '30d' }).getAttribute('aria-checked')).toBe('true');
});

it('ignores old-source completions after the provider replaces the query runtime', async () => {
  const old = source();
  const oldReport = deferred<DashboardMetrics>();
  const oldHosts = deferred<TokensByHost>();
  old.dashboard.mockImplementation(() => oldReport.promise);
  old.tokensByHost.mockImplementation(() => oldHosts.promise);
  const view = mount(old);
  await waitFor(() => expect(old.dashboard).toHaveBeenCalledTimes(1));
  const next = source();
  const nextReport = report();
  nextReport.tiles.sessions.value = 222;
  nextReport.window = shiftWindow(nextReport.window, 3 * 86_400_000);
  const nextHosts = hosts();
  nextHosts.window = shiftWindow(nextHosts.window, 5 * 86_400_000);
  next.dashboard.mockResolvedValue(nextReport);
  next.tokensByHost.mockResolvedValue(nextHosts);
  view.replace(next);
  await loaded();
  await act(async () => {
    oldReport.resolve(report());
    oldHosts.resolve(hosts());
  });
  await settle();
  expect(periodText()).toBe(expectedPeriod(nextReport));
  expect(screen.getByTestId('dashboard-summary').textContent).toContain('222 sessions');
  expect(next.dashboard).toHaveBeenCalledTimes(1);
});

it('does not request a full report for a header first opened off Dashboard or after navigating away', async () => {
  const data = source();
  mount(data, '/settings');
  await screen.findByText('fixture F1');
  await settle();
  expect(data.dashboard).not.toHaveBeenCalled();
  expect(screen.queryByTestId('report-period')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Dashboard' }));
  await loaded();
  await settle();
  expect(data.dashboard).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  await settle();
  expect(screen.queryByTestId('report-period')).toBeNull();
  expect(data.dashboard).toHaveBeenCalledTimes(1);
});

it('makes no full-report, host-token or index requests in browser preview', async () => {
  const data = { ...source(), kind: 'preview' as const };
  mount(data);
  await screen.findByText('Open the desktop app to read local Dashboard metrics.');
  expect(screen.queryByTestId('report-period')).toBeNull();
  expect(data.dashboard).not.toHaveBeenCalled();
  expect(data.tokensByHost).not.toHaveBeenCalled();
  expect(data.nativeIndexStatus).not.toHaveBeenCalled();
});

it('coalesces the page and header consumers into one cold and one request per uncached range', async () => {
  const data = source();
  const fortnight = deferred<DashboardMetrics>();
  const month = deferred<DashboardMetrics>();
  data.dashboard.mockImplementation(async (days) =>
    days === 14 ? fortnight.promise : days === 30 ? month.promise : report(days),
  );
  mount(data);
  await loaded();
  await settle();
  expect(data.dashboard).toHaveBeenCalledTimes(1);
  for (const [days, held] of [
    [14, fortnight],
    [30, month],
  ] as const) {
    fireEvent.click(screen.getByRole('radio', { name: `${days}d` }));
    await waitFor(() => expect(data.dashboard).toHaveBeenCalledWith(days));
    await settle();
    expect(data.dashboard.mock.calls.filter(([range]) => range === days)).toHaveLength(1);
    await act(async () => held.resolve(report(days)));
    await loaded();
    await settle();
    expect(periodText()).toBe(expectedPeriod(report(days)));
    expect(data.dashboard.mock.calls.filter(([range]) => range === days)).toHaveLength(1);
  }
  expect(data.dashboard).toHaveBeenCalledTimes(3);
  expect(data.tokensByHost).toHaveBeenCalledTimes(3);
});
