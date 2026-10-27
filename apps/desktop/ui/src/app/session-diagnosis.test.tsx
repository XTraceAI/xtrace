import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import warningsSynthetic from './session-diagnosis-warnings.synthetic.json';
import synthetic from './session-diagnosis.synthetic.json';
import { DataProvider } from '../data/DataProvider';
import type { DiagnosisControls } from '../data/DataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SavedSkill } from '../data/generated/SavedSkill';
import type { SessionDiagnosis } from '../data/generated/SessionDiagnosis';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../data/generated/SessionWarningsAnswer';
import type { SessionWarningsList } from '../data/generated/SessionWarningsList';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { ThemeProvider } from '../theme/ThemeProvider';
import { DetachedDiagnosis, DiagnoseButton, DiagnosisPanel } from './session-diagnosis';
import { useDiagnose } from './useSessionDiagnosis';
import { SessionWarningsDetail, useSessionWarning } from './session-warnings';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/**
 * Test-only synthetic Diagnose answers assembled by the app's own Rust answer
 * code (`session_diagnosis_tests.rs::ui_export`), returned as they are.
 */
const answers = synthetic.diagnoses as unknown as Record<string, SessionDiagnosis>;
const WORDS = SESSION_WARNINGS_TEXT.diagnose;
const served = () =>
  ({
    ...structuredClone(fixture),
    ...structuredClone(warningsSynthetic),
  }) as unknown as FixtureExport;

/**
 * The fixture with Diagnose controls whose runs wait until the test answers
 * them with one of Rust's answers.
 */
function withDiagnosis(saveAnswer: SavedSkill = synthetic.saved as SavedSkill) {
  const source = new FixtureDataSource(served());
  const runs: { readId: string; answer: (value: SessionDiagnosis | Error) => void }[] = [];
  const controls: DiagnosisControls = {
    diagnose: vi.fn(
      (_session: string, _warning: string, _zone: string, readId: string) =>
        new Promise<SessionDiagnosis>((resolve, reject) =>
          runs.push({
            readId,
            answer: (value) => (value instanceof Error ? reject(value) : resolve(value)),
          }),
        ),
    ),
    cancel: vi.fn(async () => true),
    saveSkill: vi.fn(async () => saveAnswer),
  };
  Object.defineProperty(source, 'diagnosis', { value: controls });
  const answer = (value: SessionDiagnosis | Error) =>
    act(async () => {
      runs.at(-1)!.answer(value);
    });
  return { source, controls, runs, answer };
}

/** A session page's Warnings card with Diagnose, over the real read path. */
function Page({ id }: { id: string }) {
  const query = useSessionWarning(id, 'UTC');
  const active = query.data?.state === 'read' ? query.data.warnings : undefined;
  const diagnose = useDiagnose(id, 'UTC', active);
  return (
    <>
      {diagnose && <DetachedDiagnosis diagnose={diagnose} warnings={active} />}
      <SessionWarningsDetail
        sessionId={id}
        answer={query.data}
        pending={query.isPending}
        failed={query.isError}
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
    </>
  );
}

const show = async (source: FixtureDataSource) => {
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter>
          <Page id="s-warned" />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  return screen.findAllByTestId('warning-item');
};

const diagnoseButton = (row: HTMLElement) =>
  within(row).getByRole('button', { name: /^Diagnose this warning: / });

/** Asks for the first warning's diagnosis and confirms it. */
async function run(source: FixtureDataSource) {
  const [row] = await show(source);
  fireEvent.click(diagnoseButton(row!));
  fireEvent.click(within(row!).getByRole('button', { name: WORDS.confirm }));
  await within(row!).findByTestId('diagnosis-running');
  return row!;
}

