import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SourceRecord } from '../src/data/generated/SourceRecord';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

// Generated words and identities only; no local history or provider is read.
const source = new FixtureDataSource(fixture as FixtureExport);
const mode = new URLSearchParams(location.search).get('mode');
const row = {
  ...(fixture as FixtureExport).sessions[0].rows[0],
  id: 'synthetic-layout',
  repo: `/sample/${'long-repository-'.repeat(12)}`,
  branch: 'layout',
  record_count: 1200,
  pr_links: Array.from({ length: 24 }, (_, number) => ({
    repository: 'sample/layout',
    number,
    url: `https://github.com/sample/layout/pull/${number}`,
    confidence: 'exact' as const,
    title: null,
  })),
};
const records: SourceRecord[] = Array.from({ length: mode === 'short' ? 1 : 60 }, (_, n) => ({
  id: `record-${n}`,
  role: 'assistant',
  at: null,
  blocks: [
    {
      kind: 'text',
      index: 0,
      text: `Synthetic message ${n}. ${'Readable sample text. '.repeat(8)}`,
    },
  ],
}));
records.push({
  id: 'exact-target',
  role: 'assistant',
  at: null,
  blocks: [
    {
      kind: 'tool_call',
      index: 3,
      name: 'Read',
      call_id: 'synthetic-call',
      input: { kind: 'text', text: 'Synthetic input\n'.repeat(60) },
    },
  ],
});
source.sessionRow = async () => row;
source.sessionTranscript = async () =>
  mode === 'failed'
    ? { state: 'unavailable', reason: { reason: 'replaced' } }
    : {
        state: 'loaded',
        generation: { generation: 'indexed', appended: true },
        sources: 1,
        dropped_records: 0,
        gaps: [{ gap: 'missing' }],
        records,
      };
source.sessionStretches = async () => ({
  state: 'measured',
  repeat_thresholds: { active_ms: 240_000, repeats: 5 },
  stretches: Array.from({ length: mode === 'compact' ? 1 : 48 }, (_, n) => ({
    start_uuid: `start-${n}`,
    end_uuid: `end-${n}`,
    start: '2026-09-07T12:00:00Z',
    end: '2026-09-07T12:03:00Z',
    duration_ms: 180_000,
    first_tool: { record_uuid: 'exact-target', block_index: 3 },
    active_duration_ms: 180_000,
    repeats: { state: 'measured', repeats: 0, worst: null },
    circling: false,
  })),
});
source.nativeIndexStatus = () => new Promise(() => {});
createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    <DataProvider source={source}>
      <MemoryRouter
        initialEntries={['/sessions/synthetic-layout?q=synthetic&host=claude&range=30d']}
      >
        <AppRoutes />
      </MemoryRouter>
    </DataProvider>
  </ThemeProvider>,
);
