import { MemoryRouter, useLocation } from 'react-router';
import { contextFixture, contextExcerpt } from './warning-context.synthetic';
import type { SessionExcerptStatus } from '../../data/generated/SessionExcerptStatus';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { useRef, useState } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DiagnosisControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { SavedSkill } from '../../data/generated/SavedSkill';
import type { SessionDiagnosis } from '../../data/generated/SessionDiagnosis';
import type { SessionWarningsAnswer } from '../../data/generated/SessionWarningsAnswer';
import type { SessionWarningsList } from '../../data/generated/SessionWarningsList';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { queryKeys } from '../../data/query-client';
import { ThemeProvider } from '../../theme/ThemeProvider';
import diagnosisSynthetic from '../session-diagnosis.synthetic.json';
import synthetic from '../session-warnings.synthetic.json';
import { WarningDialog, type SelectedWarning } from './WarningDialog';
import { Modal } from '../../kit/Modal';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(cleanup);
const words = SESSION_WARNINGS_TEXT.diagnose;
const answers = diagnosisSynthetic.diagnoses as unknown as Record<string, SessionDiagnosis>;
const list = structuredClone(synthetic.session_warnings) as SessionWarningsList;
const read = list.sessions.find((one) => one.session_id === 's-warned')!.answer as Extract<
  SessionWarningsAnswer,
  { state: 'read' }
>;
const selected: SelectedWarning = {
  warning: read.warnings.find((one) => one.id === synthetic.after_dismissal.warning_id)!,
  sessionId: 's-warned',
  name: 'Synthetic selected conversation',
  timezone: 'Asia/Tokyo',
};

function setup() {
  const source = new FixtureDataSource({ ...fixture, ...synthetic } as unknown as FixtureExport);
  const transcript = vi
    .spyOn(source, 'sessionWarningContext')
    .mockResolvedValue(
      contextExcerpt(
        contextFixture(selected.warning.points_at.record_uuid),
        selected.warning.points_at.record_uuid,
      ),
    );
  const cancelTranscript = vi.spyOn(source, 'cancelSessionTranscript');
  const pending: ((answer: SessionDiagnosis) => void)[] = [];
  const controls: DiagnosisControls = {
    diagnose: vi.fn(() => new Promise<SessionDiagnosis>((resolve) => pending.push(resolve))),
    cancel: vi.fn(async () => true),
    saveSkill: vi.fn(async () => diagnosisSynthetic.saved as SavedSkill),
  };
  Object.defineProperty(source, 'diagnosis', { value: controls, configurable: true });
  const warnings = vi
    .spyOn(source, 'sessionWarnings')
    .mockImplementation(async () => structuredClone(list));
  const onClose = vi.fn();
  let client: QueryClient;
  let select: (one: SelectedWarning | null) => void = () => {};
  let show: (shown: boolean) => void = () => {};
  function Harness() {
    const location = useLocation();
    client = useQueryClient();
    const [one, setOne] = useState<SelectedWarning | null>(selected);
    const [shown, setShown] = useState(true);
    select = setOne;
    show = setShown;
    const focus = useRef<HTMLButtonElement>(null);
    return (
      <>
        <output data-testid="current-location">
          {location.pathname}
          {location.search}
        </output>
        <button ref={focus}>Origin</button>
        {shown && (
          <WarningDialog
            selected={one}
            onClose={() => {
              onClose();
              setOne(null);
            }}
            returnFocusRef={focus}
          />
        )}
      </>
    );
  }
  const mount = () =>
    render(
      <MemoryRouter initialEntries={['/dashboard?range=30d']}>
        <ThemeProvider>
          <DataProvider source={source}>
            <Harness />
          </DataProvider>
        </ThemeProvider>
      </MemoryRouter>,
    );
  return {
    source,
    transcript,
    cancelTranscript,
    controls,
    pending,
    warnings,
    onClose,
    mount,
    select: (one: SelectedWarning | null) => act(() => select(one)),
    show: (shown: boolean) => act(() => show(shown)),
    client: () => client!,
    answer: (value: SessionDiagnosis, index = pending.length - 1) =>
      act(async () => pending[index]!(value)),
  };
}
const dialog = () => screen.getByTestId('dashboard-warning-modal');
const close = () => within(dialog()).getByRole('button', { name: 'Close' });
async function ask(test: ReturnType<typeof setup>) {
  test.mount();
  const button = await screen.findByRole('button', { name: /^Diagnose this warning:/ });
  fireEvent.click(button);
  return button;
}
async function run(test: ReturnType<typeof setup>) {
  await ask(test);
  fireEvent.click(screen.getByRole('button', { name: words.confirm }));
  await screen.findByTestId('diagnosis-running');
}

