import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import synthetic from '../src/app/session-diagnosis-warnings.synthetic.json';
import diagnosisSynthetic from '../src/app/session-diagnosis.synthetic.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DiagnoseButton, DiagnosisPanel } from '../src/app/session-diagnosis';
import { useDiagnose } from '../src/app/useSessionDiagnosis';
import { SessionWarningsDetail, useSessionWarning } from '../src/app/session-warnings';
import { DataProvider } from '../src/data/DataProvider';
import type { DiagnosisControls } from '../src/data/DataSource';
import type { SessionDiagnosis } from '../src/data/generated/SessionDiagnosis';
import type { SavedSkill } from '../src/data/generated/SavedSkill';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

// Explicit synthetic browser controls only: no native command, model, or file write.
const params = new URLSearchParams(location.search);
const path = params.get('path') ?? '/sessions/s-warned';
const served = { ...structuredClone(fixture), ...structuredClone(synthetic) } as FixtureExport;
const source = new FixtureDataSource(served);
// Name the synthetic warning session without importing unrelated dashboard exports.
source.sessionRow = async (id) => ({ ...served.sessions[0]!.rows[0]!, id });
const calls = { runs: [] as string[][], stops: [] as string[], saves: [] as string[][] };
let waiting: ((answer: SessionDiagnosis) => void) | undefined;
const diagnoses = diagnosisSynthetic.diagnoses as unknown as Record<string, SessionDiagnosis>;
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
    waiting?.(diagnoses.cancelled!);
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
      waiting?.(diagnoses[key]!);
      waiting = undefined;
    },
  },
});

function LocalDiagnosis() {
  const warnings = useSessionWarning('s-warned', 'UTC');
  const active = warnings.data?.state === 'read' ? warnings.data.warnings : undefined;
  const diagnose = useDiagnose('s-warned', 'UTC', active);
  return (
    <section
      data-testid="local-diagnosis"
      style={{ width: 390, maxWidth: '100%', margin: '16px auto', padding: 16 }}
    >
      <SessionWarningsDetail
        sessionId="s-warned"
        answer={warnings.data}
        pending={warnings.isPending}
        failed={warnings.isError}
        actions={
          diagnose
            ? (warning) => <DiagnoseButton diagnose={diagnose} warning={warning} />
            : undefined
        }
        below={
          diagnose
            ? (warning) => <DiagnosisPanel diagnose={diagnose} warning={warning} />
            : undefined
        }
      />
    </section>
  );
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          {params.has('diagnosislocal') ? <LocalDiagnosis /> : <AppRoutes />}
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
