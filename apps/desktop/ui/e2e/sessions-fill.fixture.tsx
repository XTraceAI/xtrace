import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionRow } from '../src/data/generated/SessionRow';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/** The app's real layout over matching pages of 50 synthetic sessions. */
const exported = fixture as FixtureExport;
const source = new FixtureDataSource(exported);
const base = exported.sessions[0];
const row = (id: string, child: boolean): SessionRow => ({
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
const all = Array.from({ length: 350 }, (_, index) => {
  const main = index === 0 || (index >= 100 && index % 50 < 5);
  return row(index === 0 ? 'main' : `${main ? 'conversation' : 'child'}-${index}`, !main);
});
const control = {
  reads: [] as { search: string; after: string | null; sort: string }[],
};
Object.assign(window, { __fill: control });
// Index catch-up is tested elsewhere. Leave this unrelated read pending so it
// cannot replay the synthetic pages in the middle of a layout assertion.
source.nativeIndexStatus = () => new Promise(() => {});
source.sessionsList = async (filter, after) => {
  control.reads.push({ search: filter.search, after, sort: filter.sort ?? 'started' });
  const matches = all.filter((session) =>
    session.title!.toLowerCase().includes(filter.search.toLowerCase()),
  );
  if (filter.sort === 'recently_active') matches.reverse();
  const offset = Number(after ?? 0);
  const rows = matches.slice(offset, offset + 50);
  return {
    ...base,
    rows,
    next: offset + 50 < matches.length ? String(offset + 50) : null,
    referenced_parents: [
      {
        session_id: 'main',
        host: 'codex',
        known_child: false,
        child_check: 'checked',
        parent: null,
      },
    ],
  };
};
source.sessionRow = async (id) => all.find((session) => session.id === id) ?? null;

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/sessions']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