it('uses only the selected session, issue and report timezone; confirmation sends nothing', async () => {
  const test = setup();
  const button = await ask(test);
  expect(screen.getAllByTestId('warning-item')).toHaveLength(1);
  expect(screen.getByTestId('warning-item').dataset.warningId).toBe(selected.warning.id);
  expect(test.warnings).toHaveBeenCalledWith(['s-warned'], 'Asia/Tokyo');
  expect(test.controls.diagnose).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: words.not_now }));
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  fireEvent.click(button);
  fireEvent.click(close());
  expect(test.onClose).toHaveBeenCalledOnce();
  expect(test.controls.diagnose).not.toHaveBeenCalled();
  test.select(selected);
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
});

it('uses native dismissal readback and keeps Undo after its source dot disappears', async () => {
  const test = setup();
  let dismissed = false;
  test.warnings.mockImplementation(
    async () =>
      structuredClone(
        dismissed ? synthetic.after_dismissal.session_warnings : synthetic.session_warnings,
      ) as SessionWarningsList,
  );
  const dismiss = vi.fn(async () => {
    dismissed = true;
    return { session_id: selected.sessionId, warning_id: selected.warning.id, dismissed: true };
  });
  const restore = vi.fn(async () => {
    dismissed = false;
    return { session_id: selected.sessionId, warning_id: selected.warning.id, dismissed: false };
  });
  Object.defineProperty(test.source, 'warningDismissals', { value: { dismiss, restore } });
  test.mount();
  fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }));
  const undo = await screen.findByRole('button', { name: 'Undo' });
  await waitFor(() => expect(document.activeElement).toBe(undo));
  expect(test.onClose).not.toHaveBeenCalled();
  expect(dismiss).toHaveBeenCalledWith(selected.sessionId, selected.warning.id);
  fireEvent.click(undo);
  const button = await screen.findByRole('button', { name: 'Dismiss' });
  await waitFor(() => expect(document.activeElement).toBe(button));
  expect(restore).toHaveBeenCalledWith(selected.sessionId, selected.warning.id);
});

it('keeps a refused dismissal honest and retryable', async () => {
  const test = setup();
  const dismiss = vi.fn(async () => {
    throw new Error('refused');
  });
  Object.defineProperty(test.source, 'warningDismissals', { value: { dismiss } });
  test.mount();
  const button = await screen.findByRole('button', { name: 'Dismiss' });
  for (let count = 1; count <= 2; count++) {
    fireEvent.click(button);
    expect((await screen.findByRole('alert')).textContent).toBe(
      SESSION_WARNINGS_TEXT.dismiss_failed,
    );
    await waitFor(() => expect(button.getAttribute('aria-disabled')).toBeNull());
    expect(dismiss).toHaveBeenCalledTimes(count);
  }
});

// Closing the window no longer stops a running diagnosis: it belongs to the
// source runtime, and the warning's dot shows it running. Reopening the
// warning shows Stop, then the kept answer.
for (const from of ['Close', 'Escape'] as const)
  it(`${from} closes at once and leaves the diagnosis running for the reopened warning`, async () => {
    const test = setup();
    await run(test);
    expect(vi.mocked(test.controls.diagnose).mock.calls[0]!.slice(0, 3)).toEqual([
      's-warned',
      selected.warning.id,
      selected.timezone,
    ]);
    if (from === 'Close') fireEvent.click(close());
    else fireEvent.keyDown(screen.getByRole('button', { name: words.stop }), { key: 'Escape' });
    await waitFor(() => expect(test.onClose).toHaveBeenCalledOnce());
    expect(test.controls.cancel).not.toHaveBeenCalled();
    test.select(selected);
    expect(await screen.findByRole('button', { name: words.stop })).toBeTruthy();
    // Focus stays with the reopened window, not on Stop.
    expect(document.activeElement).not.toBe(screen.getByRole('button', { name: words.stop }));
    await test.answer(answers.make_skill!);
    expect(await screen.findByTestId('diagnosis-result')).toBeTruthy();
    expect(test.controls.cancel).not.toHaveBeenCalled();
  });

