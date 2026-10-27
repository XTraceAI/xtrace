import { StrictMode, useEffect } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, useLocation } from 'react-router';
import fixture from '../fixtures/F1.json';
import synthetic from '../src/app/session-warnings.synthetic.json';
import diagnosisSynthetic from '../src/app/session-diagnosis.synthetic.json';
import { AppRoutes } from '../src/app/AppRoutes';
import {
  contextExcerpt,
  multiSourceContextFixture,
} from '../src/app/dashboard/warning-context.synthetic';
import { DataProvider } from '../src/data/DataProvider';
import type {
  DataSource,
  DiagnosisControls,
  WarningDismissalControls,
} from '../src/data/DataSource';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionDiagnosis } from '../src/data/generated/SessionDiagnosis';
import type { SessionExcerptStatus } from '../src/data/generated/SessionExcerptStatus';
import type { SessionPageSelection } from '../src/data/generated/SessionPageSelection';
import type { SessionPageStatus } from '../src/data/generated/SessionPageStatus';
import type { SourceRecord } from '../src/data/generated/SourceRecord';
import type { SavedSkill } from '../src/data/generated/SavedSkill';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

// These controls are explicit browser-test mocks. No native calls, models, or writes.
const params = new URLSearchParams(location.search);
const path = params.get('path') ?? '/dashboard';
const { after_dismissal: after, ...answers } = synthetic;
const served = { ...structuredClone(fixture), ...structuredClone(answers) } as FixtureExport;
const before = new FixtureDataSource(served);
const later = new FixtureDataSource({
  ...served,
  session_warnings: after.session_warnings,
  dashboard_warnings: after.dashboard_warnings,
} as FixtureExport);
const source: DataSource = new FixtureDataSource(served);
let dismissed = false;
const dismissals = { dismiss: [] as string[][], restore: [] as string[][] };
const warningControls: WarningDismissalControls = {
  dismiss: async (sessionId, warningId) => {
    dismissals.dismiss.push([sessionId, warningId]);
    if (sessionId !== after.session_id || warningId !== after.warning_id)
      throw Error('No synthetic warning');
    dismissed = true;
    return { session_id: sessionId, warning_id: warningId, dismissed: true };
  },
  restore: async (sessionId, warningId) => {
    dismissals.restore.push([sessionId, warningId]);
    dismissed = false;
    return { session_id: sessionId, warning_id: warningId, dismissed: false };
  },
};
Object.defineProperties(source, {
  warningDismissals: { value: warningControls },
  sessionWarnings: {
    value: (ids: readonly string[], zone: string) =>
      (dismissed ? later : before).sessionWarnings(ids, zone),
  },
  windowWarnings: {
    value: (days: number, end: number, zone: string) =>
      (dismissed ? later : before).windowWarnings(days, end, zone),
  },
});
Object.assign(window, { __dismissals: dismissals });
source.sessionWarningContext = async () => {
  if (params.get('context') === 'large') {
    const response = await fetch('/e2e/warning-large-context.synthetic.json');
    return response.json() as Promise<SessionExcerptStatus>;
  }
  const warning = synthetic.session_warnings.sessions
    .flatMap((entry) => (entry.answer.state === 'read' ? entry.answer.warnings : []))
    .find((warning) => warning.id === after.warning_id)!;
  const subagent = params.get('context') === 'subagent';
  const status = multiSourceContextFixture(
    subagent ? 'main-target' : warning.points_at.record_uuid,
  );
  if (subagent) status.records[6]!.id = warning.points_at.record_uuid;
  // A long agent reply (Markdown with a table) before the flagged message.
  if (params.has('longreply'))
    status.records[1]!.blocks = [
      {
        kind: 'text',
        index: 0,
        text: [
          'Synthetic long reply.',
          '',
          ...Array.from({ length: 16 }, (_, line) => `Synthetic paragraph ${line + 1}.\n`),
          '| Step | Result |',
          '| --- | --- |',
          ...Array.from({ length: 22 }, (_, row) => `| Synthetic step ${row + 1} | passed |`),
        ].join('\n'),
      },
    ];
  if (params.has('long'))
    status.records[2]!.blocks = [
      { kind: 'text', index: 0, text: 'Synthetic long content. '.repeat(500) },
    ];
  return contextExcerpt(status, warning.points_at.record_uuid, warning.points_at.block);
};
if (params.has('full-session-targets')) {
  const host = params.get('host') === 'codex' ? 'codex' : 'claude';
  const reads = {
    pages: [] as SessionPageSelection[],
    pageCalls: [] as { id: string; selection: SessionPageSelection }[],
    cancelled: [] as string[],
    contexts: [] as { recordUuid: string; block: number | null }[],
    full: 0,
  };
  Object.assign(window, { __fullSessionReads: reads });
  const records: SourceRecord[] = [
    {
      id: 'w-h3',
      role: 'user',
      at: null,
      blocks: [{ kind: 'text', index: 0, text: 'Exact Dashboard correction message.' }],
    },
    {
      id: 'w-r2',
      role: 'user',
      at: null,
      blocks: [
        {
          kind: 'tool_result',
          index: 0,
          call_id: 'saved-call',
          failed: true,
          parts: [{ kind: 'text', text: 'Exact Dashboard original failed result.' }],
        },
      ],
    },
  ];
  const loaded = {
    state: 'loaded' as const,
    generation: { generation: 'indexed' as const, appended: false },
    sources: 1,
    records,
    record_ranges: [{ start: 0, end: records.length }],
    dropped_records: 0,
    gaps: [],
  };
  const row = source.sessionRow.bind(source);
  source.sessionRow = async (...args) => {
    const found = await row(...args);
    return found ? { ...found, host } : null;
  };
  source.sessionTranscript = async () => {
    reads.full += 1;
    return {
      state: 'unavailable',
      reason:
        host === 'claude'
          ? { reason: 'too_large', limit: 'bytes', reached: 99, ceiling: 10 }
          : { reason: 'reader_limit', limit: 'source_bytes' },
    };
  };
  source.sessionTranscriptPage = async (_id, selection, readId): Promise<SessionPageStatus> => {
    reads.pages.push(selection);
    reads.pageCalls.push({ id: readId, selection });
    const position =
      selection.kind === 'anchor'
        ? records.findIndex((record) => record.id === selection.record_uuid)
        : -1;
    return {
      state: 'loaded',
      generation: loaded.generation,
      digest: 'a'.repeat(64),
      records,
      record_ranges: loaded.record_ranges,
      start: 0,
      end: records.length,
      total_records: records.length,
      dropped_records: 0,
      previous: null,
      next: null,
      target:
        position >= 0 && selection.kind === 'anchor'
          ? { record: position, block: selection.block }
          : null,
    };
  };
  source.sessionWarningContext = async (_id, recordUuid, block) => {
    reads.contexts.push({ recordUuid, block });
    return contextExcerpt(loaded, recordUuid, block);
  };
  source.cancelSessionTranscript = async (readId) => {
    reads.cancelled.push(readId);
  };
}
function RouteProbe() {
  const route = useLocation();
  useEffect(() => {
    Object.assign(window, { __warningRoute: route.pathname + route.search });
  }, [route]);
  return null;
}
if (params.has('diagnosis')) {
  const calls = { runs: [] as string[][], stops: [] as string[], saves: [] as string[][] };
  let waiting: ((answer: SessionDiagnosis) => void) | undefined;
  const answers = diagnosisSynthetic.diagnoses as unknown as Record<string, SessionDiagnosis>;
  const diagnosis: DiagnosisControls = {
    diagnose: (sessionId, warningId, zone, readId) => {
      calls.runs.push([sessionId, warningId, zone, readId]);
      return new Promise((resolve) => {
        waiting = resolve;
      });
    },
    cancel: async (readId) => {
      calls.stops.push(readId);
      if (params.get('cancel') === 'deferred') return true;
      waiting?.(answers.cancelled!);
      waiting = undefined;
      return true;
    },
    saveSkill: async (sessionId, name, draft) => {
      calls.saves.push([sessionId, name, draft]);
      return diagnosisSynthetic.saved as SavedSkill;
    },
  };
  Object.defineProperty(source, 'diagnosis', { value: diagnosis });
  Object.assign(window, {
    __diagnosis: {
      calls,
      answer: (key: string) => {
        waiting?.(answers[key]!);
        waiting = undefined;
      },
    },
  });
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <RouteProbe />
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
