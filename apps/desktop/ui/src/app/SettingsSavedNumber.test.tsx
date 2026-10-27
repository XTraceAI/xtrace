import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, HumanBreakControls } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { queryKeys } from '../data/query-client';
import { TauriDataSource } from '../data/TauriDataSource';
import { ThemeProvider } from '../theme/ThemeProvider';
import { invoke } from '@tauri-apps/api/core';
import { AppRoutes } from './AppRoutes';
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** A synthetic store: the length lives here, never in a real database. */
function controls(initial = 60) {
  let saved = initial;
  return {
    read: vi.fn(async () => saved),
    set: vi.fn(async (minutes: number) => {
      saved = minutes;
      return saved;
    }),
  } satisfies HumanBreakControls;
}

function source(humanBreak?: HumanBreakControls): DataSource {
  const unused = async () => Promise.reject(new Error('not used by this test'));
  return {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    dashboard: async () => exported.dashboards[0],
    tokensByHost: unused,
    today: async () => exported.today,
    sessionsList: unused,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionWarningContext: async () => ({
      state: 'unavailable' as const,
      reason: { reason: 'source' as const, source: { reason: 'not_indexed' as const } },
    }),
    sessionTranscript: unused,
    cancelSessionTranscript: async () => {},
    pullRequests: unused,
    refreshPullRequests: unused,
    cancelPullRequestRefresh: unused,
    pullRequestAnalytics: unused,
    pullRequestSessions: unused,
    nativeIndexStatus: async () => exported.native_index,
    subscribe: async () => () => {},
    humanBreak,
  };
}

async function mount(data: DataSource) {
  render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={['/settings']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByRole('heading', { name: 'Human time' });
  // The settled index status reconciles its own queries once; spy after that.
  await screen.findByTestId('native-index');
}

const saved = () => screen.getByTestId('human-break-saved');
const field = () =>
  screen.getByRole('textbox', { name: 'Break after (minutes)' }) as HTMLInputElement;
const saveButton = () => screen.getByRole('button', { name: 'Save' });
const reset = () => screen.getByRole('button', { name: 'Reset to 60 min' });
/** From now on, the query keys invalidated, in order. */
function spyInvalidate() {
  const spy = vi.spyOn(QueryClient.prototype, 'invalidateQueries');
  return () => spy.mock.calls.map(([filters]) => filters?.queryKey);
}

it('saves by keyboard, shows only the committed length and re-reads every Dashboard range', async () => {
  const store = controls();
  let finish: (() => void) | undefined;
  store.set.mockImplementationOnce(
    (minutes) => new Promise<number>((resolve) => (finish = () => resolve(minutes))),
  );
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  field().focus();
  fireEvent.change(field(), { target: { value: '80' } });
  expect(saveButton().hasAttribute('disabled')).toBe(false);
  fireEvent.submit(field().form!);
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith(80));
  // Pending: the saved value is still the old one and nothing can be re-sent.
  expect(saved().textContent).toBe('60 min');
  expect(screen.getByRole('button', { name: 'Saving…' }).hasAttribute('disabled')).toBe(true);
  expect(invalidated()).toEqual([]);
  await act(async () => finish?.());
  await waitFor(() => expect(saved().textContent).toBe('80 min'));
  expect(field().value).toBe('80');
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  expect(reset().hasAttribute('disabled')).toBe(false);
});

it('refuses invalid entries in the page without writing or refreshing', async () => {
  const store = controls(90);
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('90'));
  const invalidated = spyInvalidate();
  for (const value of ['4', '-5', '60.5', '', 'abc', '241']) {
    fireEvent.change(field(), { target: { value } });
    expect(field().getAttribute('aria-invalid')).toBe('true');
    expect(screen.getByRole('alert').textContent).toBe('Enter a whole number from 5 to 240.');
    expect(field().getAttribute('aria-describedby')).toContain(screen.getByRole('alert').id);
    expect(saveButton().hasAttribute('disabled')).toBe(true);
    fireEvent.submit(field().form!);
  }
  expect(store.set).not.toHaveBeenCalled();
  expect(invalidated()).toEqual([]);
  expect(saved().textContent).toBe('90 min');
  // Reset abandons the edit and restores the default.
  fireEvent.click(reset());
  await waitFor(() => expect(store.set).toHaveBeenCalledExactlyOnceWith(60));
  await waitFor(() => expect(saved().textContent).toBe('60 min'));
  expect(field().value).toBe('60');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
});