it('keeps Stop retryable after refused and failed cancellation', async () => {
  const test = setup();
  vi.mocked(test.controls.cancel)
    .mockResolvedValueOnce(false)
    .mockRejectedValueOnce(new Error('IPC'));
  await run(test);
  const stop = () => screen.getByRole('button', { name: words.stop });
  for (let count = 1; count <= 3; count++) {
    fireEvent.click(stop());
    await waitFor(() => expect(test.controls.cancel).toHaveBeenCalledTimes(count));
    if (count < 3) await waitFor(() => expect(stop().getAttribute('aria-disabled')).toBeNull());
  }
  await test.answer(answers.cancelled!);
  expect(test.onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId('diagnosis-result').dataset.state).toBe('cancelled');
});

for (const replace of [false, true])
  it(`keeps its run on ${replace ? 'selection replacement' : 'view disappearance'} and keeps the answer for its warning`, async () => {
    const test = setup();
    await run(test);
    const other = read.warnings.find((one) => one.id !== selected.warning.id)!;
    if (replace) {
      test.select({ ...selected, warning: other });
      // One runs at a time: the other warning waits, and says why.
      expect(await screen.findByTestId('diagnosis-busy')).toBeTruthy();
      expect(
        screen
          .getByRole('button', { name: /^Diagnose this warning:/ })
          .getAttribute('aria-disabled'),
      ).toBe('true');
      // The same sentence and Stop as a session page shows for another
      // warning's run.
      expect(screen.getByTestId('diagnosis-busy').textContent).toContain(words.busy);
      expect(screen.getByRole('button', { name: words.stop })).toBeTruthy();
    } else test.show(false);
    expect(test.controls.cancel).not.toHaveBeenCalled();
    await test.answer(answers.make_skill!);
    expect(
      test.client().getQueryData(['session-diagnosis', selected.sessionId, selected.warning.id]),
    ).toMatchObject({ answer: answers.make_skill });
    if (!replace) test.show(true);
    test.select(selected);
    expect(await screen.findByTestId('diagnosis-result')).toBeTruthy();
    expect(test.controls.saveSkill).not.toHaveBeenCalled();
  });

for (const replace of [false, true])
  it(`drops confirmation without sending on ${replace ? 'selection replacement' : 'view disappearance'}`, async () => {
    const test = setup();
    await ask(test);
    if (replace) test.select({ ...selected, warning: read.warnings[0]! });
    else test.show(false);
    expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
    expect(test.controls.diagnose).not.toHaveBeenCalled();
    expect(test.controls.cancel).not.toHaveBeenCalled();
    test.select(selected);
    test.show(true);
    expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  });

it('shows a fake result and saves its full draft only after the explicit Save click', async () => {
  const test = setup();
  await run(test);
  await test.answer(answers.make_skill!);
  const made = diagnosisSynthetic.diagnoses.make_skill;
  expect(screen.getByTestId('diagnosis-draft').textContent).toBe(made.draft);
  expect(test.controls.saveSkill).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: words.save_skill }));
  await screen.findByTestId('diagnosis-saved');
  expect(test.controls.saveSkill).toHaveBeenCalledWith(
    selected.sessionId,
    made.skill_name,
    made.draft,
  );
});