it('says what will be sent, to whom, and sends nothing until confirmed', async () => {
  const { source, controls } = withDiagnosis();
  const [row] = await show(source);
  const button = diagnoseButton(row!);
  expect(button.textContent).toBe(WORDS.button);
  expect(button.tabIndex).toBe(0);
  fireEvent.click(button);
  const confirm = within(row!).getByTestId('diagnosis-confirm');
  expect(confirm.textContent).toContain(SESSION_WARNINGS_TEXT.diagnose_before_sending);
  const send = within(confirm).getByRole('button', { name: WORDS.confirm });
  await waitFor(() => expect(document.activeElement).toBe(send));
  // Escape, or Not now, closes it: nothing was sent, focus is back.
  fireEvent.keyDown(send, { key: 'Escape' });
  expect(within(row!).queryByTestId('diagnosis-confirm')).toBeNull();
  expect(document.activeElement).toBe(button);
  fireEvent.click(button);
  fireEvent.click(within(row!).getByRole('button', { name: WORDS.not_now }));
  expect(within(row!).queryByTestId('diagnosis-confirm')).toBeNull();
  expect(controls.diagnose).not.toHaveBeenCalled();
});

it('runs one at a time, with Stop naming its own run', async () => {
  const { source, controls, runs, answer } = withDiagnosis();
  const row = await run(source);
  expect(controls.diagnose).toHaveBeenCalledExactlyOnceWith(
    's-warned',
    row.dataset.warningId,
    'UTC',
    runs[0]!.readId,
  );
  const running = within(row).getByTestId('diagnosis-running');
  expect(within(running).getByRole('status').textContent).toBe(WORDS.running);
  const stop = within(running).getByRole('button', { name: WORDS.stop });
  await waitFor(() => expect(document.activeElement).toBe(stop));
  // Every other Diagnose button waits.
  const others = screen.getAllByRole('button', { name: /^Diagnose this warning: / });
  expect(others.every((other) => other.getAttribute('aria-disabled') === 'true')).toBe(true);
  fireEvent.click(others[1]!);
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  fireEvent.click(stop);
  expect(controls.cancel).toHaveBeenCalledExactlyOnceWith(runs[0]!.readId);
  expect(stop.getAttribute('aria-disabled')).toBe('true');
  await answer(answers.cancelled!);
  const result = within(row).getByTestId('diagnosis-result');
  expect(within(result).getByTestId('diagnosis-text').textContent).toBe(
    (answers.cancelled as { text: string }).text,
  );
});

