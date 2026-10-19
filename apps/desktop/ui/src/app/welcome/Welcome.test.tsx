import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation, useNavigate } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import { events } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import { welcomeStorageKey } from './welcome-completion';

const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

const host = (name: string, patch: Partial<NativeHostStatus> = {}): NativeHostStatus => ({
  host: name,
  state: 'pending',
  needs_attention: false,
  detail: null,
  sessions_imported: 0,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 0,
  records_enriched: 0,
  diagnostics: 0,
  ...patch,
});
const ready: NativeIndexStatus = {
  phase: { phase: 'ready' },
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/usr/bin/python3' },
  readers: { state: 'verified', commit: 'a'.repeat(40), plugin_version: '0.1.0' },
  hosts: [
    host('claude', { state: 'complete', sessions_imported: 41, records_new: 900 }),
    host('codex', { state: 'missing_source', detail: 'no Codex history under this home' }),
    host('cursor', { state: 'missing_source' }),
  ],
  needs_attention: false,
  reconciles: 1,
  files_scanned: 212,
  codex_history_gaps: null,
};

function nativeSource({
  history = false,
  status = ready,
  counts = exported.db_counts,
}: {
  history?: boolean;
  status?: NativeIndexStatus | (() => Promise<NativeIndexStatus>);
  counts?: typeof exported.db_counts;
} = {}) {
  let emit: (() => void) | undefined;
  const source = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: vi.fn(async () => ({
      ...exported.app_info,
      fixture: null,
      had_indexed_history_at_startup: history,
    })),
    dbCounts: vi.fn(async () => counts),
    dashboard: async () => exported.dashboards[0],
    tokensByHost: async () => ({
      window: exported.dashboards[0].window,
      hosts: exported.dashboards[0].tokens_by_host,
    }),
    today: async () => exported.today,
    sessionsList: async () => exported.sessions[0],
    nativeIndexStatus: vi.fn(typeof status === 'function' ? status : async () => status),
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    // These screens open no transcript; the seam is answered, never called.
    sessionTranscript: async () => ({ state: 'unavailable', reason: { reason: 'not_indexed' } }),
    cancelSessionTranscript: async () => {},
    // Pull-request refresh is not exercised by this test.
    pullRequests: async () => Promise.reject(new Error('not used by this test')),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: async () => Promise.reject(new Error('not used by this test')),
    cancelPullRequestRefresh: async () => Promise.reject(new Error('not used by this test')),
    subscribe: async (event: string, listener: () => void) => {
      if (event === events.nativeIndexStatus) emit = listener;
      return () => {};
    },
  } satisfies DataSource;
  return { source, emit: () => emit?.() };
}

let navigateTo: ((path: string) => void) | undefined;
function Probe() {
  const location = useLocation();
  navigateTo = useNavigate();
  return <output data-testid="path">{location.pathname}</output>;
}
function mount(path: string, source: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
          <Probe />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}
const path = () => screen.getByTestId('path').textContent;
const welcomeHeading = () => screen.findByRole('heading', { level: 1, name: 'Welcome to XTrace' });
const detailsToggle = () => screen.getByRole('button', { name: 'Details' });
const openDetails = () => {
  const toggle = detailsToggle();
  if (toggle.getAttribute('aria-expanded') !== 'true') fireEvent.click(toggle);
  return screen.getByRole('region', { name: 'Details' });
};
const sourceRow = (list: HTMLElement, name: string) =>
  within(list)
    .getAllByRole('listitem')
    .find((item) => within(item).queryByText(name, { exact: true }))!;

it('offers welcome at startup when no indexed history existed, even after the scan fills counts', async () => {
  // The initial scan raced ahead: live counts already show sessions, yet the
  // startup fact says the database was empty before the index started.
  const { source } = nativeSource({ counts: { sessions: 41, records: 900, usage: 300 } });
  mount('/', source);
  await welcomeHeading();
  expect(path()).toBe('/first-launch');
  const total = screen.getByText('In the local database now').closest('p')!;
  expect(await within(total).findByText('41')).toBeTruthy();
  expect(within(total).getByText('sessions')).toBeTruthy();
  const counts = within(openDetails()).getByRole('group', { name: 'Database counts' });
  expect(within(counts).getByText('41')).toBeTruthy();
  expect(within(counts).getByText('900')).toBeTruthy();
  expect(within(counts).getByText('300')).toBeTruthy();
});

it('opens Dashboard at startup for an upgrade over existing indexed history', async () => {
  const { source } = nativeSource({ history: true });
  mount('/', source);
  expect(
    await screen.findByRole('heading', { level: 1, name: 'What your agents did' }),
  ).toBeTruthy();
  expect(path()).toBe('/dashboard');
});