for (const missing of [false, true])
  it(`offers no action when ${missing ? 'the session is missing' : 'the selected warning is stale'}`, async () => {
    const test = setup();
    test.warnings.mockResolvedValue({
      timezone: selected.timezone,
      sessions: [
        {
          session_id: selected.sessionId,
          answer: missing ? { state: 'missing' } : { ...read, warnings: [], dismissed: [] },
        },
      ],
    });
    test.mount();
    await waitFor(() =>
      expect(screen.getByTestId('warnings-state').textContent).toBe(
        missing ? SESSION_WARNINGS_TEXT.session_missing : SESSION_WARNINGS_TEXT.session_unavailable,
      ),
    );
    expect(screen.queryByRole('button', { name: 'Dismiss' })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
    expect(dialog().textContent).toContain(selected.warning.reason);
    expect(
      test.client().getQueryData(queryKeys.sessionWarnings(selected.timezone, selected.sessionId)),
    ).toBeTruthy();
  });

it('revalidates a cached empty answer on opening and offers actions only after the current read', async () => {
  const test = setup();
  test.mount();
  await screen.findByRole('button', { name: /^Diagnose/ });
  test.select(null);
  test.client().setQueryData(queryKeys.sessionWarnings(selected.timezone, selected.sessionId), {
    ...read,
    warnings: [],
    dismissed: [],
  });
  let finish: (value: SessionWarningsList) => void = () => {};
  test.warnings.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  test.select(selected);
  await waitFor(() => expect(test.warnings).toHaveBeenCalledTimes(2));
  expect(screen.getByTestId('warnings-state').textContent).toBe(
    SESSION_WARNINGS_TEXT.session_reading,
  );
  expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
  await act(async () => finish(structuredClone(list)));
  expect(await screen.findByRole('button', { name: /^Diagnose/ })).toBeTruthy();
});

for (const running of [false, true])
  it(`disables cached actions after a failed current read and ${running ? 'keeps Stop' : 'drops confirmation and retries on reopening'}`, async () => {
    const test = setup();
    await ask(test);
    if (running) fireEvent.click(screen.getByRole('button', { name: words.confirm }));
    test.warnings.mockRejectedValueOnce(new Error('read failed'));
    await act(async () => {
      await test.client().invalidateQueries({
        queryKey: queryKeys.sessionWarnings(selected.timezone, selected.sessionId),
      });
    });
    expect(screen.getByTestId('warnings-state').textContent).toBe(
      SESSION_WARNINGS_TEXT.session_unread,
    );
    expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Dismiss' })).toBeNull();
    expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
    if (running) {
      fireEvent.click(screen.getByRole('button', { name: words.stop }));
      expect(test.controls.cancel).toHaveBeenCalledOnce();
      await test.answer(answers.cancelled!);
    } else {
      expect(test.controls.diagnose).not.toHaveBeenCalled();
      fireEvent.click(close());
      test.select(selected);
      expect(await screen.findByRole('button', { name: /^Diagnose/ })).toBeTruthy();
      expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
    }
  });

it('shows the local triggering message with bounded neighbours without a model send', async () => {
  const test = setup();
  test.mount();
  const context = await screen.findByTestId('warning-context');
  await waitFor(() => expect(context.textContent).toContain('Still not fixed.'));
  expect(context.textContent).toContain('The sample build is ready.');
  expect(context.textContent).toContain('I will inspect the retry limit');
  expect(context.textContent).not.toContain('unrelated message');
  expect(context.querySelectorAll('[data-trigger]')).toHaveLength(1);
  expect(test.controls.diagnose).not.toHaveBeenCalled();
});
it('reports read failure and retries without sending the conversation', async () => {
  const test = setup();
  test.transcript.mockRejectedValueOnce(new Error('unreadable'));
  test.mount();
  fireEvent.click(await screen.findByRole('button', { name: 'Try again' }));
  await screen.findByText(/Still not fixed/);
  expect(test.transcript).toHaveBeenCalledTimes(2);
  expect(test.cancelTranscript).toHaveBeenCalledOnce();
  expect(test.controls.diagnose).not.toHaveBeenCalled();
});
for (const replace of [false, true])
  it(`cancels its transcript and drops late text on ${replace ? 'switch' : 'close'}`, async () => {
    const test = setup();
    let finish: (status: SessionExcerptStatus) => void = () => {};
    test.transcript.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    test.mount();
    await screen.findByTestId('warning-context');
    await waitFor(() => expect(test.transcript).toHaveBeenCalledOnce());
    const readId = (
      test.transcript.mock.calls[0] as unknown as [string, string, number | null, string]
    )[3];
    if (replace)
      test.select({
        ...selected,
        sessionId: 'replacement',
        warning: {
          ...selected.warning,
          id: 'replacement',
          points_at: { record_uuid: 'replacement', block: null, sent_by: 'person' as const },
        },
      });
    else fireEvent.click(close());
    expect(test.cancelTranscript).toHaveBeenCalledWith(readId);
    const status = contextFixture(selected.warning.points_at.record_uuid);
    status.records[2]!.blocks = [{ kind: 'text', index: 0, text: 'LATE PRIVATE TEXT' }];
    await act(async () => finish(contextExcerpt(status, selected.warning.points_at.record_uuid)));
    expect(screen.queryByText('LATE PRIVATE TEXT')).toBeNull();
  });
it('refuses partial source sets instead of guessing the triggering message', async () => {
  const test = setup();
  test.transcript.mockResolvedValue({ state: 'unavailable', reason: { reason: 'incomplete' } });
  test.mount();
  await screen.findByText(/Part of this conversation could not be checked/);
  expect(screen.queryByText(/Still not fixed/)).toBeNull();
});
it('shows the exact target tool output expanded, with no unrelated block', async () => {
  const test = setup();
  const warning = {
    ...selected.warning,
    points_at: { record_uuid: 'tool-result', block: 99, sent_by: 'person' as const },
  };
  test.warnings.mockResolvedValue({
    timezone: selected.timezone,
    sessions: [{ session_id: selected.sessionId, answer: { ...read, warnings: [warning] } }],
  });
  const status = contextFixture('tool-result');
  status.records[2]!.blocks = [
    { kind: 'text', index: 0, text: 'UNRELATED BLOCK' },
    {
      kind: 'tool_result',
      index: 99,
      call_id: 'call',
      failed: true,
      parts: [{ kind: 'text', text: 'Exact failed output' }],
    },
  ];
  test.transcript.mockResolvedValue(contextExcerpt(status, 'tool-result', 99));
  test.mount();
  await screen.findByText('Exact failed output');
  expect(screen.getByTestId('warning-context').textContent).not.toContain('UNRELATED BLOCK');
  expect(screen.getByRole('region', { name: 'Output, scrollable code' })).toBeTruthy();
});

it('guards another warning in the same session and requests only its excerpt', async () => {
  const test = setup();
  const full = vi.spyOn(test.source, 'sessionTranscript');
  test.mount();
  await screen.findByText(/Still not fixed/);
  expect(full).not.toHaveBeenCalled();
  expect(test.transcript.mock.calls[0]?.slice(0, 3)).toEqual([
    selected.sessionId,
    selected.warning.points_at.record_uuid,
    null,
  ]);
  test.transcript.mockResolvedValue({ state: 'unavailable', reason: { reason: 'target_missing' } });
  const another = {
    ...selected.warning,
    id: 'another',
    points_at: { record_uuid: 'another', block: null, sent_by: 'person' as const },
  };
  test.warnings.mockResolvedValue({
    timezone: selected.timezone,
    sessions: [{ session_id: selected.sessionId, answer: { ...read, warnings: [another] } }],
  });
  test.select({ ...selected, warning: another });
  expect(screen.queryByText(/Still not fixed/)).toBeNull();
  await screen.findByText(/not in the original conversation/);
});

it('opens the current issue with its saved warning ID and range', async () => {
  const test = setup();
  test.mount();
  const link = await screen.findByRole('link', { name: 'View in full session' });
  expect(link.getAttribute('href')).toContain('warning=correction%3Aw-h3');
  expect(link.getAttribute('href')).toContain('range=7d');
  fireEvent.click(link);
  expect(test.onClose).toHaveBeenCalledOnce();
  expect(screen.getByTestId('current-location').textContent).toContain(
    '/sessions/s-warned?q=s-warned&range=7d&warning=correction%3Aw-h3',
  );
});

it('keeps a dismissed issue link, and opens the full session while a diagnosis runs without stopping it', async () => {
  const dismissed = setup();
  dismissed.warnings.mockResolvedValue(
    structuredClone(synthetic.after_dismissal.session_warnings) as SessionWarningsList,
  );
  dismissed.mount();
  const dismissedLink = await screen.findByRole('link', { name: 'View in full session' });
  expect(dismissedLink.getAttribute('href')).toContain('warning=correction%3Aw-h3');
  cleanup();
  const test = setup();
  await run(test);
  const link = screen.getByRole('link', { name: 'View in full session' });
  expect(link.getAttribute('aria-disabled')).toBeNull();
  fireEvent.click(link);
  expect(test.onClose).toHaveBeenCalledOnce();
  expect(screen.getByTestId('current-location').textContent).toContain('/sessions/s-warned');
  expect(test.controls.cancel).not.toHaveBeenCalled();
});

// Once Stop is accepted it waits in every view, a reopened window included,
// until the run actually ends.
it('keeps Stop waiting after an accepted Stop, in a reopened window too, until the run ends', async () => {
  const test = setup();
  await run(test);
  fireEvent.click(screen.getByRole('button', { name: words.stop }));
  await waitFor(() =>
    expect(screen.getByRole('button', { name: words.stop }).getAttribute('aria-disabled')).toBe(
      'true',
    ),
  );
  fireEvent.click(close());
  test.select(selected);
  const stop = await screen.findByRole('button', { name: words.stop });
  expect(stop.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(stop);
  expect(test.controls.cancel).toHaveBeenCalledOnce();
  await test.answer(answers.cancelled!);
  expect((await screen.findByTestId('diagnosis-result')).dataset.state).toBe('cancelled');
});

// An ingest re-reads the session's warnings. While that re-read is in
// flight the window keeps the last finished answer: the conversation is not
// read again, the confirm step stays and Send still works.
it('keeps the window, its conversation and the confirm step while a re-read is in flight', async () => {
  const test = setup();
  await ask(test);
  await screen.findByText(/Still not fixed/);
  expect(test.transcript).toHaveBeenCalledOnce();
  test.warnings.mockImplementationOnce(() => new Promise(() => {}));
  act(() => {
    void test.client().invalidateQueries({
      queryKey: queryKeys.sessionWarnings(selected.timezone, selected.sessionId),
    });
  });
  await waitFor(() => expect(test.warnings).toHaveBeenCalledTimes(2));
  expect(screen.queryByTestId('warnings-state')).toBeNull();
  expect(screen.getByTestId('warning-item')).toBeTruthy();
  expect(screen.getByTestId('warning-context').textContent).toContain('Still not fixed');
  expect(screen.getByTestId('diagnosis-confirm')).toBeTruthy();
  expect(test.transcript).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: words.confirm }));
  expect(test.controls.diagnose).toHaveBeenCalledOnce();
  expect(await screen.findByTestId('diagnosis-running')).toBeTruthy();
});