it('shows a suggested skill whole, where it saves, and saves it on a click', async () => {
  const { source, controls, answer } = withDiagnosis();
  const row = await run(source);
  await answer(answers.make_skill!);
  const made = answers.make_skill as Extract<SessionDiagnosis, { state: 'diagnosed' }>;
  const result = within(row).getByTestId('diagnosis-result');
  // Focus moves into the answer.
  await waitFor(() =>
    expect(document.activeElement).toBe(
      within(result).getByRole('heading', { name: WORDS.heading }),
    ),
  );
  expect(within(result).getByTestId('diagnosis-ran').textContent).toBe(made.ran);
  expect(within(result).getByTestId('diagnosis-what').textContent).toContain(made.what_happened);
  expect(within(result).getByTestId('diagnosis-real').textContent).toBe(made.real_problem_text);
  expect(within(result).getByTestId('diagnosis-suggestion').textContent).toBe(made.suggestion_text);
  expect(within(result).getByTestId('diagnosis-why').textContent).toContain(made.why);
  expect(within(result).getByTestId('diagnosis-sent').textContent).toContain(made.sent.text);
  // The whole draft, in a box the keyboard can reach.
  const draft = within(result).getByTestId('diagnosis-draft');
  expect(draft.textContent).toBe(made.draft);
  expect(draft.tabIndex).toBe(0);
  expect(within(result).getByTestId('diagnosis-path').textContent).toBe(
    `${WORDS.skill_path} ${made.skill_path}`,
  );
  const save = within(result).getByRole('button', { name: WORDS.save_skill });
  fireEvent.click(save);
  expect(controls.saveSkill).toHaveBeenCalledExactlyOnceWith(
    's-warned',
    made.skill_name,
    made.draft,
  );
  const saved = await within(result).findByTestId('diagnosis-saved');
  const said = synthetic.saved as Extract<SavedSkill, { state: 'saved' }>;
  expect(saved.textContent).toBe(`${said.text} ${said.path}`);
  expect(save.getAttribute('aria-disabled')).toBe('true');
  // Close puts it away and gives focus back to Diagnose.
  fireEvent.click(within(result).getByRole('button', { name: WORDS.close }));
  await waitFor(() => expect(within(row).queryByTestId('diagnosis-result')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(diagnoseButton(row)));
  fireEvent.click(diagnoseButton(row));
  expect(within(row).getByTestId('diagnosis-confirm')).toBeTruthy();
});

it('says why a skill was not saved', async () => {
  const { source, answer } = withDiagnosis(synthetic.save_refused as SavedSkill);
  const row = await run(source);
  await answer(answers.make_skill!);
  fireEvent.click(within(row).getByRole('button', { name: WORDS.save_skill }));
  const saved = await within(row).findByTestId('diagnosis-saved');
  expect(saved.textContent).toBe((synthetic.save_refused as { text: string }).text);
  expect(saved.dataset.state).toBe('refused');
});

it('offers an instruction line to copy, names its file, and writes nothing', async () => {
  const { source, controls, answer } = withDiagnosis();
  const writeText = vi.fn(async () => {});
  vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
  const row = await run(source);
  await answer(answers.add_to_claude_md!);
  const line = answers.add_to_claude_md as Extract<SessionDiagnosis, { state: 'diagnosed' }>;
  const result = within(row).getByTestId('diagnosis-result');
  const part = within(result).getByTestId('diagnosis-line');
  expect(part.textContent).toContain(WORDS.line_for_file.replace('{file}', line.instruction_file!));
  expect(within(part).getByTestId('diagnosis-line-text').textContent).toBe(line.draft);
  fireEvent.click(within(part).getByRole('button', { name: WORDS.copy }));
  expect(writeText).toHaveBeenCalledExactlyOnceWith(line.draft);
  expect((await within(part).findByTestId('diagnosis-copied')).textContent).toBe(WORDS.copied);
  expect(controls.saveSkill).not.toHaveBeenCalled();
  // The tool that was skipped, and what the one used also reads.
  expect(within(result).getByTestId('diagnosis-tried').textContent).toContain(line.tried[0]!.text);
  expect(within(result).getByTestId('diagnosis-limits').textContent).toContain(
    line.limits[0]!.text,
  );
});

it('says when copying failed', async () => {
  const { source, answer } = withDiagnosis();
  vi.stubGlobal('navigator', {
    ...navigator,
    clipboard: { writeText: vi.fn(async () => Promise.reject(new Error('denied'))) },
  });
  const row = await run(source);
  await answer(answers.add_to_claude_md!);
  fireEvent.click(within(row).getByRole('button', { name: WORDS.copy }));
  expect((await within(row).findByTestId('diagnosis-copied')).textContent).toBe(WORDS.copy_failed);
});

for (const state of ['no_agent_tool', 'run_failed', 'answer_refused', 'busy'] as const)
  it(`says the native sentence when it could not diagnose: ${state}`, async () => {
    const { source, answer } = withDiagnosis();
    const row = await run(source);
    const given = answers[state] as Extract<SessionDiagnosis, { text: string }>;
    await answer(given);
    const result = within(row).getByTestId('diagnosis-result');
    expect(result.dataset.state).toBe(state);
    expect(within(result).getByTestId('diagnosis-text').textContent).toBe(given.text);
    if ('tried' in given)
      for (const tried of given.tried) expect(result.textContent).toContain(tried.text);
    if ('reason' in given && typeof given.reason === 'string')
      expect(result.textContent).toContain(given.reason);
    if ('ran' in given)
      expect(within(result).getByTestId('diagnosis-ran').textContent).toBe(given.ran);
  });

it('says when the diagnosis could not be asked for at all', async () => {
  const { source, answer } = withDiagnosis();
  const row = await run(source);
  await answer(new Error('worker gone'));
  expect(within(row).getByTestId('diagnosis-result').textContent).toContain(WORDS.failed);
});

it('keeps an answer in memory while the page is left and opened again', async () => {
  const { source, answer } = withDiagnosis();
  let setShown: (shown: boolean) => void = () => {};
  function Toggle() {
    const [shown, set] = useState(true);
    setShown = set;
    return shown ? <Page id="s-warned" /> : null;
  }
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter>
          <Toggle />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  const [row] = await screen.findAllByTestId('warning-item');
  fireEvent.click(diagnoseButton(row!));
  fireEvent.click(within(row!).getByRole('button', { name: WORDS.confirm }));
  await answer(answers.nothing!);
  await screen.findByTestId('diagnosis-result');
  act(() => setShown(false));
  expect(screen.queryByTestId('diagnosis-result')).toBeNull();
  act(() => setShown(true));
  const result = await screen.findByTestId('diagnosis-result');
  expect(within(result).getByTestId('diagnosis-real').textContent).toBe(
    (answers.nothing as { real_problem_text: string }).real_problem_text,
  );
});

for (const running of [false, true])
  it(`navigation drops old confirmation and keeps a running Stop (${running})`, async () => {
    const { source, controls, answer } = withDiagnosis();
    let navigate: (id: string) => void = () => {};
    function Navigate() {
      const [id, setId] = useState('s-warned');
      navigate = setId;
      return <Page id={id} />;
    }
    render(
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter>
            <Navigate />
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>,
    );
    const [row] = await screen.findAllByTestId('warning-item');
    fireEvent.click(diagnoseButton(row!));
    if (running) fireEvent.click(screen.getByRole('button', { name: WORDS.confirm }));
    act(() => navigate('s-clean'));
    expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
    if (running) {
      fireEvent.click(screen.getByRole('button', { name: WORDS.stop }));
      expect(controls.cancel).toHaveBeenCalledOnce();
      await answer(answers.cancelled!);
    } else expect(controls.diagnose).not.toHaveBeenCalled();
    act(() => navigate('s-warned'));
    const rows = await screen.findAllByTestId('warning-item');
    if (running) await screen.findByTestId('diagnosis-result');
    else {
      expect(diagnoseButton(rows[0]!).getAttribute('aria-disabled')).toBeNull();
      fireEvent.click(diagnoseButton(rows[0]!));
      expect(screen.getByTestId('diagnosis-confirm')).toBeTruthy();
    }
  });

it('lets Stop be retried when cancellation was not accepted', async () => {
  const { source, controls } = withDiagnosis();
  vi.mocked(controls.cancel)
    .mockResolvedValueOnce(false)
    .mockRejectedValueOnce(new Error('unavailable'));
  const row = await run(source);
  const stop = within(row).getByRole('button', { name: WORDS.stop });
  for (let count = 1; count <= 2; count++) {
    fireEvent.click(stop);
    await waitFor(() => expect(stop.getAttribute('aria-disabled')).toBeNull());
    expect(controls.cancel).toHaveBeenCalledTimes(count);
  }
});

for (const running of [false, true])
  it(`removing a warning clears its confirmation or keeps Stop (${running})`, async () => {
    const { source, controls, answer } = withDiagnosis();
    const list = structuredClone(warningsSynthetic.session_warnings) as SessionWarningsList;
    vi.spyOn(source, 'sessionWarnings').mockImplementation(async () => structuredClone(list));
    Object.defineProperty(source, 'warningDismissals', {
      value: {
        dismiss: async (sessionId: string, warningId: string) => {
          const read = list.sessions.find((session) => session.session_id === sessionId)!
            .answer as Extract<SessionWarningsAnswer, { state: 'read' }>;
          const index = read.warnings.findIndex((warning) => warning.id === warningId);
          read.dismissed.push(read.warnings.splice(index, 1)[0]!);
          return { session_id: sessionId, warning_id: warningId, dismissed: true };
        },
        restore: async () => {
          throw new Error('unused');
        },
      },
    });
    const [row] = await show(source);
    fireEvent.click(diagnoseButton(row!));
    if (running) fireEvent.click(screen.getByRole('button', { name: WORDS.confirm }));
    fireEvent.click(within(row!).getByRole('button', { name: SESSION_WARNINGS_TEXT.dismiss }));
    await screen.findByTestId('dismissed-warning');
    expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
    if (running) {
      fireEvent.click(screen.getByRole('button', { name: WORDS.stop }));
      expect(controls.cancel).toHaveBeenCalledOnce();
      await answer(answers.cancelled!);
    } else expect(controls.diagnose).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('button', { name: /^Diagnose this warning:/ })
          .every((button) => button.getAttribute('aria-disabled') !== 'true'),
      ).toBe(true),
    );
  });

