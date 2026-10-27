import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { session, warningSource } from '../../../e2e/mounted-session-warnings.data';
import { second, warning } from '../../../e2e/session-warnings.data';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource, DiagnosisControls } from '../../data/DataSource';
import type { SessionPageSelection } from '../../data/generated/SessionPageSelection';
import type { SessionPageStatus } from '../../data/generated/SessionPageStatus';
import type { SessionDiagnosis } from '../../data/generated/SessionDiagnosis';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import synthetic from '../session-diagnosis.synthetic.json';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
beforeEach(() => {
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe(target: Element) {
        queueMicrotask(() =>
          this.callback(
            [{ target, isIntersecting: true }] as IntersectionObserverEntry[],
            this as unknown as IntersectionObserver,
          ),
        );
      }
      unobserve() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
const words = SESSION_WARNINGS_TEXT.diagnose;
const answers = synthetic.diagnoses as unknown as Record<string, SessionDiagnosis>;
function supplied() {
  const source = warningSource();
  let finish!: (answer: SessionDiagnosis) => void;
  const controls: DiagnosisControls = {
    diagnose: vi.fn(
      () =>
        new Promise<SessionDiagnosis>((resolve) => {
          finish = resolve;
        }),
    ),
    cancel: vi.fn(async () => true),
    saveSkill: vi.fn(async () => ({
      state: 'saved' as const,
      text: 'Synthetic saved',
      path: 'synthetic/SKILL.md',
    })),
  };
  Object.defineProperty(source, 'diagnosis', { value: controls });
  return {
    source,
    controls,
    answer: (answer: SessionDiagnosis) => act(async () => finish(answer)),
  };
}
function mount(initial = supplied()) {
  let navigate!: ReturnType<typeof useNavigate>;
  function Navigation() {
    navigate = useNavigate();
    return null;
  }
  function App({ source }: { source: typeof initial.source }) {
    return (
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter
            initialEntries={[`/sessions/${session.id}?warning=${encodeURIComponent(warning.id)}`]}
          >
            <AppRoutes />
            <Navigation />
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>
    );
  }
  const view = render(<App source={initial.source} />);
  return {
    ...initial,
    view,
    replace: (next: typeof initial.source) => view.rerender(<App source={next} />),
    navigate: (path: string) =>
      act(() => {
        void navigate(path);
      }),
  };
}
const selected = () => screen.getByRole('list', { name: 'Selected warning' });
const ask = () =>
  fireEvent.click(within(selected()).getByRole('button', { name: /^Diagnose this warning:/ }));
const send = () => fireEvent.click(within(selected()).getByRole('button', { name: words.confirm }));

it('mounts consent and evidence without replacing or rereading the conversation; a completed answer reopens', async () => {
  const initial = supplied();
  const transcript = vi.spyOn(initial.source, 'sessionTranscript');
  const h = mount(initial);
  await screen.findByRole('list', { name: 'Selected warning' });
  await waitFor(() => expect(transcript).toHaveBeenCalledOnce());
  const conversation = screen.getByRole('region', { name: 'Conversation content' });
  ask();
  expect(screen.getByTestId('diagnosis-confirm').textContent).toContain(
    SESSION_WARNINGS_TEXT.diagnose_before_sending,
  );
  fireEvent.keyDown(screen.getByRole('button', { name: words.confirm }), { key: 'Escape' });
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  ask();
  fireEvent.click(screen.getByRole('button', { name: words.not_now }));
  expect(h.controls.diagnose).not.toHaveBeenCalled();
  ask();
  send();
  expect(h.controls.diagnose).toHaveBeenCalledOnce();
  await h.answer(answers.make_skill!);
  await screen.findByTestId('diagnosis-result');
  fireEvent.click(screen.getByRole('button', { name: '[11]' }));
  await waitFor(() => expect(document.activeElement?.getAttribute('data-number')).toBe('11'));
  expect(screen.getByRole('region', { name: 'Conversation content' })).toBe(conversation);
  expect(transcript).toHaveBeenCalledOnce();
  expect(h.controls.saveSkill).not.toHaveBeenCalled();
  h.navigate(`/sessions/${session.id}?range=30d&warning=${encodeURIComponent(warning.id)}`);
  expect(screen.getByRole('region', { name: 'Conversation content' })).toBe(conversation);
  expect(transcript).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: second.name }));
  expect(screen.queryByTestId('diagnosis-result')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: warning.name }));
  await screen.findByTestId('diagnosis-result');
  expect(h.controls.diagnose).toHaveBeenCalledOnce();
  expect(transcript).toHaveBeenCalledOnce();
});

it('keeps detached Stop until the run settles and retries false or rejected cancellation', async () => {
  const h = mount();
  vi.mocked(h.controls.cancel)
    .mockResolvedValueOnce(false)
    .mockRejectedValueOnce(Error('Synthetic cancel failure'));
  await screen.findByRole('list', { name: 'Selected warning' });
  ask();
  send();
  fireEvent.click(screen.getByRole('button', { name: second.name }));
  expect(within(selected()).queryByTestId('diagnosis-running')).toBeNull();
  const stop = screen.getByRole('button', { name: words.stop });
  for (let count = 1; count <= 2; count++) {
    fireEvent.click(stop);
    await waitFor(() => expect(stop.getAttribute('aria-disabled')).toBeNull());
    expect(h.controls.cancel).toHaveBeenCalledTimes(count);
  }
  fireEvent.click(stop);
  await waitFor(() => expect(stop.getAttribute('aria-disabled')).toBe('true'));
  // Another warning of this session is shown: its run says one runs at a time,
  // the same words the Dashboard window uses, with Stop waiting once accepted.
  expect(screen.getByTestId('diagnosis-busy').textContent).toContain(words.busy);
  await h.answer(answers.cancelled!);
  expect(screen.queryByTestId('diagnosis-busy')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: warning.name }));
  await screen.findByTestId('diagnosis-result');
});