it('drops the warning, its confirm step and its actions when a finished re-read no longer lists it', async () => {
  const test = setup();
  await ask(test);
  test.warnings.mockResolvedValueOnce({
    timezone: selected.timezone,
    sessions: [
      {
        session_id: selected.sessionId,
        answer: {
          ...read,
          warnings: read.warnings.filter((one) => one.id !== selected.warning.id),
          dismissed: [],
        },
      },
    ],
  });
  await act(async () => {
    await test.client().invalidateQueries({
      queryKey: queryKeys.sessionWarnings(selected.timezone, selected.sessionId),
    });
  });
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Dismiss' })).toBeNull();
  expect(screen.getByTestId('warnings-state').textContent).toBe(
    SESSION_WARNINGS_TEXT.session_unavailable,
  );
  expect(test.controls.diagnose).not.toHaveBeenCalled();
});

// Selecting another warning while the window stays open remounts its content
// (the content is keyed by session and warning), so that warning, too, acts
// only on a read finished after it opened, not on the cached answer.
it('waits for a fresh read after switching to another warning in the open window', async () => {
  const test = setup();
  test.mount();
  await screen.findByRole('button', { name: /^Diagnose/ });
  const other = read.warnings.find((one) => one.id !== selected.warning.id)!;
  let finish: (value: SessionWarningsList) => void = () => {};
  test.warnings.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  test.select({ ...selected, warning: other });
  await waitFor(() => expect(test.warnings).toHaveBeenCalledTimes(2));
  expect(screen.getByTestId('warnings-state').textContent).toBe(
    SESSION_WARNINGS_TEXT.session_reading,
  );
  expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
  await act(async () => finish(structuredClone(list)));
  expect((await screen.findByTestId('warning-item')).dataset.warningId).toBe(other.id);
});

