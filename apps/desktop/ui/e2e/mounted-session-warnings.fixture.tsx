import { createRoot } from 'react-dom/client';
import { MemoryRouter, useLocation } from 'react-router';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import { filters, warningSource } from './mounted-session-warnings.data';
import '../src/index.css';

const path = new URLSearchParams(location.search).get('path') ?? `/sessions?${filters}`;
function Location() {
  const location = useLocation();
  return (
    <output data-testid="fixture-location" style={{ position: 'fixed', bottom: 0, fontSize: 8 }}>
      {location.pathname + location.search}
    </output>
  );
}
createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    <DataProvider source={warningSource()}>
      <MemoryRouter initialEntries={[path]}>
        <AppRoutes />
        <Location />
      </MemoryRouter>
    </DataProvider>
  </ThemeProvider>,
);
