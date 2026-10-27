import type { DataSource } from '../src/data/DataSource';
import type { CodexPageStatus } from '../src/data/generated/CodexPageStatus';
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
const parameters = new URLSearchParams(location.search);
const mode = parameters.get('mode');
const paged = parameters.has('paged');
const row = {
  ...(fixture as FixtureExport).sessions[0].rows[0],
  id: 'synthetic-layout',
  repo: `/sample/${'long-repository-'.repeat(12)}`,
  branch: 'layout',
  record_count: 1200,
  pr_links: Array.from({ length: 24 }, (_, number) => ({
    repository: 'sample/synthetic-layout-keyboard-scrolling-acceptance',
    number,
    url: `https://github.com/sample/layout/pull/${number}`,
    confidence: 'exact' as const,
    title: null,
  })),
};
const records: SourceRecord[] = Array.from(
  { length: mode === 'short' || (paged && mode === 'compact') ? 1 : 60 },
  (_, n) => ({
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
  }),
);
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
records.push({
  id: 'adjacent-tool',
  role: 'assistant',
  at: null,
  blocks: [{ kind: 'tool_call', index: 0, name: 'Bash', call_id: 'adjacent-call', input: null }],
});
source.sessionRow = async () => ({ ...row, host: paged ? 'codex' : row.host });
source.sessionTranscript = async () =>
  mode === 'failed'
    ? { state: 'unavailable', reason: { reason: 'replaced' } }
    : {
        state: 'loaded',
        generation: { generation: 'indexed', appended: true },
        sources: 1,
        record_ranges: [{ start: 0, end: records.length }],
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
    repeats: {
      state: 'measured',
      repeats: n === 0 ? 3 : 0,
      worst:
        n === 0
          ? {
              tool_name: 'Read',
              count: 4,
              representative: {
                record_uuid: mode === 'unavailable-repeat' ? 'missing-target' : 'exact-target',
                block_index: 3,
              },
            }
          : null,
    },
    circling: false,
  })),
});
if (paged) {
  source.sessionTranscript = async () => ({
    state: 'unavailable',
    reason: { reason: 'reader_limit', limit: 'source_bytes' },
  });
  const data: DataSource = source;
  const digest = 'b'.repeat(64);
  data.codexTranscriptPage = async (_id, selection): Promise<CodexPageStatus> => {
    let start = 0,
      end = Math.min(31, records.length),
      target: number | null = null;
    if (selection.kind === 'cursor') {
      const boundary = Number(selection.cursor.split(':')[2]);
      start = selection.cursor.includes(':after:') ? boundary : Math.max(0, boundary - 31);
      end = selection.cursor.includes(':after:') ? Math.min(records.length, start + 31) : boundary;
    } else if (selection.kind === 'tool_call') {
      const found = records.findIndex((record) => record.id === selection.record_uuid);
      if (found < 0) return { state: 'unavailable', reason: { reason: 'target_missing' } };
      if (
        !records[found]!.blocks.some(
          (block) => block.kind === 'tool_call' && block.index === selection.block,
        )
      )
        return { state: 'unavailable', reason: { reason: 'block_missing' } };
      start = Math.max(0, found - 16);
      end = Math.min(records.length, found + 2);
      target = found - start;
    }
    return {
      state: 'loaded',
      digest,
      records: records.slice(start, end),
      start,
      end,
      total: records.length,
      previous: start > 0 ? `v1:before:${start}:${digest}` : null,
      next: end < records.length ? `v1:after:${end}:${digest}` : null,
      target,
    };
  };
}
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