const quoted = (
  marks = [
    { block: 0, start: 0, end: 5 },
    { block: 0, start: 6, end: 9 },
  ],
) =>
  contextExcerpt(
    contextFixture(selected.warning.points_at.record_uuid),
    selected.warning.points_at.record_uuid,
    null,
    marks,
  );

/** The heading over why a warning was flagged, and the reason it is described by. */
function flagged() {
  const heading = screen.getByTestId('warning-heading');
  const described = document.getElementById(heading.getAttribute('aria-describedby') ?? '');
  return { heading, reason: described?.textContent };
}

// The reason is a tooltip on the heading, not a line of its own: hovering
// or focusing the heading (a tab stop) shows it, and the heading is
// described by it for a screen reader.
it('puts the reason in a tooltip on the heading, opened by keyboard focus', async () => {
  const test = setup();
  test.transcript.mockResolvedValue(quoted([]));
  test.mount();
  await screen.findByTestId('warning-heading');
  const { heading, reason } = flagged();
  expect(heading.tagName).toBe('H3');
  expect(heading.textContent).toBe(SESSION_WARNINGS_TEXT.flagged_headings.correction);
  expect(heading.tabIndex).toBe(0);
  expect(reason).toBe(selected.warning.reason);
  // No visible reason line: the only copy in the window is the hidden description.
  expect(screen.queryByTestId('warning-reason')).toBeNull();
  expect(screen.getByTestId('warning-reason-text').className).toBe('sr-only');
  expect(screen.queryByTestId('warning-reason-tip')).toBeNull();
  heading.focus();
  fireEvent.focus(heading);
  expect((await screen.findByTestId('warning-reason-tip')).textContent).toBe(
    selected.warning.reason,
  );
  // Hover is checked in a real browser (dashboard-warning-interaction.spec.ts).
});

