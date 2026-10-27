import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DiagnosisControls } from '../data/DataSource';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { SessionDiagnosis } from '../data/generated/SessionDiagnosis';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import { useDiagnose, useKept, type Diagnose } from './useSessionDiagnosis';

afterEach(cleanup);
const warning: SessionWarningItem = {
  id: 'correction:record-a',
  kind: 'correction',
  label: 'Correction',
  at_ms: 100,
  day: 'Oct 27',
  reason: 'Synthetic correction',
  compact_reason: 'Synthetic correction',
  name: 'Synthetic warning',
  record_uuids: ['record-a'],
  points_at: { record_uuid: 'record-a', block: null },
  returned: null,
};
const warnings = [warning];
const answer: SessionDiagnosis = { state: 'cancelled', text: 'Synthetic cancelled answer' };
function pending<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function sourceWithControls() {
  const source = new FixtureDataSource(fixture as unknown as FixtureExport);
  const runs: ReturnType<typeof pending<SessionDiagnosis>>[] = [];
  const controls: DiagnosisControls = {
    diagnose: vi.fn(() => {
      const run = pending<SessionDiagnosis>();
      runs.push(run);
      return run.promise;
    }),
    cancel: vi.fn(async () => true),
    saveSkill: vi.fn(async () => ({
      state: 'saved' as const,
      path: 'synthetic/SKILL.md',
      text: 'Saved',
    })),
  };
  Object.defineProperty(source, 'diagnosis', { value: controls });
  return { source, controls, runs };
}
function harness(initial = sourceWithControls()) {
  let diagnose: Diagnose | null = null;
  let kept: ReturnType<typeof useKept>;
  let client!: QueryClient;
  type Props = {
    session?: string;
    selected?: string | null;
    items?: readonly SessionWarningItem[];
    visible?: boolean;
    source?: FixtureDataSource;
  };
  function Probe({ session = 'session-a', selected, items = warnings }: Props) {
    diagnose = useDiagnose(session, 'America/Los_Angeles', items, selected);
    kept = useKept(session, warning.id);
    client = useQueryClient();
    return <output>{diagnose?.stage?.kind ?? 'idle'}</output>;
  }
  function App(props: Props) {
    return (
      <DataProvider source={props.source ?? initial.source}>
        {props.visible !== false && <Probe {...props} />}
      </DataProvider>
    );
  }
  const view = render(<App />);
  const ready = () => waitFor(() => expect(diagnose).not.toBeNull());
  const ask = () => act(() => diagnose!.ask(warning.id));
  const send = () => act(() => diagnose!.send(warning.id));
  return {
    ...initial,
    view,
    ready,
    ask,
    send,
    rerender: (props: Props) => view.rerender(<App {...props} />),
    get diagnose() {
      return diagnose;
    },
    get kept() {
      return kept;
    },
    get client() {
      return client;
    },
  };
}

it('starts no run or save on mount, selection, ask, or Not now; sending requires confirmation', async () => {
  const h = harness();
  await h.ready();
  h.send();
  h.ask();
  expect(h.diagnose?.stage?.kind).toBe('confirm');
  act(() => h.diagnose!.notNow());
  expect(h.diagnose?.stage).toBeNull();
  h.send();
  act(() => h.diagnose!.ask('missing-warning'));
  expect(h.diagnose?.stage).toBeNull();
  expect(h.controls.diagnose).not.toHaveBeenCalled();
  expect(h.controls.saveSkill).not.toHaveBeenCalled();
});

it('runs one at a time and passes the exact session, warning, timezone and unique run ID', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  const first = h.diagnose!.stage!;
  expect(first.kind).toBe('running');
  if (first.kind !== 'running') throw new Error('Expected running stage');
  expect(h.controls.diagnose).toHaveBeenCalledExactlyOnceWith(
    'session-a',
    warning.id,
    'America/Los_Angeles',
    first.readId,
  );
  h.ask();
  h.send();
  expect(h.controls.diagnose).toHaveBeenCalledOnce();
  await act(async () => h.runs[0]!.resolve(answer));
  await waitFor(() => expect(h.kept).toEqual({ readId: first.readId, answer }));
  expect(h.diagnose!.stage).toBeNull();
  h.ask();
  h.send();
  expect(h.controls.diagnose).toHaveBeenCalledTimes(2);
  expect(h.diagnose!.stage).not.toEqual(first);
});

