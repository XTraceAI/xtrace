import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import type { DataSource, LeaderboardControls } from '../src/data/DataSource';
import { loadFixtureDataSource } from '../src/data/FixtureDataSource';
import type { LeaderboardStatus } from '../src/data/generated/LeaderboardStatus';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The F1 app on /leaderboard with synthetic leaderboard controls: `?joined=1`
 * starts as a member, anything else as not joined. The nickname `taken` is
 * refused as the leaderboard refuses a taken one. Nothing is saved or sent.
 */
const params = new URLSearchParams(location.search);
const notJoined: LeaderboardStatus = {
  available: true,
  unavailable_reason: null,
  joined: false,
  nickname: null,
  last_sent: null,
  standing: null,
  page_url: 'https://leaderboard.xtrace.ai/',
  error: null,
  notice: null,
};
const member = (nickname: string): LeaderboardStatus => ({
  ...notJoined,
  joined: true,
  nickname,
  last_sent: { leverage_tenths: 123, at_ms: Date.UTC(2026, 9, 27, 14, 30) },
  standing: { state: 'ranked', rank: 2 },
});
let status = params.get('joined') === '1' ? member('ada') : notJoined;
const refuseTaken = async (nickname: string) => {
  if (nickname === 'taken')
    throw { message: 'That nickname is taken. Try another one.', field: 'nickname' };
};
const leaderboard: LeaderboardControls = {
  read: async () => status,
  join: async (nickname) => {
    await refuseTaken(nickname);
    status = member(nickname);
    return status;
  },
  rename: async (nickname) => {
    await refuseTaken(nickname);
    status = { ...status, nickname };
    return status;
  },
  leave: async () => {
    status = notJoined;
    return status;
  },
  openPage: async () => {
    (window as { pageOpened?: number }).pageOpened =
      ((window as { pageOpened?: number }).pageOpened ?? 0) + 1;
  },
};

const root = document.getElementById('root');
if (!root) throw new Error('The fixture root is missing.');
void loadFixtureDataSource('F1').then((fixture) => {
  const source = new Proxy(fixture, {
    get: (target, key) => (key === 'leaderboard' ? leaderboard : Reflect.get(target, key)),
  }) as DataSource;
  createRoot(root).render(
    <StrictMode>
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter initialEntries={['/leaderboard']}>
            <AppRoutes />
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>
    </StrictMode>,
  );
});