// A correction's reason is its whole message with the native marks shown;
// the conversation and the reason share one read.
it('shows a correction as "you wrote" and its whole message with the native marks', async () => {
  const test = setup();
  test.transcript.mockResolvedValue(quoted());
  test.mount();
  await waitFor(() => expect(screen.getByTestId('warning-reason').dataset.quote).toBe(''));
  const reason = screen.getByTestId('warning-reason');
  expect(reason.textContent).toBe(
    `${SESSION_WARNINGS_TEXT.you_wrote}Still not fixed. The sample build fails at the same step. Please check the retry limit instead of changing the timeout again.`,
  );
  expect([...reason.querySelectorAll('mark')].map((mark) => mark.textContent)).toEqual([
    'Still',
    'not',
  ]);
  expect(reason.textContent).not.toContain(selected.warning.reason);
  // The heading still carries the reason, as for every kind.
  expect(flagged().reason).toBe(selected.warning.reason);
  expect(test.transcript).toHaveBeenCalledOnce();
  expect((test.transcript.mock.calls[0] as unknown[])[4]).toEqual({
    id: selected.warning.id,
    timezone: selected.timezone,
  });
  expect(dialog().textContent).not.toContain('take a closer look');
});

for (const state of ['loading', 'failed', 'unmarked', 'unavailable'] as const)
  it(`gives the correction only its heading's reason while the message is ${state}`, async () => {
    const test = setup();
    if (state === 'loading') test.transcript.mockImplementation(() => new Promise(() => {}));
    else if (state === 'failed') test.transcript.mockRejectedValue(new Error('unreadable'));
    else if (state === 'unmarked') test.transcript.mockResolvedValue(quoted([]));
    else
      test.transcript.mockResolvedValue({
        state: 'unavailable',
        reason: { reason: 'target_missing' },
      });
    test.mount();
    await screen.findByTestId('warning-heading');
    await waitFor(() => expect(test.transcript).toHaveBeenCalled());
    if (state !== 'loading')
      await waitFor(() =>
        expect(screen.queryByText('Loading the original conversation…')).toBeNull(),
      );
    expect(flagged().reason).toBe(selected.warning.reason);
    expect(screen.queryByTestId('warning-reason')).toBeNull();
    expect(dialog().querySelector('mark')).toBeNull();
    expect(dialog().textContent).not.toContain('take a closer look');
  });

// The native side marks only a correction it can stand behind; for any
// other warning it sends no marks, and the window shows the reason.
it('asks for every warning by its own ID and shows its reason when nothing is marked', async () => {
  const test = setup();
  const other = read.warnings.find((one) => one.kind !== 'correction')!;
  test.transcript.mockResolvedValue(quoted([]));
  test.mount();
  await screen.findByTestId('warning-heading');
  test.select({ ...selected, warning: other });
  await waitFor(() => expect(screen.getByTestId('warning-item').dataset.warningId).toBe(other.id));
  await waitFor(() => expect(test.transcript).toHaveBeenCalledTimes(2));
  expect((test.transcript.mock.calls[1] as unknown[])[4]).toEqual({
    id: other.id,
    timezone: selected.timezone,
  });
  expect(flagged().heading.textContent).toBe(SESSION_WARNINGS_TEXT.flagged_headings[other.kind]);
  expect(flagged().reason).toBe(other.reason);
  expect(screen.queryByTestId('warning-reason')).toBeNull();
});

