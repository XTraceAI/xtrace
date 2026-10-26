import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={new FixtureDataSource(fixture as FixtureExport)}>
        <HashRouter>
          <AppRoutes />
        </HashRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
