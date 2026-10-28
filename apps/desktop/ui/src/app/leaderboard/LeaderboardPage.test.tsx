import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { LeaderboardControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { LeaderboardStatus } from '../../data/generated/LeaderboardStatus';
import { events } from '../../data/ipc-names';
import { clock } from '../../kit/clock';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import { JOIN_TEXT, LEAVE_BACKUPS, LEAVE_TEXT } from './leaderboard';

const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const SENT_AT = 1_800_000_000_000;
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
const CODE = '123456';
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
    join: vi.fn<LeaderboardControls['join']>(async (nickname, email) => {
      status = { ...notJoined, code_sent: { nickname, email, choose_nickname: false } };
      return status;
    }),
    confirm: vi.fn<LeaderboardControls['confirm']>(async (code, nickname) => {
      if (code !== CODE)
        throw {
          message: 'That code is not right. Check the email and try again.',
          field: 'code',
        };
      status = { ...joined, nickname: nickname ?? status.code_sent?.nickname ?? 'ada' };
      return status;
    }),
    cancelCode: vi.fn<LeaderboardControls['cancelCode']>(async () => {
      status = notJoined;
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

async function mount(leaderboard?: LeaderboardControls, source = new FixtureDataSource(exported)) {
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
  // The code step names the email and takes the focus in its field.
  expect((await screen.findByTestId('leaderboard-code-sent')).textContent).toBe(
    'We sent a 6-digit code to ada@example.com. Enter it below. It expires in 15 minutes.',
  );
  const code = screen.getByLabelText('Code');
  expect(document.activeElement).toBe(code);
  expect(code.getAttribute('inputmode')).toBe('numeric');
  expect(code.getAttribute('autocomplete')).toBe('one-time-code');
  fireEvent.change(code, { target: { value: CODE } });
  fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
  await waitFor(() => expect(leaderboard.confirm).toHaveBeenCalledWith(CODE, undefined));
  expect(await screen.findByText('You are on the leaderboard')).toBeTruthy();
  expect(summary()['Nickname']).toBe(' ada ');
  // The form is gone, so focus moves to the page heading.
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1 })),
  );
});

