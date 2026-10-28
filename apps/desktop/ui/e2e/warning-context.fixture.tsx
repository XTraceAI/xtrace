import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import fixture from '../fixtures/F1.json';
import { warning } from './session-warnings.data';
import { WarningContext } from '../src/app/dashboard/WarningContext';
import {
  contextExcerpt,
  contextFixture,
  contextMessage,
} from '../src/app/dashboard/warning-context.synthetic';
import { DataProvider } from '../src/data/DataProvider';
import type { DataSource } from '../src/data/DataSource';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionExcerptStatus } from '../src/data/generated/SessionExcerptStatus';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

const params = new URLSearchParams(location.search);
const source: DataSource = new FixtureDataSource(fixture as unknown as FixtureExport);
const status = contextFixture('target');
if (params.has('long'))
  status.records[2]!.blocks = [
    { kind: 'text', index: 0, text: 'SyntheticLongContext'.repeat(1500) },
  ];
if (params.has('omitted'))
  status.records[2]!.blocks.push({
    kind: 'tool_result',
    index: 99,
    call_id: 'unpaired',
    failed: true,
    parts: [{ kind: 'text', text: 'Synthetic tool failure. No linked call name is available.' }],
  });
// The person's message an admitted mistake answers, kept by the native side
// ahead of the reply's neighbours: right before them (`lead-adjacent`), or
// with an agent step between that the excerpt leaves out (`lead`).
const lead = params.has('lead') || params.has('lead-adjacent') ? 'asked' : null;
if (lead) status.records[0] = contextMessage('asked', 'Please make the sample build pass.');
if (params.has('lead')) {
  status.records.splice(1, 0, {
    ...contextMessage('skipped-step', 'Synthetic skipped agent step.'),
    role: 'assistant',
  });
  status.record_ranges = [{ start: 0, end: status.records.length }];
}
const block = params.has('omitted') ? 99 : null;
const calls = { reads: [] as unknown[][], cancels: [] as string[] };
Object.defineProperty(source, 'sessionWarningContext', {
  value: async (...args: unknown[]): Promise<SessionExcerptStatus> => {
    calls.reads.push(args);
    return params.has('refusal')
      ? { state: 'unavailable', reason: { reason: 'target_ambiguous' } }
      : contextExcerpt(status, 'target', block, [], lead);
  },
});
source.cancelSessionTranscript = async (readId: string) => {
  calls.cancels.push(readId);
};
Object.assign(window, { __context: calls });
function Sample() {
  const [shown, setShown] = useState(true);
  return (
    <main
      style={{
        maxWidth: 680,
        margin: '24px auto',
        padding: 16,
        background: 'var(--surface)',
        color: 'var(--body)',
      }}
    >
      <button onClick={() => setShown((value) => !value)}>
        {shown ? 'Close context' : 'Open context'}
      </button>
      {shown && (
        <WarningContext
          sessionId="synthetic-session"
          warning={{ ...warning, points_at: { record_uuid: 'target', block } }}
          timezone="UTC"
        />
      )}
    </main>
  );
}
createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    <DataProvider source={source}>
      <Sample />
    </DataProvider>
  </ThemeProvider>,
);
