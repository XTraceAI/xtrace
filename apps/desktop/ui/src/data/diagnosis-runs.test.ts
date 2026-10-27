import { QueryClient } from '@tanstack/react-query';
import { expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import type { DiagnosisControls } from './DataSource';
import { createDiagnosisRuns, elapsedClock, keptKey } from './diagnosis-runs';
import { FixtureDataSource } from './FixtureDataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { SessionDiagnosis } from './generated/SessionDiagnosis';

const answer: SessionDiagnosis = { state: 'cancelled', text: 'Synthetic cancelled answer' };

function setup() {
  const source = new FixtureDataSource(fixture as unknown as FixtureExport);
  let finish: (value: SessionDiagnosis) => void = () => {};
  const controls: DiagnosisControls = {
    diagnose: vi.fn(
      () =>
        new Promise<SessionDiagnosis>((resolve) => {
          finish = resolve;
        }),
    ),
    cancel: vi.fn(async () => true),
    saveSkill: vi.fn(),
  };
  Object.defineProperty(source, 'diagnosis', { value: controls });
  const client = new QueryClient();
  const runs = createDiagnosisRuns(source, client);
  runs.attach();
  return { runs, client, controls, finish: (value: SessionDiagnosis) => finish(value) };
}

it('writes elapsed time as m:ss', () => {
  expect(elapsedClock(0)).toBe('0:00');
  expect(elapsedClock(9_999)).toBe('0:09');
  expect(elapsedClock(62_000)).toBe('1:02');
  expect(elapsedClock(-5)).toBe('0:00');
});

it('runs one at a time, tells listeners, and keeps the answer under its warning', async () => {
  const h = setup();
  const heard = vi.fn();
  h.runs.subscribe(heard);
  expect(h.runs.start('session-a', 'warning-a', 'UTC')).toBe(true);
  expect(h.runs.start('session-b', 'warning-b', 'UTC')).toBe(false);
  expect(h.controls.diagnose).toHaveBeenCalledOnce();
  const run = h.runs.getRun()!;
  expect(run).toMatchObject({ sessionId: 'session-a', warningId: 'warning-a' });
  expect(typeof run.startedAt).toBe('number');
  h.finish(answer);
  await vi.waitFor(() => expect(h.runs.getRun()).toBeNull());
  expect(heard).toHaveBeenCalledTimes(2);
  expect(h.client.getQueryData(keptKey('session-a', 'warning-a'))).toEqual({
    readId: run.readId,
    answer,
  });
});

it('stops through the controls it was sent with only when its runtime ends, and drops the late answer', async () => {
  const h = setup();
  h.runs.start('session-a', 'warning-a', 'UTC');
  const { readId } = h.runs.getRun()!;
  expect(h.controls.cancel).not.toHaveBeenCalled();
  h.runs.detach();
  expect(h.controls.cancel).toHaveBeenCalledExactlyOnceWith(readId);
  expect(h.runs.getRun()).toBeNull();
  h.finish(answer);
  await Promise.resolve();
  await Promise.resolve();
  expect(h.client.getQueryData(keptKey('session-a', 'warning-a'))).toBeUndefined();
});
