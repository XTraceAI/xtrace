import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
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
  if (params.has('long'))
    status.records[2]!.blocks = [
      { kind: 'text', index: 0, text: 'Synthetic long content. '.repeat(500) },
    ];
  return contextExcerpt(status, warning.points_at.record_uuid, warning.points_at.block);
};
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
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
