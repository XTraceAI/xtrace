import type { DataSource } from '../src/data/DataSource';
import type { CodexPageStatus } from '../src/data/generated/CodexPageStatus';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { MetricSessionStretch } from '../src/data/generated/MetricSessionStretch';
import type { SessionRow } from '../src/data/generated/SessionRow';
import type { SourceRecord } from '../src/data/generated/SourceRecord';

/** Authored synthetic conversation, using the same data boundary as the native page. */
function conversationFixture(source: FixtureDataSource, row: SessionRow) {
  const text = (id: string, role: 'user' | 'assistant', content: string): SourceRecord => ({
    id,
    role,
    at: null,
    blocks: [{ kind: 'text', index: 0, text: content }],
  });
  const call = (index: number, name: string) => ({
    kind: 'tool_call' as const,
    index,
    name,
    call_id: `call-${index}`,
    input: { kind: 'json' as const, value: { command: 'synthetic configuration check' } },
  });
  const records: SourceRecord[] = [
    text('prompt', 'user', 'Please rename the sample app and keep its project folder name.'),
    text('answer', 'assistant', 'I’ll check the configuration and visible labels.'),
    { id: 'tools', role: 'assistant', at: null, blocks: [call(0, 'Read'), call(1, 'Bash')] },
    {
      id: 'result',
      role: 'user',
      at: null,
      blocks: [
        {
          kind: 'tool_result',
          index: 0,
          call_id: 'call-0',
          failed: false,
          parts: [{ kind: 'text', text: 'Synthetic configuration content.' }],
        },
      ],
    },
    text('followup', 'user', 'Keep the saved settings, too.'),
    {
      id: 'mixed',
      role: 'assistant',
      at: null,
      blocks: [
        {
          kind: 'text',
          index: 0,
          text: 'The display name and settings folder are configured separately.',
        },
        call(2, 'Read'),
        call(3, 'mcp__workspace__update_plan'),
        { kind: 'text', index: 4, text: 'I’ll update the visible name and preserve the settings.' },
      ],
    },
  ];
  const repeated = (
    record_uuid: string,
    block_index: number,
    count: number,
    circling: boolean,
    tool = 'Read',
  ): MetricSessionStretch => ({
    start_uuid: 'prompt',
    end_uuid: record_uuid,
    start: '2026-10-19T18:05:00Z',
    end: '2026-10-19T18:10:00Z',
    duration_ms: 300_000,
    active_duration_ms: 240_000,
    first_tool: { record_uuid, block_index },
    repeats: {
      state: 'measured',
      repeats: count - 1,
      worst: { tool_name: tool, count, representative: { record_uuid, block_index } },
    },
    circling,
  });
  source.sessionRow = async (id) => ({ ...row, id, repo: 'sample-project', branch: 'rename-app' });
  source.sessionTranscript = async () => ({
    state: 'loaded',
    generation: { generation: 'indexed', appended: false },
    sources: 1,
    record_ranges: [{ start: 0, end: records.length }],
    records,
    dropped_records: 0,
    gaps: [],
  });
  source.sessionStretches = async () => ({
    state: 'measured',
    repeat_thresholds: { active_ms: 240_000, repeats: 5 },
    stretches: [
      repeated('tools', 0, 4, false),
      repeated('mixed', 3, 7, true, 'mcp__workspace__update_plan'),
    ],
  });
  if (new URLSearchParams(location.search).has('paged')) {
    const data: DataSource = source;
    const rowRead = source.sessionRow;
    source.sessionRow = async (...args) => {
      const found = await rowRead(...args);
      return found ? { ...found, host: 'codex' } : null;
    };
    source.sessionTranscript = async () => ({
      state: 'unavailable',
      reason: { reason: 'reader_limit', limit: 'source_bytes' },
    });
    const stretchesRead = source.sessionStretches;
    source.sessionStretches = async (...args) => {
      const answer = await stretchesRead(...args);
      return answer.state === 'measured'
        ? {
            ...answer,
            stretches: [
              ...answer.stretches,
              repeated('missing-synthetic-call', 0, 6, false, 'Bash'),
            ],
          }
        : answer;
    };
    const digest = 'a'.repeat(64);
    data.codexTranscriptPage = async (_id, selection): Promise<CodexPageStatus> => {
      if (
        selection.kind === 'tool_call' &&
        new URLSearchParams(location.search).has('paged-changed')
      )
        return { state: 'unavailable', reason: { reason: 'changed' } };
      let start = 0,
        end = 2,
        target: number | null = null;
      if (selection.kind === 'cursor' && selection.cursor.includes(':after:')) {
        start = 2;
        end = records.length;
      }
      if (selection.kind === 'tool_call') {
        const found = records.findIndex((record) => record.id === selection.record_uuid);
        if (found < 0) return { state: 'unavailable', reason: { reason: 'target_missing' } };
        if (
          !records[found]?.blocks.some(
            (block) => block.index === selection.block && block.kind === 'tool_call',
          )
        )
          return { state: 'unavailable', reason: { reason: 'block_missing' } };
        start = Math.max(0, found - 1);
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
}

const source = new FixtureDataSource(fixture as FixtureExport);
conversationFixture(source, fixture.sessions[0]!.rows[0]! as SessionRow);
if (new URLSearchParams(location.search).has('single-repeat')) {
  const read = source.sessionStretches;
  source.sessionStretches = async (...args) => {
    const answer = await read(...args);
    return answer.state === 'measured'
      ? { ...answer, stretches: answer.stretches.slice(0, 1) }
      : answer;
  };
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/sessions/synthetic-conversation']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
