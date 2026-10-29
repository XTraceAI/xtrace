import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, useLocation } from 'react-router';
import fixture from '../fixtures/F1.json';
import { DataProvider } from '../src/data/DataProvider';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import {
  SessionWarningMarker,
  SessionWarningsDetail,
  WarningDot,
  useSessionWarning,
  warningHref,
} from '../src/app/session-warnings';
import { answer, dismissedAnswer, warning } from './session-warnings.data';
import '../src/index.css';
import '../src/styles/metrics.css';

// This isolated page uses supplied answers. It never opens the app or native data.
let current = answer;
const source = new FixtureDataSource(fixture as FixtureExport);
source.sessionWarnings = async () => ({
  timezone: 'UTC',
  sessions: [{ session_id: 'synthetic-session', answer: current }],
});
Object.defineProperty(source, 'warningDismissals', {
  value: {
    dismiss: async (session_id: string, warning_id: string) => {
      current = dismissedAnswer;
      return { session_id, warning_id, dismissed: true };
    },
    restore: async (session_id: string, warning_id: string) => {
      current = answer;
      return { session_id, warning_id, dismissed: false };
    },
  },
});

function Samples() {
  const query = useSessionWarning('synthetic-session', 'UTC');
  const [opened, setOpened] = useState(0);
  const [highlight, setHighlight] = useState<string | null>(null);
  const location = useLocation();
  return (
    <main
      style={{
        maxWidth: 680,
        margin: 32,
        padding: 24,
        background: 'var(--surface)',
        color: 'var(--body)',
      }}
    >
      <h1>Shared warnings</h1>
      <p>Synthetic component samples</p>
      <div style={{ display: 'flex', gap: 16, alignItems: 'center', marginBlock: 24 }}>
        <SessionWarningMarker answer={query.data} to={warningHref('synthetic-session', '')} />
        <span style={{ position: 'relative', width: 180, height: 28, background: 'var(--track)' }}>
          <WarningDot
            sessionId="synthetic-session"
            warning={warning}
            left={0.25}
            to={warningHref('synthetic-session', warning.id)}
          />
          <WarningDot
            sessionId="synthetic-session"
            warning={{ ...warning, name: 'Open synthetic warning detail' }}
            left={0.75}
            onOpen={() => setOpened((n) => n + 1)}
          />
        </span>
      </div>
      <output aria-label="Opened count">{opened}</output>
      <output aria-label="Current location">{location.pathname + location.search}</output>
      <p>
        <button onClick={() => setHighlight(warning.id)}>Select supplied warning</button>
      </p>
      <SessionWarningsDetail
        sessionId="synthetic-session"
        answer={query.data}
        pending={query.isPending}
        failed={query.isError}
        highlight={highlight}
      />
    </main>
  );
}

createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    <DataProvider source={source}>
      <MemoryRouter>
        <Samples />
      </MemoryRouter>
    </DataProvider>
  </ThemeProvider>,
);
