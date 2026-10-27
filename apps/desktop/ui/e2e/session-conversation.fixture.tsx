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