it('records completion only on the explicit continue, and a restart then opens Dashboard', async () => {
  const { source } = nativeSource();
  const first = mount('/', source);
  await welcomeHeading();
  // Leaving through the sidebar is not a completion.
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(localStorage.getItem(welcomeStorageKey)).toBeNull();
  first.unmount();
  const second = mount('/', source);
  await welcomeHeading();
  fireEvent.click(screen.getByRole('button', { name: 'Open Dashboard' }));
  expect(path()).toBe('/dashboard');
  expect(localStorage.getItem(welcomeStorageKey)).toBe('completed');
  second.unmount();
  mount('/', nativeSource().source);
  expect(
    await screen.findByRole('heading', { level: 1, name: 'What your agents did' }),
  ).toBeTruthy();
  expect(path()).toBe('/dashboard');
});

it('never traps navigation when storage cannot be written or read', async () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('quota');
  });
  const { source } = nativeSource();
  const view = mount('/', source);
  await welcomeHeading();
  fireEvent.click(screen.getByRole('button', { name: 'Open Dashboard' }));
  expect(path()).toBe('/dashboard');
  view.unmount();
  // Unreadable storage reads as not completed; an upgrade still skips welcome.
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('blocked');
  });
  const again = mount('/', nativeSource().source);
  await welcomeHeading();
  again.unmount();
  mount('/', nativeSource({ history: true }).source);
  expect(
    await screen.findByRole('heading', { level: 1, name: 'What your agents did' }),
  ).toBeTruthy();
});

it('preserves explicit deep links, and `/` after startup is an ordinary Dashboard link', async () => {
  const { source } = nativeSource();
  const view = mount('/sessions', source);
  expect(await screen.findByRole('heading', { level: 1, name: 'Sessions' })).toBeTruthy();
  expect(path()).toBe('/sessions');
  await act(async () => navigateTo?.('/'));
  expect(path()).toBe('/dashboard');
  view.unmount();
  // An explicit welcome address is shown even after completion or over history.
  localStorage.setItem(welcomeStorageKey, 'completed');
  mount('/first-launch', nativeSource({ history: true }).source);
  await welcomeHeading();
  expect(path()).toBe('/first-launch');
});

it('keeps Dashboard as the default when the startup fact cannot be read', async () => {
  const { source } = nativeSource();
  source.appInfo.mockRejectedValue(new Error('backend detail'));
  mount('/', source);
  expect(await screen.findByText(/App data could not be loaded/)).toBeTruthy();
  await waitFor(() => expect(path()).toBe('/dashboard'));
});

it('reads nothing in browser preview and lands on Dashboard', async () => {
  const unavailable = vi.fn(async () => {
    throw new Error('unavailable');
  });
  const source: DataSource = {
    kind: 'preview',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    dashboard: unavailable,
    tokensByHost: unavailable,
    today: unavailable,
    sessionsList: unavailable,
    appInfo: unavailable,
    dbCounts: unavailable,
    sessionRow: unavailable,
    sessionStretches: unavailable,
    sessionTranscript: unavailable,
    cancelSessionTranscript: unavailable,
    pullRequests: unavailable,
    pullRequestAnalytics: unavailable,
    pullRequestSessions: unavailable,
    refreshPullRequests: unavailable,
    cancelPullRequestRefresh: unavailable,
    nativeIndexStatus: unavailable,
    subscribe: async () => () => {},
  };
  const view = mount('/', source);
  await act(async () => {});
  expect(path()).toBe('/dashboard');
  view.unmount();
  mount('/first-launch', source);
  await welcomeHeading();
  expect(screen.getByText('Open the desktop app to read local history.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Open Dashboard' })).toBeTruthy();
  expect(unavailable).not.toHaveBeenCalled();
});

it('reports a scan in progress by transcripts read, never as a percentage, and follows status events', async () => {
  let status: NativeIndexStatus = {
    ...ready,
    phase: { phase: 'scanning' },
    freshness: { freshness: 'unknown' },
    python: { state: 'resolving' },
    hosts: ready.hosts.map((h) => host(h.host)),
    reconciles: 0,
    files_scanned: 128,
    codex_history_gaps: null,
  };
  const { source, emit } = nativeSource({ status: async () => status });
  mount('/first-launch', source);
  await welcomeHeading();
  expect(await screen.findByText('Scanning')).toBeTruthy();
  expect(screen.getAllByText('Waiting to be read')).toHaveLength(3);
  expect(screen.getAllByText('Will be read in this scan.')).toHaveLength(3);
  expect(screen.getByText('Looking for Python…')).toBeTruthy();
  const details = openDetails();
  expect(within(details).getByTestId('welcome-files').textContent).toBe('128');
  expect(within(details).getByText('Looking…')).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/%|of \d+ files|\d+ \/ \d+/);
  status = { ...ready };
  await act(async () => emit());
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 600));
  });
  expect(await screen.findByText('Ready')).toBeTruthy();
  expect(screen.getByTestId('welcome-files').textContent).toBe('212');
});

