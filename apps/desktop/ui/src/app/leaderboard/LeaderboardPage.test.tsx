import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { LeaderboardControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { LeaderboardStatus } from '../../data/generated/LeaderboardStatus';
import { clock } from '../../kit/clock';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import { JOIN_TEXT, LEAVE_TEXT } from './leaderboard';

const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const SENT_AT = 1_800_000_000_000;
const notJoined: LeaderboardStatus = {
  available: true,
  joined: false,
  nickname: null,
  last_sent: null,
  standing: null,
  page_url: 'https://leaderboard.xtrace.ai/',
  error: null,
  notice: null,
};
const joined: LeaderboardStatus = {
  ...notJoined,
  joined: true,
  nickname: 'ada',
  last_sent: { leverage_tenths: 123, at_ms: SENT_AT },
  standing: { state: 'ranked', rank: 2 },
};

/** A synthetic leaderboard: the status lives here, never on a server. */
function controls(initial: LeaderboardStatus) {
  let status = initial;
  const answer = async () => status;
  return {
    read: vi.fn<LeaderboardControls['read']>(answer),
    join: vi.fn<LeaderboardControls['join']>(async (nickname) => {
      status = { ...joined, nickname };
      return status;
    }),
    rename: vi.fn<LeaderboardControls['rename']>(async (nickname) => {
      status = { ...status, nickname };
      return status;
    }),
    leave: vi.fn<LeaderboardControls['leave']>(async () => {
      status = notJoined;
      return status;
    }),
    openPage: vi.fn<LeaderboardControls['openPage']>(async () => {}),
  } satisfies LeaderboardControls;
}

async function mount(leaderboard?: LeaderboardControls) {
  const source = new FixtureDataSource(exported);
  if (leaderboard) Object.defineProperty(source, 'leaderboard', { value: leaderboard });
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={['/leaderboard']}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await waitFor(() => expect(screen.queryByText('Connecting live updates…')).toBeNull());
}

const page = () => document.querySelector<HTMLElement>('.xt-leaderboard')!;
const summary = () =>
  Object.fromEntries(
    [...screen.getByTestId('leaderboard-summary').querySelectorAll('dt')].map((dt) => [
      dt.textContent,
      dt.nextElementSibling?.textContent,
    ]),
  );

it('explains what joining sends and joins with exactly what was typed', async () => {
  const leaderboard = controls(notJoined);
  await mount(leaderboard);
  expect(await screen.findByText(JOIN_TEXT)).toBeTruthy();
  expect(screen.getByRole('heading', { level: 1, name: 'Leaderboard' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Leaderboard' }).getAttribute('aria-current')).toBe(
    'page',
  );
  fireEvent.change(screen.getByLabelText('Nickname'), { target: { value: ' ada ' } });
  fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'ada@example.com' } });
  fireEvent.click(screen.getByRole('button', { name: 'Join' }));
  // The leaderboard checks the values; the app sends them as typed.
  await waitFor(() => expect(leaderboard.join).toHaveBeenCalledWith(' ada ', 'ada@example.com'));
  expect(await screen.findByText('You are on the leaderboard')).toBeTruthy();
  expect(summary()['Nickname']).toBe(' ada ');
});