it('keeps the saved length and the Dashboard cache when a save fails', async () => {
  const store = controls();
  store.set.mockRejectedValueOnce(new Error('application database is closed'));
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  fireEvent.change(field(), { target: { value: '90' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The break length could not be saved. The length shown is the saved one.',
  );
  // The setting was read back once and still holds the old length.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(saved().textContent).toBe('60 min');
  // The edit stays for a retry; nothing was invalidated.
  expect(field().value).toBe('90');
  expect(invalidated()).toEqual([]);
  expect(screen.queryByText(/application database is closed/)).toBeNull();
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('90 min'));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
});

it('treats a committed save whose reply was lost as saved', async () => {
  const store = controls();
  store.set.mockImplementationOnce(async (minutes) => {
    await store.set(minutes);
    throw new Error('application database write did not report its outcome');
  });
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('70 min'));
  // Read back as the requested length: a save, with the edit and error cleared.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(field().value).toBe('70');
  expect(field().getAttribute('aria-invalid')).toBe('false');
  expect(reset().hasAttribute('disabled')).toBe(false);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  // The draft was cleared: a new edit starts from the saved length.
  fireEvent.change(field(), { target: { value: '75' } });
  expect(saveButton().hasAttribute('disabled')).toBe(false);
});

/** A write that commits and then loses its reply. */
function commitThenLoseReply(store: ReturnType<typeof controls>) {
  store.set.mockImplementationOnce(async (minutes) => {
    await store.set(minutes);
    throw new Error('application database write did not report its outcome');
  });
}

const UNKNOWN_70 =
  'The break length could not be read back, so whether 70 min was saved is unknown. The length shown was last confirmed before this save. Check the saved length before changing it; the Dashboard will read the saved length again.';
const check = () => screen.getByRole('button', { name: 'Check saved length' });

/**
 * Saves 70 over a saved 60: the write commits (or not) and loses its reply,
 * and the one read-back fails too. Returns the spy of invalidated keys.
 */
async function saveUnknown70(store: ReturnType<typeof controls>, commits = true) {
  if (commits) commitThenLoseReply(store);
  else store.set.mockRejectedValueOnce(new Error('application database is closed'));
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const invalidated = spyInvalidate();
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(UNKNOWN_70);
  return invalidated;
}

it('reports an unknown outcome after a lost reply and a failed read-back, and writes nothing more', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  // One read-back only, then one conservative Dashboard refresh.
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(screen.queryByText(/could not be saved/)).toBeNull();
  // The cached length is only the last confirmed one.
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(saved().textContent).toBe('60 min');
  expect(field().value).toBe('70');
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(check().hasAttribute('disabled')).toBe(false);
  // Neither the edit, the cached length nor the keyboard writes anything.
  fireEvent.submit(field().form!);
  fireEvent.change(field(), { target: { value: '60' } });
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  fireEvent.submit(field().form!);
  fireEvent.click(reset());
  await act(async () => {});
  // Only the lost-reply write (and the commit it wraps).
  expect(store.set.mock.calls).toEqual([[70], [70]]);
  expect(store.read).toHaveBeenCalledTimes(2);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
});

it('settles an unknown save as saved when a check reads the requested length, then resets by writing', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  fireEvent.click(check());
  await waitFor(() => expect(saved().textContent).toBe('70 min'));
  expect(store.read).toHaveBeenCalledTimes(3);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByText('Saved')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Check saved length' })).toBeNull();
  // The draft was cleared; the ranges were already re-read when the outcome became unknown.
  expect(field().value).toBe('70');
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  // Reset now writes 60 over the confirmed 70 and re-reads every range.
  fireEvent.click(reset());
  await waitFor(() => expect(saved().textContent).toBe('60 min'));
  expect(store.set.mock.calls).toEqual([[70], [70], [60]]);
  expect(invalidated()).toEqual([
    ['metrics', 'dashboard'],
    ['metrics', 'dashboard'],
  ]);
  expect(reset().hasAttribute('disabled')).toBe(true);
});

it('settles an unknown save as failed when a check reads the old length, keeping the edit to retry', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store, false);
  fireEvent.click(check());
  expect(
    (
      await screen.findByText(
        'The break length could not be saved. The length shown is the saved one.',
      )
    ).textContent,
  ).toBeTruthy();
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(screen.getByText('Saved')).toBeTruthy();
  expect(saved().textContent).toBe('60 min');
  expect(field().value).toBe('70');
  // Already the default: Reset has nothing to write; Save retries the edit.
  expect(reset().hasAttribute('disabled')).toBe(true);
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  fireEvent.click(saveButton());
  await waitFor(() => expect(saved().textContent).toBe('70 min'));
  expect(store.set.mock.calls).toEqual([[70], [70]]);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(invalidated()).toEqual([
    ['metrics', 'dashboard'],
    ['metrics', 'dashboard'],
  ]);
});