it('shows each source outcome without install, connection or capture claims', async () => {
  const { source } = nativeSource({
    status: {
      ...ready,
      python: { state: 'missing', reason: 'python3 was not found' },
      freshness: { freshness: 'degraded', reason: 'the watcher could not be registered' },
      hosts: [
        host('claude', {
          state: 'incomplete',
          sessions_imported: 40,
          sessions_partial: 2,
          sessions_skipped: 1,
          skipped_conversations: [],
          skipped_conversations_omitted: 0,
          records_new: 880,
          diagnostics: 3,
          needs_attention: true,
        }),
        host('codex', {
          state: 'missing_runtime',
          detail: 'python3 was not found',
          needs_attention: true,
        }),
        host('cursor', {
          state: 'reader_failed',
          detail: 'the reader exited with status 1',
          needs_attention: true,
        }),
      ],
      needs_attention: true,
    },
  });
  mount('/first-launch', source);
  const list = await screen.findByRole('list', { name: 'Local history sources' });
  // Ready can carry failures: the phase says so rather than a clean Ready.
  expect(screen.getByText('Ready with problems')).toBeTruthy();
  expect(screen.getByTestId('welcome-phase').textContent).toContain('3 sources reported a problem');
  // At first glance: the outcome, and a next step in words, never the raw error.
  const claude = sourceRow(list, 'Claude Code');
  expect(within(claude).getByText('Read with gaps')).toBeTruthy();
  expect(within(claude).getByText('40 sessions')).toBeTruthy();
  const codex = sourceRow(list, 'Codex');
  expect(within(codex).getByText('Needs Python 3')).toBeTruthy();
  expect(
    within(codex).getByText(
      'Install Python 3.10 or newer, then reopen XTrace to read this history.',
    ),
  ).toBeTruthy();
  const cursor = sourceRow(list, 'Cursor');
  expect(within(cursor).getByText('Reader failed')).toBeTruthy();
  expect(
    within(cursor).getByText(
      'This history could not be read. See Details, or Settings → Native index.',
    ),
  ).toBeTruthy();
  expect(screen.getByText('Python not found')).toBeTruthy();
  expect(document.body.textContent).not.toContain('the reader exited with status 1');
  expect(document.body.textContent).not.toContain('python3 was not found');
  // Details holds the raw facts.
  const details = openDetails();
  const sources = within(details).getByRole('list', { name: 'Source details' });
  expect(sourceRow(sources, 'Claude Code').textContent).toContain('40 · 2 partial · 1 skipped');
  expect(sourceRow(sources, 'Cursor').textContent).toContain('the reader exited with status 1');
  expect(within(details).getByTestId('welcome-freshness').textContent).toBe(
    'Degraded: the watcher could not be registered',
  );
  expect(within(details).getByText(/Not found: python3 was not found/)).toBeTruthy();
  // Installing Python is a step to take; nothing claims an agent is installed or capturing.
  expect(document.body.textContent).not.toMatch(
    /not installed|is installed|connected|capturing|plugin missing|telemetry|\$ /i,
  );
});

it('reports a disabled index and a missing source plainly', async () => {
  mount(
    '/first-launch',
    nativeSource({ status: exported.native_index as NativeIndexStatus }).source,
  );
  expect(await screen.findByText('Not running')).toBeTruthy();
  expect(screen.getByTestId('welcome-phase').textContent).toBe(
    `The local index is not running: ${(exported.native_index.phase as { reason: string }).reason}`,
  );
  expect(
    screen.getByText('No sources are read while the local index is not running.'),
  ).toBeTruthy();
  cleanup();
  mount('/first-launch', nativeSource().source);
  const list = await screen.findByRole('list', { name: 'Local history sources' });
  const cursor = sourceRow(list, 'Cursor');
  expect(within(cursor).getByText('No local history found')).toBeTruthy();
  // No history is not a problem: the line says so plainly, in a calm colour.
  expect(within(cursor).getByText('No history from this tool on this Mac.')).toBeTruthy();
  expect(cursor.hasAttribute('data-attention')).toBe(false);
  expect(within(openDetails()).getByText('1 of 3 read · last scan since launch')).toBeTruthy();
});