it('shows the leaderboard’s refusal under the form and keeps what was typed', async () => {
  const leaderboard = controls(notJoined);
  leaderboard.join.mockRejectedValue('That nickname is taken. Try another one.');
  await mount(leaderboard);
  const nickname = await screen.findByLabelText('Nickname');
  fireEvent.change(nickname, { target: { value: 'ada' } });
  // An empty email is not refused by the app: the leaderboard decides.
  fireEvent.click(screen.getByRole('button', { name: 'Join' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe('That nickname is taken. Try another one.');
  expect(leaderboard.join).toHaveBeenCalledWith('ada', '');
  expect((nickname as HTMLInputElement).value).toBe('ada');
  expect(nickname.getAttribute('aria-describedby')).toBe(alert.id);
  expect(screen.queryByText('You are on the leaderboard')).toBeNull();
});

it('says once that the app forgot a membership the leaderboard no longer has', async () => {
  await mount(
    controls({
      ...notJoined,
      notice: 'The leaderboard no longer has your membership, so this app is not joined any more.',
    }),
  );
  expect((await screen.findByTestId('leaderboard-notice')).textContent).toBe(
    'The leaderboard no longer has your membership, so this app is not joined any more.',
  );
  expect(screen.getByRole('button', { name: 'Join' })).toBeTruthy();
});

it('shows the nickname, the number as the Dashboard writes it, the rank and when it was sent', async () => {
  const leaderboard = controls(joined);
  await mount(leaderboard);
  await screen.findByText('You are on the leaderboard');
  expect(summary()).toEqual({
    Nickname: 'ada',
    'Leverage, 7 days': '12.3×',
    Rank: '#2 on the public page',
    'Last sent': clock(SENT_AT, { date: 'day' }),
  });
  fireEvent.click(screen.getByRole('button', { name: 'Open leaderboard.xtrace.ai' }));
  expect(leaderboard.openPage).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('alert')).toBeNull();
});

it('says why a member is not listed and shows an upload error', async () => {
  await mount(
    controls({
      ...joined,
      last_sent: { leverage_tenths: null, at_ms: SENT_AT },
      standing: {
        state: 'not_listed',
        reason: 'Not listed yet: there is no leverage number for your last 7 days.',
      },
      error: 'Could not reach the leaderboard. Your number will be sent again within the hour.',
    }),
  );
  await screen.findByText('You are on the leaderboard');
  expect(summary()['Leverage, 7 days']).toBe('No 7-day number');
  expect(summary()['Rank']).toBe(
    'Not listed yet: there is no leverage number for your last 7 days.',
  );
  expect(screen.getByRole('alert').textContent).toBe(
    'Could not reach the leaderboard. Your number will be sent again within the hour.',
  );
});

it('changes the nickname, and shows a refusal without changing it', async () => {
  const leaderboard = controls(joined);
  await mount(leaderboard);
  fireEvent.click(await screen.findByRole('button', { name: 'Change nickname' }));
  const input = screen.getByLabelText('New nickname') as HTMLInputElement;
  expect(input.value).toBe('ada');
  expect(document.activeElement).toBe(input);
  leaderboard.rename.mockRejectedValueOnce('That nickname is taken. Try another one.');
  fireEvent.change(input, { target: { value: 'bob' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save nickname' }));
  expect((await screen.findByRole('alert')).textContent).toBe(
    'That nickname is taken. Try another one.',
  );
  expect(summary()['Nickname']).toBe('ada');
  fireEvent.change(input, { target: { value: 'Ada_2' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save nickname' }));
  await waitFor(() => expect(summary()['Nickname']).toBe('Ada_2'));
  expect(leaderboard.rename).toHaveBeenLastCalledWith('Ada_2');
  expect(screen.queryByLabelText('New nickname')).toBeNull();
  // Cancel closes the form without asking anything.
  fireEvent.click(screen.getByRole('button', { name: 'Change nickname' }));
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(screen.queryByLabelText('New nickname')).toBeNull();
  expect(leaderboard.rename).toHaveBeenCalledTimes(2);
});

it('asks before leaving, and leaves only on confirmation', async () => {
  const leaderboard = controls(joined);
  await mount(leaderboard);
  const leaveButton = await screen.findByRole('button', { name: 'Leave leaderboard' });
  fireEvent.click(leaveButton);
  let dialog = await screen.findByRole('dialog', { name: 'Leave the leaderboard?' });
  expect(within(dialog).getByText(LEAVE_TEXT)).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(leaderboard.leave).not.toHaveBeenCalled();
  // A refusal stays in the dialog; nothing changes.
  leaderboard.leave.mockRejectedValueOnce(
    'Could not reach the leaderboard. Check your internet connection and try again. You are still on the leaderboard.',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Leave leaderboard' }));
  dialog = await screen.findByRole('dialog', { name: 'Leave the leaderboard?' });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Leave leaderboard' }));
  expect((await within(dialog).findByRole('alert')).textContent).toContain(
    'You are still on the leaderboard.',
  );
  expect(screen.getByText('You are on the leaderboard')).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Leave leaderboard' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(leaderboard.leave).toHaveBeenCalledTimes(2);
  expect(await screen.findByRole('button', { name: 'Join' })).toBeTruthy();
  await act(async () => {});
});

it('joins nothing in the preview or in a copy that shows test data', async () => {
  await mount();
  expect(within(page()).getByText(JOIN_TEXT)).toBeTruthy();
  expect(within(page()).getByText(/This preview sends nothing/)).toBeTruthy();
  expect(within(page()).queryByRole('button', { name: 'Join' })).toBeNull();
  cleanup();
  await mount(controls({ ...notJoined, available: false }));
  expect(await screen.findByTestId('leaderboard-unavailable')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Join' })).toBeNull();
});
