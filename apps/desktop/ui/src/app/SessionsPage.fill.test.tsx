import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionRow } from '../data/generated/SessionRow';
import { SessionsPage } from './SessionsPage';

vi.mock('./useAppInfo', () => ({ useNativeIndexStatus: () => ({ data: undefined }) }));

const base = (fixture as FixtureExport).sessions[0];
const row = (id: string, child = false): SessionRow => ({
  ...base.rows[0],
  id,
  title: id,
  host: 'codex',
  known_child: child,
  child_check: child ? 'child' : 'checked',
  parent: child
    ? { session_id: 'main', host: 'codex', title: 'main', evidence: 'native_spawn' }
    : null,
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

/** jsdom has no layout: supply the viewport and the drawn DOM's height. */
function layout(initialHeight: number) {
  let height = initialHeight;
  const observers = new Set<() => void>();
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (
    this: HTMLElement,
  ) {
    return this.classList.contains('xt-table-scroll') ? height : 0;
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement,
  ) {
    const size = this.classList.contains('xt-data-table')
      ? 30 + this.querySelectorAll('.xt-table-block').length * 40
      : this.classList.contains('xt-table-head')
        ? 30
        : 0;
    return {
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 900,
      bottom: size,
      width: 900,
      height: size,
      toJSON: () => ({}),
    };
  });
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private callback: () => void) {
        observers.add(callback);
      }
      observe() {}
      disconnect() {
        observers.delete(this.callback);
      }
    },
  );
  return (next: number) =>
    act(() => {
      height = next;
      for (const observer of [...observers]) observer();
    });
}
function mount(source: FixtureDataSource) {
  render(
    <MemoryRouter>
      <DataProvider source={source}>
        <SessionsPage />
      </DataProvider>
    </MemoryRouter>,
  );
}

it('keeps paging through collapsed groups, stops when filled, and fills again on resize', async () => {
  const resize = layout(250);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (_filter, after) => {
    const page = Number(after ?? 0);
    return {
      ...base,
      rows:
        page === 0
          ? [row('main'), ...Array.from({ length: 49 }, (_, i) => row(`child-${i}`, true))]
          : page === 1
            ? Array.from({ length: 50 }, (_, i) => row(`more-child-${i}`, true))
            : Array.from({ length: 8 }, (_, i) => row(`page-${page}-${i}`)),
      next: page < 3 ? String(page + 1) : null,
    };
  });
  mount(source);
  await screen.findByText('page-2-0');
  expect(list.mock.calls.map((call) => call[1])).toEqual([null, '1', '2']);
  expect(screen.getAllByRole('link', { name: /^Open session/ })).toHaveLength(9);
  resize(350);
  expect(list).toHaveBeenCalledTimes(3);
  resize(1000);
  await screen.findByText('page-3-0');
  expect(list).toHaveBeenCalledTimes(4);
  resize(2000);
  expect(list).toHaveBeenCalledTimes(4);
  expect(screen.queryByRole('button', { name: 'Load more sessions' })).toBeNull();
});

it('fills a new filter from its own cursor and does not use the empty-state height', async () => {
  layout(100);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (filter, after) => ({
    ...base,
    rows: after
      ? [row(`${filter.search || 'all'}-visible`)]
      : [{ ...row('hidden', true), parent: null }],
    next: after ? null : `${filter.search || 'all'}-next`,
  }));
  mount(source);
  await screen.findByText('all-visible');
  fireEvent.change(screen.getByLabelText('Search sessions'), { target: { value: 'filtered' } });
  await screen.findByText('filtered-visible');
  expect(list.mock.calls.map(([filter, after]) => [filter.search, after])).toEqual([
    ['', null],
    ['', 'all-next'],
    ['filtered', null],
    ['filtered', 'filtered-next'],
  ]);
});

it('stops after a page error, keeps manual retry, and never duplicates a pending request', async () => {
  const resize = layout(250);
  const source = new FixtureDataSource(fixture as FixtureExport);
  let reject!: (reason: Error) => void;
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (_filter, after) => {
    if (!after) return { ...base, rows: [row('main')], next: 'next' };
    return new Promise((_resolve, fail) => {
      reject = fail;
    });
  });
  mount(source);
  await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  resize(1000);
  resize(1200);
  expect(list).toHaveBeenCalledTimes(2);
  await act(async () => reject(new Error('Page unavailable')));
  await screen.findByRole('alert');
  resize(1400);
  expect(list).toHaveBeenCalledTimes(2);
  list.mockImplementation(async () => ({ ...base, rows: [row('retry')], next: null }));
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await screen.findByText('retry');
  expect(list).toHaveBeenCalledTimes(3);
});

it('leaves manual loading available when the visible rows already fill the table', async () => {
  layout(100);
  const source = new FixtureDataSource(fixture as FixtureExport);
  const list = vi.spyOn(source, 'sessionsList').mockImplementation(async (_filter, after) => ({
    ...base,
    rows: [row(`${after ?? 'first'}-0`), row(`${after ?? 'first'}-1`)],
    next: after ? null : 'manual',
  }));
  mount(source);
  await screen.findByText('first-0');
  expect(list).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('button', { name: 'Load more sessions' }));
  await screen.findByText('manual-0');
  expect(list).toHaveBeenCalledTimes(2);
});