it('offers Retry on a read failure and keeps Dashboard one click away', async () => {
  const { source } = nativeSource();
  source.nativeIndexStatus.mockRejectedValueOnce(new Error('backend detail'));
  mount('/first-launch', source);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('Local index status could not be read.');
  expect(document.body.textContent).not.toContain('backend detail');
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }));
  expect(await screen.findByText('Ready')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole('button', { name: 'Open Dashboard' }));
  expect(path()).toBe('/dashboard');
});

it('adds no polling of its own once the status is settled', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const { source } = nativeSource();
    mount('/first-launch', source);
    await screen.findByText('Ready');
    const status = source.nativeIndexStatus.mock.calls.length;
    const counts = source.dbCounts.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(source.nativeIndexStatus.mock.calls.length).toBe(status);
    expect(source.dbCounts.mock.calls.length).toBe(counts);
  } finally {
    vi.useRealTimers();
  }
});

it('counts and colours the problem sources the app reports, including a cancelled scan', async () => {
  // The app decides which scans need attention (a cancelled scan does; no
  // local history does not); Welcome counts and words exactly those.
  const { source } = nativeSource({
    status: {
      ...ready,
      hosts: [
        host('claude', { state: 'complete' }),
        host('codex', { state: 'cancelled', needs_attention: true }),
        host('cursor', { state: 'missing_source' }),
      ],
      needs_attention: true,
    },
  });
  mount('/first-launch', source);
  const list = await screen.findByRole('list', { name: 'Local history sources' });
  expect(screen.getByText('Ready with problems')).toBeTruthy();
  expect(screen.getByTestId('welcome-phase').textContent).toContain('1 source reported a problem');
  const codex = sourceRow(list, 'Codex');
  expect(within(codex).getByText('Stopped before finishing')).toBeTruthy();
  expect(codex.getAttribute('data-attention')).toBe('true');
  expect(within(sourceRow(list, 'Cursor')).getByText('No local history found')).toBeTruthy();
});

it('shows the exact texts the release check reads, with Details closed', async () => {
  // scripts/ci/hosted-ax.swift reads these as whole, separate texts on a
  // ready welcome without opening anything, and presses one Open Dashboard.
  const { source } = nativeSource();
  mount('/first-launch', source);
  await welcomeHeading();
  expect(await screen.findByText('The initial scan finished.', { exact: true })).toBeTruthy();
  expect(screen.getByText('Bundled readers verified', { exact: true })).toBeTruthy();
  expect(screen.getByText('In the local database now', { exact: true })).toBeTruthy();
  await waitFor(() => expect(document.body.textContent).not.toContain('Reading database counts'));
  expect(screen.getAllByRole('button', { name: 'Open Dashboard' })).toHaveLength(1);
  expect(detailsToggle().getAttribute('aria-expanded')).toBe('false');
  expect(screen.queryByTestId('welcome-files')).toBeNull();
});

it('keeps Reading database counts as the loading text until counts arrive', async () => {
  let resolve: ((counts: typeof exported.db_counts) => void) | undefined;
  const { source } = nativeSource();
  source.dbCounts.mockImplementation(
    () => new Promise<typeof exported.db_counts>((done) => (resolve = done)),
  );
  mount('/first-launch', source);
  await welcomeHeading();
  const total = screen.getByText('In the local database now').closest('p')!;
  expect(total.textContent).toContain('Reading database counts');
  await act(async () => resolve?.(exported.db_counts));
  await waitFor(() => expect(total.textContent).not.toContain('Reading database counts'));
});

it('keeps technical details closed until Details is opened, and closes them again', async () => {
  mount('/first-launch', nativeSource().source);
  await welcomeHeading();
  await screen.findByText('Ready');
  const toggle = detailsToggle();
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  const region = document.getElementById(toggle.getAttribute('aria-controls')!)!;
  expect(region.hidden).toBe(true);
  expect(screen.queryByTestId('welcome-freshness')).toBeNull();
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(region.hidden).toBe(false);
  expect(within(region).getByTestId('welcome-freshness').textContent).toBe(
    'Live: new activity is added as it happens',
  );
  expect(within(region).getByText('Found at /usr/bin/python3')).toBeTruthy();
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(region.hidden).toBe(true);
});

it('names each page it points to by its own title, as plain text', async () => {
  mount('/first-launch', nativeSource().source);
  const step = (await screen.findByRole('heading', { level: 2, name: "What you'll find" })).closest(
    'section',
  )!;
  for (const title of ['Dashboard', 'Sessions', 'Scores', 'Pull requests'])
    expect(within(step).getByText(title, { exact: true })).toBeTruthy();
  expect(within(step).queryAllByRole('link')).toHaveLength(0);
  expect(
    screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent),
  ).toEqual(
    expect.arrayContaining(['Finding your agent history', "What you'll find", 'Stays on this Mac']),
  );
});
