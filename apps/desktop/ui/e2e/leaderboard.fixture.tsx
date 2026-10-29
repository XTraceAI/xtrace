import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import type { DataSource, LeaderboardControls } from '../src/data/DataSource';
import { loadFixtureDataSource } from '../src/data/FixtureDataSource';
import type { LeaderboardStatus } from '../src/data/generated/LeaderboardStatus';
import type { LeaderboardTable } from '../src/data/generated/LeaderboardTable';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The F1 app on /leaderboard with synthetic leaderboard controls: `?joined=1`
 * starts as a member, anything else as not joined. An email without `@` is
 * refused, the emailed code is 123456, and the nickname `taken` is refused
 * after a right code, as the leaderboard does. While joined, the public list
 * is a few synthetic members with the member second (`?board=empty` lists no
 * one, `?board=error` cannot be read). Nothing is saved or sent.
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
  code_sent: null,
  welcome: null,
};
const member = (nickname: string): LeaderboardStatus => ({
  ...notJoined,
  joined: true,
  nickname,
  last_sent: { leverage_tenths: 123, at_ms: Date.UTC(2026, 9, 27, 14, 30) },
  standing: { state: 'ranked', rank: 2 },
});
let status = params.get('joined') === '1' ? member('ada') : notJoined;
const others = [
  { rank: 1, nickname: 'grace', leverage_tenths: 412, country: 'US' },
  { rank: 3, nickname: 'linus', leverage_tenths: 90, country: 'FI' },
  { rank: 3, nickname: 'margaret', leverage_tenths: 90, country: null },
  { rank: 5, nickname: 'barbara', leverage_tenths: 41, country: 'DE' },
];
/** The list as the native side answers it: the member's row marked by nickname. */
const board = (): LeaderboardTable => {
  if (!status.joined)
    return {
      state: 'unavailable',
      rows: [],
      reason: 'Join the leaderboard to see the public list here.',
      refresh_after_seconds: null,
    };
  if (params.get('board') === 'error')
    return {
      state: 'error',
      rows: [],
      reason: 'Could not reach the leaderboard. Check your internet connection and try again.',
      refresh_after_seconds: null,
    };
  if (params.get('board') === 'empty')
    return { state: 'ok', rows: [], reason: null, refresh_after_seconds: null };
  const you = { rank: 2, nickname: status.nickname!, leverage_tenths: 123, country: 'GB' };
  const rows = [others[0], you, ...others.slice(1)].map((row) => ({
    ...row,
    is_you: row.nickname.toLowerCase() === status.nickname!.toLowerCase(),
  }));
  return { state: 'ok', rows, reason: null, refresh_after_seconds: null };
};
const tableReads = () => (window as { tableReads?: number }).tableReads ?? 0;
const refuseTaken = async (nickname: string) => {
  if (nickname === 'taken')
    throw { message: 'That nickname is taken. Try another one.', field: 'nickname' };
};
const refuseBadEmail = async (email: string) => {
  if (!email.includes('@'))
    throw { message: 'That email address does not look right.', field: 'email' };
};
const leaderboard: LeaderboardControls = {
  read: async () => status,
  table: async () => {
    (window as { tableReads?: number }).tableReads = tableReads() + 1;
    return board();
  },
  // As the leaderboard does, a taken nickname is only said after a right code.
  join: async (nickname, email) => {
    await refuseBadEmail(email);
    status = { ...notJoined, code_sent: { nickname, email, choose_nickname: false } };
    return status;
  },
  // The synthetic code is 123456; it stays valid after a taken nickname.
  confirm: async (code, nickname) => {
    if (code !== '123456')
      throw { message: 'That code is not right. Check the email and try again.', field: 'code' };
    const sent = status.code_sent!;
    const wanted = nickname ?? sent.nickname;
    if (wanted === 'taken') {
      status = { ...status, code_sent: { ...sent, nickname: wanted, choose_nickname: true } };
      throw {
        message: 'That nickname is taken. Choose another one and confirm again.',
        field: 'nickname',
      };
    }
    status = member(wanted);
    return status;
  },
  cancelCode: async () => {
    status = notJoined;
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