it('offers no Diagnose where no coding tool can be run, and none on a dismissed warning', async () => {
  const source = new FixtureDataSource(served());
  await show(source);
  expect(screen.queryByRole('button', { name: /^Diagnose/ })).toBeNull();
  cleanup();
  const { source: withTools } = withDiagnosis();
  const list = structuredClone(warningsSynthetic.session_warnings) as SessionWarningsList;
  const entry = list.sessions.find((s) => s.session_id === 's-warned')!;
  const read = entry.answer as Extract<SessionWarningsAnswer, { state: 'read' }>;
  read.dismissed = [read.warnings.shift()!];
  vi.spyOn(withTools, 'sessionWarnings').mockResolvedValue(list);
  await show(withTools);
  const gone = await screen.findByTestId('dismissed-warning');
  expect(within(gone).queryByRole('button', { name: /^Diagnose/ })).toBeNull();
  expect(screen.getAllByRole('button', { name: /^Diagnose this warning/ })).toHaveLength(
    read.warnings.length,
  );
});

it('says on a warning that it shows again because it grew', async () => {
  const { source } = withDiagnosis();
  const list = structuredClone(warningsSynthetic.session_warnings) as SessionWarningsList;
  const read = list.sessions.find((s) => s.session_id === 's-warned')!.answer as Extract<
    SessionWarningsAnswer,
    { state: 'read' }
  >;
  read.warnings[0]!.returned =
    'Shown again: you dismissed it at 2 failures; the call has now failed 3 times.';
  vi.spyOn(source, 'sessionWarnings').mockResolvedValue(list);
  const [row] = await show(source);
  expect(within(row!).getByTestId('warning-returned').textContent).toBe(read.warnings[0]!.returned);
  expect(row!.dataset.pointsAt).toBe(
    read.warnings[0]!.points_at.block === null
      ? read.warnings[0]!.points_at.record_uuid
      : `${read.warnings[0]!.points_at.record_uuid}#${read.warnings[0]!.points_at.block}`,
  );
});