it('clears consent on selection and session changes and cancels the original source on replacement', async () => {
  const h = mount();
  await screen.findByRole('list', { name: 'Selected warning' });
  ask();
  fireEvent.click(screen.getByRole('button', { name: second.name }));
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: warning.name }));
  ask();
  h.navigate('/sessions/synthetic-other-session');
  expect(screen.queryByTestId('diagnosis-confirm')).toBeNull();
  expect(h.controls.diagnose).not.toHaveBeenCalled();
  h.navigate(`/sessions/${session.id}?warning=${encodeURIComponent(warning.id)}`);
  await screen.findByRole('list', { name: 'Selected warning' });
  ask();
  send();
  const readId = vi.mocked(h.controls.diagnose).mock.calls[0]![3];
  const next = supplied();
  h.replace(next.source);
  await screen.findByRole('list', { name: 'Selected warning' });
  expect(h.controls.cancel).toHaveBeenCalledExactlyOnceWith(readId);
  expect(next.controls.cancel).not.toHaveBeenCalled();
  await h.answer(answers.make_skill!);
  expect(screen.queryByTestId('diagnosis-result')).toBeNull();
  expect(screen.queryByTestId('diagnosis-running')).toBeNull();
});

it.each(['claude', 'codex'] as const)(
  'keeps the active %s page through diagnosis consent and completion',
  async (host) => {
    const initial = supplied();
    const readRow = initial.source.sessionRow.bind(initial.source);
    initial.source.sessionRow = async (...args) => {
      const row = await readRow(...args);
      return row ? { ...row, host } : null;
    };
    const transcript = vi.spyOn(initial.source, 'sessionTranscript').mockResolvedValue({
      state: 'unavailable',
      reason:
        host === 'claude'
          ? { reason: 'too_large', limit: 'bytes', reached: 99, ceiling: 10 }
          : { reason: 'reader_limit', limit: 'source_bytes' },
    });
    const cancel = vi.spyOn(initial.source, 'cancelSessionTranscript');
    const pageNames: string[] = [];
    const pageRead = vi.fn(
      async (
        _session: string,
        selection: SessionPageSelection,
        readId: string,
      ): Promise<SessionPageStatus> => {
        pageNames.push(readId);
        const later = selection.kind === 'cursor';
        const anchored = selection.kind === 'anchor';
        return {
          state: 'loaded',
          generation: { generation: 'unrecorded' },
          digest: 'a'.repeat(64),
          records: [
            {
              id: anchored ? selection.record_uuid : later ? 'later' : 'first',
              role: 'user',
              at: null,
              blocks: [
                {
                  kind: 'text',
                  index: anchored ? 1 : 0,
                  text: later ? 'Combined later page' : 'Combined first page',
                },
                ...(anchored && selection.block !== null
                  ? [
                      {
                        kind: 'tool_result' as const,
                        index: selection.block,
                        call_id: 'saved',
                        failed: true,
                        parts: [{ kind: 'text' as const, text: 'Combined original result' }],
                      },
                    ]
                  : []),
              ],
            },
          ],
          record_ranges: [{ start: 0, end: 1 }],
          start: later ? 1 : 0,
          end: later ? 2 : 1,
          total_records: 2,
          dropped_records: 0,
          previous: later ? 'before' : null,
          next: later ? null : 'after',
          target: anchored ? { record: 0, block: selection.block } : null,
        };
      },
    );
    const data: DataSource = initial.source;
    data.sessionTranscriptPage = pageRead;
    const h = mount(initial);
    await screen.findByText('Combined first page');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await screen.findByText('Combined later page');
    const conversation = screen.getByRole('region', { name: 'Conversation content' });
    const activeRead = pageNames.at(-1)!;
    const readsBeforeDiagnosis = pageRead.mock.calls.length;
    expect(pageRead.mock.calls.filter((call) => call[1].kind === 'anchor')).toHaveLength(1);
    ask();
    expect(screen.getByTestId('diagnosis-confirm')).toBeTruthy();
    expect(h.controls.diagnose).not.toHaveBeenCalled();
    expect(screen.getByText('Combined later page')).toBeTruthy();
    send();
    expect(h.controls.diagnose).toHaveBeenCalledOnce();
    await h.answer(answers.make_skill!);
    await screen.findByTestId('diagnosis-result');
    expect(screen.getByRole('region', { name: 'Conversation content' })).toBe(conversation);
    expect(screen.getByText('Combined later page')).toBeTruthy();
    expect(pageRead).toHaveBeenCalledTimes(readsBeforeDiagnosis);
    expect(transcript).toHaveBeenCalledOnce();
    expect(cancel.mock.calls.flat()).not.toContain(activeRead);
  },
);