it('cancels a queued run by its own ID and allows retry after a rejected cancellation', async () => {
  const h = harness();
  vi.mocked(h.controls.cancel).mockRejectedValueOnce(new Error('Synthetic failure'));
  await h.ready();
  h.ask();
  h.send();
  const id = vi.mocked(h.controls.diagnose).mock.calls[0]![3];
  await act(async () => expect(await h.diagnose!.stop()).toBe(false));
  await act(async () => expect(await h.diagnose!.stop()).toBe(true));
  expect(vi.mocked(h.controls.cancel).mock.calls).toEqual([[id], [id]]);
  await act(async () => h.runs[0]!.resolve(answer));
  await waitFor(() => expect(h.kept).toMatchObject({ answer }));
  expect(await h.diagnose!.stop()).toBe(false);
});

it.each(['selection', 'session', 'removed'] as const)(
  'drops a confirmation after %s changes',
  async (change) => {
    const h = harness();
    await h.ready();
    h.ask();
    h.rerender(
      change === 'selection'
        ? { selected: null }
        : change === 'session'
          ? { session: 'session-b' }
          : { items: [] },
    );
    expect(h.diagnose!.stage).toBeNull();
    h.send();
    expect(h.controls.diagnose).not.toHaveBeenCalled();
  },
);

it('keeps Stop for the original session when selection, warnings and session change during a run', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  const stage = h.diagnose!.stage;
  h.rerender({ session: 'session-b', selected: null, items: [] });
  expect(h.diagnose!.stage).toEqual(stage);
  await act(async () => h.diagnose!.stop());
  await act(async () => h.runs[0]!.resolve(answer));
  expect(h.kept).toBeUndefined();
  h.rerender({});
  await waitFor(() => expect(h.kept).toMatchObject({ answer }));
});

it('keeps completed answers through ordinary data invalidation and a page remount in the same source', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  await act(async () => h.runs[0]!.resolve(answer));
  await waitFor(() => expect(h.kept).toMatchObject({ answer }));
  await act(async () => h.client.invalidateQueries({ queryKey: ['sessions'] }));
  h.rerender({ visible: false });
  h.rerender({ visible: true });
  await waitFor(() => expect(h.kept).toMatchObject({ answer }));
  expect(h.controls.diagnose).toHaveBeenCalledOnce();
  expect(h.controls.cancel).not.toHaveBeenCalled();
  act(() => h.diagnose!.close(warning.id));
  await waitFor(() => expect(h.kept).toBeUndefined());
});

it('cancels through the original controls once on source replacement and drops its late answer', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  const oldClient = h.client;
  const next = sourceWithControls();
  h.rerender({ source: next.source });
  await h.ready();
  const id = vi.mocked(h.controls.diagnose).mock.calls[0]![3];
  expect(h.controls.cancel).toHaveBeenCalledExactlyOnceWith(id);
  expect(next.controls.cancel).not.toHaveBeenCalled();
  expect(h.client).not.toBe(oldClient);
  expect(h.diagnose!.stage).toBeNull();
  await act(async () => h.runs[0]!.resolve(answer));
  expect(h.kept).toBeUndefined();
  expect(oldClient.getQueryData(['session-diagnosis', 'session-a', warning.id])).toBeUndefined();
});

it('drops completed answers when the app source is replaced', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  await act(async () => h.runs[0]!.resolve(answer));
  await waitFor(() => expect(h.kept).toMatchObject({ answer }));
  h.rerender({ source: sourceWithControls().source });
  await h.ready();
  expect(h.kept).toBeUndefined();
});

it('cancels on page unmount and refuses a late answer even if the page returns to the same source', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  h.rerender({ visible: false });
  expect(h.controls.cancel).toHaveBeenCalledOnce();
  h.rerender({ visible: true });
  await act(async () => h.runs[0]!.resolve(answer));
  expect(h.kept).toBeUndefined();
  expect(h.diagnose!.stage).toBeNull();
});

it('keeps an invoke failure as a failed answer and never saves automatically', async () => {
  const h = harness();
  await h.ready();
  h.ask();
  h.send();
  await act(async () => h.runs[0]!.reject(new Error('Synthetic run failure')));
  await waitFor(() => expect(h.kept).toMatchObject({ failed: true }));
  expect(h.diagnose!.stage).toBeNull();
  expect(h.controls.saveSkill).not.toHaveBeenCalled();
});

it('offers no controller in an ordinary fixture without supplied controls', async () => {
  const h = harness();
  await h.ready();
  h.rerender({ source: new FixtureDataSource(fixture as unknown as FixtureExport) });
  await waitFor(() => expect(h.diagnose).toBeNull());
});
