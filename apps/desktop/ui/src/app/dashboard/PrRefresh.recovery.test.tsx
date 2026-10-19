import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import { events } from '../../data/ipc-names';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { PrRow } from '../../data/generated/PrRow';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { PrInventory } from '../PrInventory';
import { prRow } from '../prs.synthetic';
import { PrRefresh } from './PrRefresh';
import { NO_LINKS, withPrEffort } from './pr-effort.synthetic';

afterEach(cleanup);

it('keeps manual recovery reachable with only not-found links and reveals the inventory row after access returns', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let row: PrRow = prRow(123, {
    last_attempted_at_ms: 1,
    status: { status: 'not_found_on_github' },
  });
  source.pullRequests = vi.fn(async () => ({ rows: [row] }));
  // GitHub access and the committed native response are simulated. The real
  // components and shared query invalidation handle the recovered inventory.
  source.refreshPullRequests = vi.fn(async (ids: number[]) => {
    row = prRow(123, {
      title: 'Recovered private pull request',
      state: 'open',
      refreshed_at_ms: 2,
      last_attempted_at_ms: 2,
      status: { status: 'refreshed' },
    });
    source.emit(events.prsRefreshed);
    return {
      requested: ids.length,
      attempted: ids.length,
      succeeded: ids.length,
      failed: 0,
      skipped: 0,
      unrecorded: 0,
      cancelled: false,
      committed: true,
      rows: [],
    };
  });
  const report = withPrEffort((fixture as FixtureExport).dashboards[0]!, NO_LINKS);
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter>
          <PrRefresh
            window={report.window}
            tile={report.pr_effort.current.tile}
            auto={{ enabled: true, checking: false, paused: null, last_finished_at_ms: 1 }}
          />
          <div data-testid="inventory">
            <PrInventory />
          </div>
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByText('No session links a pull request yet.');
  const button = screen.getByRole('button', { name: 'Refresh pull-request facts' });
  expect(button.dataset.state).toBe('ready');
  expect(source.refreshPullRequests).not.toHaveBeenCalled();
  fireEvent.click(button);
  const dialog = await screen.findByRole('dialog', { name: 'Refresh pull-request facts' });
  // Never-confirmed failures remain excluded from successful-check counts.
  expect(within(dialog).getByTestId('pr-refresh').textContent).toBe(
    'No session has a confirmed pull-request link yet.',
  );
  fireEvent.click(await within(dialog).findByRole('checkbox', { name: /#223/ }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Refresh 1 pull request' }));
  await waitFor(() => expect(source.refreshPullRequests).toHaveBeenCalledExactlyOnceWith([123]));
  await within(dialog).findByTestId('pr-refresh-report');
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(
    await within(screen.getByTestId('inventory')).findByText('Recovered private pull request'),
  ).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Refresh pull-request facts' })).toBeTruthy();
});
