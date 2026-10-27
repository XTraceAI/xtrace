import { answer as warningAnswer, warning, second } from './session-warnings.data';
import type { SessionPageSelection } from '../src/data/generated/SessionPageSelection';
import type { DataSource } from '../src/data/DataSource';
import type { SessionPageStatus } from '../src/data/generated/SessionPageStatus';
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
  const parameters = new URLSearchParams(location.search);
  const warningTargets = parameters.has('warning-targets');
  const pageSelections: SessionPageSelection[] = [];
  if (warningTargets) Object.assign(window, { __warningPageSelections: pageSelections });

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
  if (warningTargets) {
    records[0]!.blocks = [
      {
        kind: 'text',
        index: 0,
        text: 'Original warning message. ' + 'Long original message '.repeat(1000),
      },
    ];
    records[3]!.blocks = [
      {
        kind: 'tool_result',
        index: 128,
        call_id: 'call-0',
        failed: true,
        parts: [
          {
            kind: 'text',
            text: 'Original warning result. ' + 'Long original returned text '.repeat(1000),
          },
        ],
      },
    ];
    const items = [
      { ...warning, points_at: { record_uuid: 'prompt', block: null } },
      { ...second, points_at: { record_uuid: 'result', block: 128 } },
    ];
    let current = { ...warningAnswer, warnings: items, dismissed: [] as typeof items };
    source.sessionWarnings = async (ids, timezone) => ({
      timezone,
      sessions: ids.map((session_id) => ({ session_id, answer: current })),
    });
    Object.defineProperty(source, 'warningDismissals', {
      value: {
        dismiss: async (session_id: string, warning_id: string) => {
          const item = current.warnings.find((item) => item.id === warning_id)!;
          current = {
            ...current,
            warnings: current.warnings.filter((item) => item.id !== warning_id),
            dismissed: [...current.dismissed, item],
          };
          return { session_id, warning_id, dismissed: true };
        },
        restore: async (session_id: string, warning_id: string) => {
          const item = current.dismissed.find((item) => item.id === warning_id)!;
          current = {
            ...current,
            warnings: [...current.warnings, item],
            dismissed: current.dismissed.filter((item) => item.id !== warning_id),
          };
          return { session_id, warning_id, dismissed: false };
        },
      },
    });
  }
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
  source.sessionRow = async (id) => ({
    ...row,
    id,
    host: parameters.get('host') ?? row.host,
    repo: 'sample-project',
    branch: 'rename-app',
  });
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
    const pagedHost =
      new URLSearchParams(location.search).get('host') === 'claude' ? 'claude' : 'codex';
    const data: DataSource = source;
    const rowRead = source.sessionRow;
    source.sessionRow = async (...args) => {
      const found = await rowRead(...args);
      return found ? { ...found, host: pagedHost } : null;
    };
    source.sessionTranscript = async () => ({
      state: 'unavailable',
      reason:
        pagedHost === 'claude'
          ? { reason: 'too_large', limit: 'bytes', reached: 99, ceiling: 10 }
          : { reason: 'reader_limit', limit: 'source_bytes' },
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
    data.sessionTranscriptPage = async (_id, selection): Promise<SessionPageStatus> => {
      pageSelections.push(selection);
      if (
        selection.kind === 'anchor' &&
        (parameters.has('paged-changed') ||
          (parameters.has('warning-changed') && selection.record_uuid === 'result'))
      )
        return { state: 'unavailable', reason: { reason: 'changed' } };
      let start = 0,
        end = 2,
        target: number | null = null;
      if (selection.kind === 'cursor' && selection.cursor.includes(':after:')) {
        start = 2;
        end = records.length;
      }
      if (selection.kind === 'anchor') {
        const found = records.findIndex((record) => record.id === selection.record_uuid);
        if (found < 0)
          return {
            state: 'unavailable',
            reason: { reason: 'read', detail: { reason: 'target_missing' } },
          };
        if (
          selection.expected === 'message'
            ? selection.block !== null
            : !records[found]?.blocks.some(
                (block) =>
                  block.index === selection.block &&
                  (selection.expected === 'tool_call'
                    ? block.kind === 'tool_call'
                    : block.kind === 'tool_call' || block.kind === 'tool_result'),
              )
        )
          return {
            state: 'unavailable',
            reason: { reason: 'read', detail: { reason: 'block_missing' } },
          };
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
        generation:
          pagedHost === 'claude'
            ? { generation: 'indexed', appended: false }
            : { generation: 'unrecorded' },
        record_ranges: [{ start: 0, end: end - start }],
        dropped_records: 0,
        total_records: records.length,
        previous: start > 0 ? `v1:before:${start}:${digest}` : null,
        next: end < records.length ? `v1:after:${end}:${digest}` : null,
        target:
          target === null
            ? null
            : { record: target, block: selection.kind === 'anchor' ? selection.block : null },
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
        <MemoryRouter
          initialEntries={[
            '/sessions/synthetic-conversation' +
              (new URLSearchParams(location.search).has('warning-targets')
                ? '?warning=' + encodeURIComponent(warning.id)
                : ''),
          ]}
        >
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
