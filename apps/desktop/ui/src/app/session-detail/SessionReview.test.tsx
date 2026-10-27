import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useRef, useState } from 'react';
import fixture from '../../../fixtures/F1.json';
import { answer as warned } from '../../../e2e/session-warnings.data';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { DataProvider } from '../../data/DataProvider';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import { useSessionWarning } from '../session-warnings';
import { SessionReview } from './SessionReview';

afterEach(cleanup);
const first = warned.warnings[0]!;
const second = warned.warnings[1]!;

function harness() {
  const source = new FixtureDataSource(fixture as FixtureExport);
  const answers = new Map(['A', 'B'].map((id) => [id, structuredClone(warned)]));
  let failed = false;
  const reads: string[] = [];
  const moves = vi.fn();
  const pending: {
    session: string;
    warning: string;
    undo: boolean;
    finish: (refuse?: boolean) => void;
  }[] = [];
  const change = (session: string, warning: string, undo: boolean) =>
    new Promise<import('../../data/generated/SessionWarningDismissal').SessionWarningDismissal>(
      (resolve, reject) => {
        pending.push({
          session,
          warning,
          undo,
          finish: (refuse = false) => {
            if (refuse) {
              reject(Error('refused'));
              return;
            }
            const answer = answers.get(session)!;
            const from = undo ? answer.dismissed : answer.warnings;
            const target = from.find((entry) => entry.id === warning)!;
            if (!target) throw Error('test warning missing');
            const to = undo ? answer.warnings : answer.dismissed;
            to.push(target);
            from.splice(from.indexOf(target), 1);
            resolve({ session_id: session, warning_id: warning, dismissed: !undo });
          },
        });
      },
    );
  source.sessionWarnings = async (ids, timezone) => {
    reads.push(...ids);
    if (failed) throw Error('Readback failed');
    return {
      timezone,
      sessions: ids.map((session_id) => ({
        session_id,
        answer: structuredClone(answers.get(session_id)!),
      })),
    };
  };
  Object.defineProperty(source, 'warningDismissals', {
    value: {
      dismiss: (s: string, w: string) => change(s, w, false),
      restore: (s: string, w: string) => change(s, w, true),
    },
  });
  function Review({ session }: { session: string }) {
    const query = useSessionWarning(session, 'UTC');
    const [selection, setSelection] = useState(first.id);
    const generation = useRef(0);
    return (
      <SessionReview
        sessionId={session}
        answer={query.data}
        pending={query.isPending}
        failed={query.isError}
        selection={selection}
        navigationGeneration={generation.current}
        onSelect={(id) => {
          generation.current++;
          setSelection(id);
        }}
        onReadbackSelect={(id) => {
          moves(session, id);
          setSelection(id);
        }}
      >
        <p>Conversation placeholder</p>
      </SessionReview>
    );
  }
  function Harness() {
    const [session, setSession] = useState('A');
    return (
      <DataProvider source={source}>
        <button onClick={() => setSession(session === 'A' ? 'B' : 'A')}>Change session</button>
        <span data-testid="session">{session}</span>
        <Review key={session} session={session} />
      </DataProvider>
    );
  }
  render(<Harness />);
  return {
    pending,
    reads,
    moves,
    failRead: () => {
      failed = true;
    },
  };
}
const selected = () => screen.getByRole('list', { name: 'Selected warning' });
const action = (name = 'Dismiss') => within(selected()).getByRole('button', { name });
async function settle(operation: ReturnType<typeof harness>['pending'][number], refuse = false) {
  await act(async () => operation.finish(refuse));
}
async function choose(warning: typeof first) {
  fireEvent.click(await screen.findByRole('button', { name: warning.name }));
  await waitFor(() => expect(selected().textContent).toContain(warning.reason));
}

describe('selected warning readback ownership', () => {
  it.each([false, true])(
    'an old session cannot move selection after route change (return=%s)',
    async (returnToA) => {
      const h = harness();
      await screen.findByRole('button', { name: 'Dismiss' });
      fireEvent.click(action());
      expect(h.pending).toHaveLength(1);
      fireEvent.click(screen.getByRole('button', { name: 'Change session' }));
      await waitFor(() => expect(screen.getByTestId('session').textContent).toBe('B'));
      if (returnToA) {
        fireEvent.click(screen.getByRole('button', { name: 'Change session' }));
        await waitFor(() => expect(screen.getByTestId('session').textContent).toBe('A'));
      }
      await choose(second);
      const focus = document.activeElement;
      await settle(h.pending[0]!);
      await waitFor(() => expect(h.reads.filter((id) => id === 'A').length).toBeGreaterThan(1));
      expect(h.moves).not.toHaveBeenCalled();
      expect(selected().textContent).toContain(second.reason);
      expect(document.activeElement).toBe(focus);
    },
  );
  it.each([false, true])(
    'explicit warning navigation wins over an old dismissal (return=%s)',
    async (returnToFirst) => {
      const h = harness();
      await screen.findByRole('button', { name: 'Dismiss' });
      fireEvent.click(action());
      await choose(second);
      if (returnToFirst) await choose(first);
      const focus = document.activeElement;
      await settle(h.pending[0]!);
      await waitFor(() => expect(h.reads.length).toBeGreaterThan(1));
      expect(h.moves).not.toHaveBeenCalled();
      expect(document.activeElement).toBe(focus);
      if (!returnToFirst) {
        await choose(first);
        expect(action('Undo').getAttribute('aria-disabled')).not.toBe('true');
      }
    },
  );
  it('normal Dismiss and Undo move only after readback and focus the new action', async () => {
    const h = harness();
    await screen.findByRole('button', { name: 'Dismiss' });
    fireEvent.click(action());
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
    await settle(h.pending[0]!);
    await waitFor(() => expect(document.activeElement).toBe(action('Undo')));
    fireEvent.click(action('Undo'));
    await settle(h.pending[1]!);
    await waitFor(() => expect(document.activeElement).toBe(action()));
    expect(h.moves).toHaveBeenCalledTimes(2);
  });
  it('a refused dismissal leaves the selected warning with a usable retry', async () => {
    const h = harness();
    await screen.findByRole('button', { name: 'Dismiss' });
    fireEvent.click(action());
    await settle(h.pending[0]!, true);
    await screen.findByRole('alert');
    expect(h.moves).not.toHaveBeenCalled();
    expect(action().getAttribute('aria-disabled')).not.toBe('true');
  });
  it('two pending warnings settle independently without reviving a superseded selection', async () => {
    const h = harness();
    await screen.findByRole('button', { name: 'Dismiss' });
    fireEvent.click(action());
    await choose(second);
    fireEvent.click(action());
    await settle(h.pending[0]!);
    expect(h.moves).not.toHaveBeenCalled();
    await settle(h.pending[1]!);
    await waitFor(() => expect(document.activeElement).toBe(action('Undo')));
    expect(h.moves.mock.calls).toEqual([['A', second.id]]);
    await choose(first);
    expect(action('Undo').getAttribute('aria-disabled')).not.toBe('true');
  });
  it('a failed readback never invents a move', async () => {
    const h = harness();
    await screen.findByRole('button', { name: 'Dismiss' });
    fireEvent.click(action());
    const focus = document.activeElement;
    h.failRead();
    await settle(h.pending[0]!);
    await waitFor(() => expect(action().getAttribute('aria-disabled')).not.toBe('true'));
    expect(h.moves).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(focus);
  });
});