it('stays unknown when a check cannot read the setting, and a later check settles it', async () => {
  const store = controls();
  const invalidated = await saveUnknown70(store);
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.click(check());
  await waitFor(() => expect(store.read).toHaveBeenCalledTimes(3));
  await waitFor(() => expect(check().hasAttribute('disabled')).toBe(false));
  expect(screen.getByRole('alert').textContent).toBe(UNKNOWN_70);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
  // A check never refreshes the Dashboard again.
  expect(invalidated()).toEqual([['metrics', 'dashboard']]);
  fireEvent.click(check());
  await waitFor(() => expect(screen.getByText('Saved')).toBeTruthy());
  expect(saved().textContent).toBe('70 min');
  expect(store.set.mock.calls).toEqual([[70], [70]]);
});

it('lets no setting query read update or settle an unknown save', async () => {
  const store = controls();
  const mounted = vi.spyOn(QueryClient.prototype, 'mount');
  // A background read starts before the save and answers after it: whatever it
  // says, it was not read after the write, so it must not confirm anything.
  let answer: ((minutes: number) => void) | undefined;
  commitThenLoseReply(store);
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  const client = mounted.mock.contexts[0] as QueryClient;
  store.read.mockImplementationOnce(() => new Promise<number>((resolve) => (answer = resolve)));
  await act(async () => void client.invalidateQueries({ queryKey: queryKeys.humanBreak }));
  expect(store.read).toHaveBeenCalledTimes(2);
  store.read.mockRejectedValueOnce(new Error('application storage operation failed'));
  fireEvent.change(field(), { target: { value: '70' } });
  fireEvent.click(saveButton());
  expect((await screen.findByRole('alert')).textContent).toBe(UNKNOWN_70);
  await act(async () => answer?.(70));
  // Neither the stale answer nor a later refetch reads or changes anything.
  await act(async () => {
    await client.invalidateQueries({ queryKey: queryKeys.humanBreak });
    await client.refetchQueries({ queryKey: queryKeys.humanBreak });
  });
  expect(store.read).toHaveBeenCalledTimes(3);
  expect(screen.getByRole('alert').textContent).toBe(UNKNOWN_70);
  expect(screen.getByText('Last confirmed')).toBeTruthy();
  expect(client.getQueryData(queryKeys.humanBreak)).toBe(60);
  // Only the user's check settles it, from its own read.
  fireEvent.click(check());
  await waitFor(() => expect(saved().textContent).toBe('70 min'));
  expect(store.read).toHaveBeenCalledTimes(4);
  expect(screen.queryByRole('alert')).toBeNull();
});

it('sends one save at a time', async () => {
  const store = controls();
  await mount(source(store));
  await waitFor(() => expect(field().value).toBe('60'));
  fireEvent.change(field(), { target: { value: '80' } });
  act(() => {
    field().form!.requestSubmit();
    field().form!.requestSubmit();
  });
  await waitFor(() => expect(saved().textContent).toBe('80 min'));
  expect(store.set).toHaveBeenCalledExactlyOnceWith(80);
});

it('shows an unreadable setting as unavailable with no control to change it', async () => {
  const store = controls();
  store.read.mockRejectedValue(new Error('application storage operation failed'));
  await mount(source(store));
  expect((await screen.findByRole('alert')).textContent).toBe(
    'The break length could not be read.',
  );
  expect(saved().textContent).toBe('Unavailable');
  expect(field().hasAttribute('disabled')).toBe(true);
  expect(saveButton().hasAttribute('disabled')).toBe(true);
  expect(reset().hasAttribute('disabled')).toBe(true);
});

it('a committed change invalidates every Dashboard range and nothing else', async () => {
  const client = new QueryClient();
  const keys = [
    queryKeys.dashboard(7),
    queryKeys.dashboard(14),
    queryKeys.dashboard(30),
    queryKeys.today,
    queryKeys.tokensByHost(7),
    queryKeys.sessions(7),
    queryKeys.contentRetention,
  ];
  for (const key of keys) client.setQueryData(key, {});
  await client.invalidateQueries({ queryKey: queryKeys.dashboards, refetchType: 'none' });
  expect(keys.map((key) => client.getQueryState(key)?.isInvalidated)).toEqual([
    true,
    true,
    true,
    false,
    false,
    false,
    false,
  ]);
});

it('shows no typing speed setting', async () => {
  await mount(source(controls()));
  expect(screen.queryByRole('heading', { name: 'Typing speed' })).toBeNull();
  expect(screen.queryByRole('textbox', { name: 'Words per minute' })).toBeNull();
});

// Settings no longer shows the typing speed; the Dashboard's typing estimate still reads it.
it('invokes the native typing-speed commands with plain integers', async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === 'open_typing_test' ? undefined : 80,
  );
  const { typingSpeed } = new TauriDataSource();
  expect(await typingSpeed.read()).toBe(80);
  expect(await typingSpeed.set(80)).toBe(80);
  await typingSpeed.openTest();
  expect(vi.mocked(invoke).mock.calls).toEqual([
    ['typing_speed'],
    ['set_typing_speed', { wpm: 80 }],
    ['open_typing_test'],
  ]);
});
