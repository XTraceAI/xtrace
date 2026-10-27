import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { warning } from '../../../e2e/session-warnings.data';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { SessionExcerptStatus } from '../../data/generated/SessionExcerptStatus';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { WarningContext } from './WarningContext';
import { contextExcerpt, contextFixture } from './warning-context.synthetic';

afterEach(cleanup);
const point = { ...warning, points_at: { record_uuid: 'target', block: null } };
function pending() {
  let resolve!: (value: SessionExcerptStatus) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<SessionExcerptStatus>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function supplied() {
  const source = new FixtureDataSource(fixture as unknown as FixtureExport);
  const calls: ReturnType<typeof pending>[] = [];
  const read = vi.fn<DataSource['sessionWarningContext']>(() => {
    const call = pending();
    calls.push(call);
    return call.promise;
  });
  Object.defineProperty(source, 'sessionWarningContext', { value: read });
  const cancel = vi.spyOn(source, 'cancelSessionTranscript').mockResolvedValue();
  return { source, read, calls, cancel };
}
function mount(initial = supplied()) {
  type Props = {
    source?: FixtureDataSource;
    record?: string;
    block?: number | null;
    shown?: boolean;
  };
  function App({ source = initial.source, record = 'target', block = null, shown = true }: Props) {
    return (
      <DataProvider source={source}>
        {shown && (
          <WarningContext
            sessionId="session-a"
            warning={{ ...point, points_at: { record_uuid: record, block } }}
            timezone="UTC"
          />
        )}
      </DataProvider>
    );
  }
  const view = render(<App />);
  return { ...initial, view, change: (props: Props) => view.rerender(<App {...props} />) };
}
const excerpt = () => contextExcerpt(contextFixture('target'), 'target');

it('reads the exact saved point once, and shows only the native bounded records', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  expect(h.read).toHaveBeenCalledWith('session-a', 'target', null, expect.any(String), {
    id: point.id,
    timezone: 'UTC',
  });
  await act(async () => h.calls[0]!.resolve(excerpt()));
  // The warning's own message is tagged with its kind, once.
  expect(screen.getByTestId('warning-context-flag').textContent).toBe(
    SESSION_WARNINGS_TEXT.message_tags[point.kind],
  );
  expect(screen.getByText(/Still not fixed/)).toBeTruthy();
  expect(screen.queryByText(/unrelated message/)).toBeNull();
  expect(screen.getByTestId('warning-context').querySelectorAll('[data-record-id]')).toHaveLength(
    3,
  );
  h.change({});
  expect(h.read).toHaveBeenCalledOnce();
});

it('cancels a replaced point and drops its late text while the new point is loading', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  const id = h.read.mock.calls[0]![3];
  h.change({ record: 'new-target', block: 150 });
  expect(h.cancel).toHaveBeenCalledExactlyOnceWith(id);
  await waitFor(() => expect(h.read).toHaveBeenCalledTimes(2));
  expect(h.read).toHaveBeenLastCalledWith('session-a', 'new-target', 150, expect.any(String), {
    id: point.id,
    timezone: 'UTC',
  });
  await act(async () => h.calls[0]!.resolve(excerpt()));
  expect(screen.queryByText(/Still not fixed/)).toBeNull();
  expect(screen.getByRole('status').textContent).toBe('Loading the original conversation…');
});

it('cancels through the original source on replacement and drops text after close', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  const next = supplied();
  h.change({ source: next.source });
  await waitFor(() => expect(next.read).toHaveBeenCalledOnce());
  expect(h.cancel).toHaveBeenCalledExactlyOnceWith(h.read.mock.calls[0]![3]);
  expect(next.cancel).not.toHaveBeenCalled();
  await act(async () => h.calls[0]!.resolve(excerpt()));
  expect(screen.queryByText(/Still not fixed/)).toBeNull();
  h.change({ source: next.source, shown: false });
  expect(next.cancel).toHaveBeenCalledExactlyOnceWith(next.read.mock.calls[0]![3]);
  await act(async () => next.calls[0]!.resolve(excerpt()));
  expect(screen.queryByTestId('warning-context')).toBeNull();
});

it('retries a failed read with a new ID and clears the old failure', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  await act(async () => h.calls[0]!.reject(Error('Synthetic read failure')));
  expect(screen.getByRole('status').textContent).toBe(
    'The conversation could not be read. Try again.',
  );
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await waitFor(() => expect(h.read).toHaveBeenCalledTimes(2));
  expect(h.read.mock.calls[1]![3]).not.toBe(h.read.mock.calls[0]![3]);
  await act(async () => h.calls[1]!.resolve(excerpt()));
  expect(screen.getByText(/Still not fixed/)).toBeTruthy();
});

it('shows omitted blocks under the native "You" caption and never invents a person from text', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  const status = excerpt();
  if (status.state !== 'loaded') throw Error('Missing fixture');
  status.records = [{ ...status.records[1]!, omitted_blocks: true, only_tool_results: false }];
  await act(async () => h.calls[0]!.resolve(status));
  // What was saved under the user role is captioned with the native word for
  // it, never a name read from the text.
  expect(screen.getByText(SESSION_WARNINGS_TEXT.you)).toBeTruthy();
  expect(screen.queryByText('Person')).toBeNull();
  expect(
    screen.getByText('Other blocks in this message are not shown in this excerpt.'),
  ).toBeTruthy();
});

it('shows the exact generation note and unavailable-history sentence without rendering records', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  const status = excerpt();
  if (status.state !== 'loaded') throw Error('Missing fixture');
  status.generation = { generation: 'unrecorded' };
  await act(async () => h.calls[0]!.resolve(status));
  expect(
    screen.getByText(
      'This text is not tied to a measurement: at least one of this session’s files has no recorded reading.',
    ),
  ).toBeTruthy();
  h.change({ record: 'unsupported' });
  await waitFor(() => expect(h.read).toHaveBeenCalledTimes(2));
  await act(async () =>
    h.calls[1]!.resolve({
      state: 'unavailable',
      reason: { reason: 'source', source: { reason: 'unsupported_history' } },
    }),
  );
  expect(screen.getByRole('status').textContent).toMatch(/history format/);
  expect(screen.getByTestId('warning-context').querySelectorAll('[data-record-id]')).toHaveLength(
    0,
  );
});

it('says "You" only for a message M-02 counts as a person\'s, and the neutral role caption otherwise', async () => {
  const h = mount();
  await waitFor(() => expect(h.read).toHaveBeenCalledOnce());
  const status = excerpt();
  if (status.state !== 'loaded') throw Error('Missing fixture');
  // A sub-agent prompt, a reminder or command output is saved under the user
  // role too: without M-02's yes it is not "You".
  status.records = status.records.map((entry) => ({ ...entry, person: null }));
  await act(async () => h.calls[0]!.resolve(status));
  expect(screen.queryByText(SESSION_WARNINGS_TEXT.you)).toBeNull();
  const target = screen
    .getByTestId('warning-context')
    .querySelector<HTMLElement>('[data-record-id="target"]')!;
  expect(target.dataset.own).toBeUndefined();
  expect(target.querySelector('.xt-chat-who')!.textContent).toContain('User');
  expect(target.querySelector('.xt-chat-bubble')).toBeNull();
});