it('opens the exact sent message from a citation without rereading the source or sending again', async () => {
  const { source, controls, answer } = withDiagnosis();

  const transcriptRead = vi.spyOn(source, 'sessionTranscript');
  const row = await run(source);
  const made = structuredClone(answers.make_skill) as Extract<
    SessionDiagnosis,
    { state: 'diagnosed' }
  >;
  made.messages[10]!.text =
    'A saved line\n[1] fake label <script>alert(1)</script> https://example.test';
  await answer(made);
  const evidence = within(row).getByTestId('diagnosis-evidence');
  expect(evidence.hasAttribute('open')).toBe(false);
  expect(within(row).getByTestId('diagnosis-details').hasAttribute('open')).toBe(false);
  fireEvent.click(within(row).getByRole('button', { name: '[11]' }));
  const messages = within(evidence).getAllByTestId('diagnosis-message');
  expect(messages).toHaveLength(made.messages.length);
  expect(evidence.hasAttribute('open')).toBe(true);
  await waitFor(() => expect(document.activeElement).toBe(messages[10]));
  expect(messages[10]!.textContent).toBe(made.messages[10]!.label + made.messages[10]!.text);
  expect(messages[10]!.querySelector('script, a')).toBeNull();
  expect(messages[0]!.textContent).toContain('[cut]');
  expect(controls.diagnose).toHaveBeenCalledTimes(1);

  expect(transcriptRead).not.toHaveBeenCalled();
});

