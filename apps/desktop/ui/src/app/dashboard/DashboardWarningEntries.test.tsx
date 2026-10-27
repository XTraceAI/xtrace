import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import synthetic from '../session-warnings.synthetic.json';
import { DataProvider } from '../../data/DataProvider';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { SessionTitleControls } from '../../data/DataSource';
import { useDayWarnings } from './HumanTimeline';

afterEach(cleanup);

it('uses the report window and zone, names warnings with native host titles, and shares whole-day cache entries', async () => {
  const served = {
    ...structuredClone(fixture),
    ...structuredClone(synthetic),
  } as unknown as FixtureExport;
  const report = served.dashboards[0]!;
  const source = new FixtureDataSource(served);
  const read = vi.spyOn(source, 'windowWarnings');
  const titles: SessionTitleControls = {
    read: vi.fn(async (ids: readonly string[]) => ({
      titles: ids.map((id) => ({ id, title: `Host title ${id}` })),
    })),
    cancel: vi.fn(async () => {}),
  };
  Object.defineProperty(source, 'titles', { value: titles });
  function Probe({ end = report.window.end_ms }: { end?: number }) {
    const warnings = useDayWarnings({ ...report, window: { ...report.window, end_ms: end } });
    return (
      <output>
        {[...warnings.byDate.values()]
          .flatMap((day) => day.warnings.map((entry) => warnings.nameOf(entry)))
          .join('|')}
      </output>
    );
  }
  const app = (end?: number) => (
    <DataProvider source={source}>
      <Probe end={end} />
    </DataProvider>
  );
  const view = render(app());
  await screen.findByText(/Host title s-warned/);
  expect(read).toHaveBeenCalledExactlyOnceWith(
    report.window.days,
    report.window.end_ms,
    report.window.timezone,
  );
  view.rerender(app(report.window.end_ms + 1000));
  await waitFor(() => expect(read).toHaveBeenCalledOnce());
  expect(titles.read).toHaveBeenCalled();
});