it('names an admitted mistake with its own heading and tag, its native reason on the heading', async () => {
  const test = setup();
  // Supplied synthetic DTO: the native side words the reason and tag.
  const admitted = {
    ...selected.warning,
    id: 'admitted_mistake:synthetic-reply',
    kind: 'admitted_mistake' as const,
    label: 'Admitted mistake',
    reason: 'Supplied admission reason (10:05)',
    compact_reason: null,
    record_uuids: [selected.warning.points_at.record_uuid, 'synthetic-reply'],
    points_at: { record_uuid: 'synthetic-reply', block: null, sent_by: 'agent' as const },
  };
  const withAdmission = structuredClone(list);
  const answer = withAdmission.sessions.find((one) => one.session_id === 's-warned')!.answer;
  if (answer.state === 'read') answer.warnings.push(admitted);
  test.warnings.mockImplementation(async () => structuredClone(withAdmission));
  test.transcript.mockResolvedValue(
    contextExcerpt(contextFixture('synthetic-reply'), 'synthetic-reply'),
  );
  test.mount();
  await screen.findByTestId('warning-heading');
  test.select({ ...selected, warning: admitted });
  await waitFor(() =>
    expect(screen.getByTestId('warning-item').dataset.warningId).toBe(admitted.id),
  );
  expect(
    within(dialog()).getByRole('heading', {
      name: SESSION_WARNINGS_TEXT.flagged_headings.admitted_mistake,
    }),
  ).toBeTruthy();
  expect(SESSION_WARNINGS_TEXT.flagged_headings.admitted_mistake).toBe(
    'The agent admitted a mistake.',
  );
  expect(SESSION_WARNINGS_TEXT.message_tags.admitted_mistake).toBe('Admitted mistake');
  await waitFor(() =>
    expect(screen.getByTestId('warning-context-flag').textContent).toBe('Admitted mistake'),
  );
  // Its reason is the heading's tooltip, not a line under it.
  expect(flagged().reason).toBe(admitted.reason);
  expect(screen.queryByTestId('warning-reason')).toBeNull();
  fireEvent.focus(flagged().heading);
  expect((await screen.findByTestId('warning-reason-tip')).textContent).toBe(admitted.reason);
});

/** A full press on the window's backdrop, as a pointer makes it. */
function pressBackdrop() {
  const backdrop = document.querySelector<HTMLElement>('.xt-modal-backdrop')!;
  fireEvent.pointerDown(backdrop, { button: 0, pointerType: 'mouse' });
  fireEvent.mouseDown(backdrop, { button: 0 });
  fireEvent.pointerUp(backdrop, { button: 0, pointerType: 'mouse' });
  fireEvent.mouseUp(backdrop, { button: 0 });
  fireEvent.click(backdrop, { button: 0 });
}

// A press outside the window closes it the way × and Escape do: a running
// diagnosis keeps running, and focus goes back to what opened it.
it('closes on a press outside the window, keeps a running diagnosis, and returns focus', async () => {
  const test = setup();
  await run(test);
  // A press inside, on the conversation, does not close it.
  const context = screen.getByTestId('warning-context');
  fireEvent.pointerDown(context, { button: 0, pointerType: 'mouse' });
  fireEvent.mouseDown(context, { button: 0 });
  fireEvent.pointerUp(document.body, { button: 0, pointerType: 'mouse' });
  fireEvent.mouseUp(document.body, { button: 0 });
  fireEvent.click(document.body, { button: 0 });
  expect(test.onClose).not.toHaveBeenCalled();
  pressBackdrop();
  await waitFor(() => expect(test.onClose).toHaveBeenCalledOnce());
  expect(test.controls.cancel).not.toHaveBeenCalled();
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Origin' })),
  );
});

it('leaves every other window open on a press outside it', async () => {
  const onOpenChange = vi.fn();
  render(
    <ThemeProvider>
      <Modal open onOpenChange={onOpenChange} aria-label="Synthetic other window">
        <button type="button">Inside</button>
      </Modal>
    </ThemeProvider>,
  );
  await screen.findByRole('dialog', { name: 'Synthetic other window' });
  pressBackdrop();
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(onOpenChange).not.toHaveBeenCalled();
  expect(screen.getByRole('dialog', { name: 'Synthetic other window' })).toBeTruthy();
});