it('shows the leaderboard’s refusal under the form and keeps what was typed', async () => {
  const leaderboard = controls(notJoined);
  leaderboard.join.mockRejectedValue({
    message: 'That nickname is taken. Try another one.',
    field: 'nickname',
  });
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
  // Only the field the refusal names is marked.
  expect(nickname.getAttribute('aria-invalid')).toBe('true');
  expect(screen.getByLabelText('Email').getAttribute('aria-invalid')).toBeNull();
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
  leaderboard.rename.mockRejectedValueOnce({
    message: 'That nickname is taken. Try another one.',
    field: 'nickname',
  });
  fireEvent.change(input, { target: { value: 'bob' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save nickname' }));
  expect((await screen.findByRole('alert')).textContent).toBe(
    'That nickname is taken. Try another one.',
  );
  expect(summary()['Nickname']).toBe('ada');
  expect(input.getAttribute('aria-invalid')).toBe('true');
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
  expect(within(dialog).getByText(LEAVE_BACKUPS)).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(leaderboard.leave).not.toHaveBeenCalled();
  // A refusal stays in the dialog; nothing changes.
  leaderboard.leave.mockRejectedValueOnce({
    message:
      'Could not reach the leaderboard. Check your internet connection and try again. You are still on the leaderboard.',
    field: null,
  });
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
  await mount(
    controls({
      ...notJoined,
      available: false,
      unavailable_reason: 'This copy of XTrace does not send to the leaderboard.',
    }),
  );
  expect((await screen.findByTestId('leaderboard-unavailable')).textContent).toBe(
    'This copy of XTrace does not send to the leaderboard.',
  );
  expect(screen.queryByRole('button', { name: 'Join' })).toBeNull();
});

it('reads the status again after a background upload', async () => {
  const leaderboard = controls(joined);
  const source = new FixtureDataSource(exported);
  await mount(leaderboard, source);
  await screen.findByText('You are on the leaderboard');
  leaderboard.read.mockResolvedValue({
    ...joined,
    last_sent: { leverage_tenths: 150, at_ms: SENT_AT },
  });
  const before = leaderboard.read.mock.calls.length;
  act(() => source.emit(events.leaderboardChanged));
  await waitFor(() => expect(summary()['Leverage, 7 days']).toBe('15×'));
  expect(leaderboard.read.mock.calls.length).toBeGreaterThan(before);
});

it('marks a wrong code, sends a new one, and goes back keeping the nickname', async () => {
  const leaderboard = controls(notJoined);
  await mount(leaderboard);
  fireEvent.change(await screen.findByLabelText('Nickname'), { target: { value: 'ada' } });
  fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'ada@exmaple.com' } });
  fireEvent.click(screen.getByRole('button', { name: 'Join' }));
  const code = await screen.findByLabelText('Code');
  fireEvent.change(code, { target: { value: '000000' } });
  fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe('That code is not right. Check the email and try again.');
  expect(code.getAttribute('aria-invalid')).toBe('true');
  // The field names where the code went, then the refusal.
  const sentLine = screen.getByTestId('leaderboard-code-sent');
  expect(code.getAttribute('aria-describedby')).toBe(`${sentLine.id} ${alert.id}`);
  // A new code goes to the same nickname and email.
  fireEvent.click(screen.getByRole('button', { name: 'Send a new code' }));
  await waitFor(() => expect(leaderboard.join).toHaveBeenCalledTimes(2));
  expect(leaderboard.join).toHaveBeenLastCalledWith('ada', 'ada@exmaple.com');
  expect(await screen.findByText('A new code was sent. Only the newest code works.')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  // A typo in the email: back to the form, nickname kept, email to change.
  fireEvent.click(screen.getByRole('button', { name: 'Use a different email' }));
  const email = (await screen.findByLabelText('Email')) as HTMLInputElement;
  expect(leaderboard.cancelCode).toHaveBeenCalledTimes(1);
  expect((screen.getByLabelText('Nickname') as HTMLInputElement).value).toBe('ada');
  expect(document.activeElement).toBe(email);
  expect(screen.queryByLabelText('Code')).toBeNull();
});

it('shows the code step the app still holds, and asks for another nickname with the same code', async () => {
  const leaderboard = controls({
    ...notJoined,
    code_sent: { nickname: 'ada', email: 'ada@example.com', choose_nickname: false },
  });
  leaderboard.confirm.mockRejectedValueOnce({
    message: 'That nickname is taken. Choose another one and confirm again.',
    field: 'nickname',
  });
  await mount(leaderboard);
  const code = await screen.findByLabelText('Code');
  // Only the code is asked for until the leaderboard says otherwise.
  expect(screen.queryByLabelText('Nickname')).toBeNull();
  fireEvent.change(code, { target: { value: CODE } });
  fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
  await waitFor(() => expect(leaderboard.confirm).toHaveBeenCalledWith(CODE, undefined));
  const nickname = (await screen.findByLabelText('Nickname')) as HTMLInputElement;
  expect(nickname.value).toBe('ada');
  expect(nickname.getAttribute('aria-invalid')).toBe('true');
  expect(document.activeElement).toBe(nickname);
  expect(screen.getByRole('alert').textContent).toBe(
    'That nickname is taken. Choose another one and confirm again.',
  );
  // The code stays; the same code goes again with the new nickname.
  expect((screen.getByLabelText('Code') as HTMLInputElement).value).toBe(CODE);
  fireEvent.change(nickname, { target: { value: 'ada_2' } });
  fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
  await waitFor(() => expect(leaderboard.confirm).toHaveBeenLastCalledWith(CODE, 'ada_2'));
  expect(await screen.findByText('You are on the leaderboard')).toBeTruthy();
  expect(summary()['Nickname']).toBe('ada_2');
});

it('asks for the nickname again when the app already knows it was taken', async () => {
  await mount(
    controls({
      ...notJoined,
      code_sent: { nickname: 'ada', email: 'ada@example.com', choose_nickname: true },
    }),
  );
  const nickname = await screen.findByLabelText('Nickname');
  expect(nickname.getAttribute('aria-invalid')).toBe('true');
});

it('welcomes someone signing back in to an existing entry once', async () => {
  const leaderboard = controls({
    ...notJoined,
    code_sent: { nickname: 'new_name', email: 'ada@example.com', choose_nickname: false },
  });
  leaderboard.confirm.mockResolvedValueOnce({
    ...joined,
    nickname: 'ada',
    welcome: 'Welcome back. You are signed in to your existing entry as ada.',
  });
  const source = new FixtureDataSource(exported);
  await mount(leaderboard, source);
  fireEvent.change(await screen.findByLabelText('Code'), { target: { value: CODE } });
  fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
  expect((await screen.findByTestId('leaderboard-welcome')).textContent).toBe(
    'Welcome back. You are signed in to your existing entry as ada.',
  );
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1 })),
  );
  // A later status read, which never carries it, leaves it on the page.
  leaderboard.read.mockResolvedValue({ ...joined, nickname: 'ada', last_sent: null });
  const before = leaderboard.read.mock.calls.length;
  act(() => source.emit(events.leaderboardChanged));
  await waitFor(() => expect(leaderboard.read.mock.calls.length).toBeGreaterThan(before));
  await waitFor(() => expect(summary()['Last sent']).toBe('Not sent yet'));
  expect(screen.getByTestId('leaderboard-welcome')).toBeTruthy();
});

it('sends a new code with the nickname being chosen', async () => {
  const leaderboard = controls({
    ...notJoined,
    code_sent: { nickname: 'ada', email: 'ada@example.com', choose_nickname: true },
  });
  await mount(leaderboard);
  fireEvent.change(await screen.findByLabelText('Nickname'), { target: { value: 'ada_3' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send a new code' }));
  await waitFor(() => expect(leaderboard.join).toHaveBeenCalledWith('ada_3', 'ada@example.com'));
});