for (const key of ['nothing', 'problem_no_suggestion', 'not_sure'] as const) {
  it(`shows the native verdict separately from no suggested action: ${key}`, async () => {
    const { source, controls, answer } = withDiagnosis();
    const row = await run(source);
    const made = answers[key] as Extract<SessionDiagnosis, { state: 'diagnosed' }>;
    await answer(made);
    expect(within(row).getByTestId('diagnosis-real').textContent).toBe(made.real_problem_text);
    expect(within(row).getByTestId('diagnosis-suggestion').textContent).toBe(
      'No skill or instruction change suggested.',
    );
    expect(within(row).queryByTestId('diagnosis-skill')).toBeNull();
    expect(within(row).queryByTestId('diagnosis-line')).toBeNull();
    if (key === 'not_sure') {
      expect(within(row).queryByRole('button', { name: '[999]' })).toBeNull();
      expect(row.textContent).toContain(WORDS.reference_unavailable);
    }
    expect(controls.saveSkill).not.toHaveBeenCalled();
  });
}

it('destroys evidence selection on changed saved point and does not restore it on A to B to A', async () => {
  const { source, controls, answer } = withDiagnosis();
  const initial = warningsSynthetic.session_warnings.sessions.flatMap((entry) =>
    entry.answer.state === 'read' ? entry.answer.warnings : [],
  )[0] as SessionWarningItem;
  let change: (warning: SessionWarningItem) => void = () => {};
  function Owned() {
    const [warning, setWarning] = useState(initial);
    change = setWarning;
    const diagnose = useDiagnose('s-warned', 'UTC', [warning])!;
    return (
      <div data-warning-id={warning.id}>
        <DiagnoseButton diagnose={diagnose} warning={warning} />
        <DiagnosisPanel diagnose={diagnose} warning={warning} />
      </div>
    );
  }
  render(
    <DataProvider source={source}>
      <Owned />
    </DataProvider>,
  );
  fireEvent.click(await screen.findByRole('button', { name: /^Diagnose this warning:/ }));
  fireEvent.click(screen.getByRole('button', { name: WORDS.confirm }));
  await answer(answers.nothing!);
  await screen.findByTestId('diagnosis-result');
  fireEvent.click(screen.getByRole('button', { name: '[13]' }));
  await waitFor(() => expect(document.activeElement?.getAttribute('data-number')).toBe('13'));
  act(() =>
    change({ ...initial, points_at: { record_uuid: 'replacement-saved-point', block: null } }),
  );
  expect(screen.getByTestId('diagnosis-evidence').hasAttribute('open')).toBe(false);
  expect(
    screen
      .getAllByTestId('diagnosis-message')
      .some((message) => message.hasAttribute('data-selected')),
  ).toBe(false);
  act(() => change(initial));
  expect(screen.getByTestId('diagnosis-evidence').hasAttribute('open')).toBe(false);
  expect(
    screen
      .getAllByTestId('diagnosis-message')
      .some((message) => message.hasAttribute('data-selected')),
  ).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: WORDS.close }));
  await waitFor(() => expect(screen.queryByTestId('diagnosis-result')).toBeNull());
  fireEvent.click(screen.getByRole('button', { name: /^Diagnose this warning:/ }));
  expect(screen.getByTestId('diagnosis-confirm')).toBeTruthy();
  expect(controls.diagnose).toHaveBeenCalledTimes(1);
});

it('shows the native missing-project sentence and offers no Save without a native skill path', async () => {
  const { source, controls, answer } = withDiagnosis();
  const row = await run(source);
  const made = structuredClone(answers.make_skill) as Extract<
    SessionDiagnosis,
    { state: 'diagnosed' }
  >;
  made.skill_path = null;
  await answer(made);
  expect(within(row).getByTestId('diagnosis-draft').textContent).toBe(made.draft);
  expect(row.textContent).toContain(WORDS.skill_no_path);
  expect(within(row).queryByRole('button', { name: WORDS.save_skill })).toBeNull();
  expect(controls.saveSkill).not.toHaveBeenCalled();
});

it('shows the native save-failure sentence when the explicit save request rejects', async () => {
  const { source, controls, answer } = withDiagnosis();
  vi.mocked(controls.saveSkill).mockRejectedValueOnce(Error('Synthetic save failure'));
  const row = await run(source);
  await answer(answers.make_skill!);
  fireEvent.click(within(row).getByRole('button', { name: WORDS.save_skill }));
  expect((await within(row).findByTestId('diagnosis-saved')).textContent).toBe(WORDS.save_failed);
});
