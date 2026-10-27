import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { createDataSource } from './createDataSource';
import { FixtureDataSource } from './FixtureDataSource';
import { TauriDataSource } from './TauriDataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { SessionExcerptStatus } from './generated/SessionExcerptStatus';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: vi.fn() }));
afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllEnvs();
});

it.each([null, 150])(
  'reads the exact native warning point with block %s and cancels its own read',
  async (block) => {
    const source = new TauriDataSource();
    const answer: SessionExcerptStatus = {
      state: 'unavailable',
      reason: { reason: 'target_ambiguous' },
    };
    vi.mocked(invoke).mockResolvedValueOnce(answer).mockResolvedValueOnce(undefined);
    expect(invoke).not.toHaveBeenCalled();
    expect(
      await source.sessionWarningContext('session-a', 'record-a', block, 'read-a', {
        id: 'correction:record-a',
        timezone: 'Asia/Tokyo',
      }),
    ).toBe(answer);
    // The warning's ID and zone go with it: the native side looks it up and
    // marks a correction.
    expect(invoke).toHaveBeenNthCalledWith(1, 'session_warning_context', {
      sessionId: 'session-a',
      recordUuid: 'record-a',
      block,
      readId: 'read-a',
      warningId: 'correction:record-a',
      timezone: 'Asia/Tokyo',
    });
    await source.cancelSessionTranscript('read-a');
    expect(invoke).toHaveBeenNthCalledWith(2, 'cancel_session_transcript', { readId: 'read-a' });
  },
);

it('keeps ordinary fixtures offline and adds no dismissal controls', async () => {
  const source = new FixtureDataSource(fixture as unknown as FixtureExport);
  expect(await source.sessionWarningContext()).toEqual({
    state: 'unavailable',
    reason: { reason: 'source', source: { reason: 'not_indexed' } },
  });
  expect('warningDismissals' in source).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});

it('keeps browser preview unavailable without invoking native code', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('DEV', false);
  const source = await createDataSource();
  await expect(
    source.sessionWarningContext('session-a', 'record-a', null, 'read-a'),
  ).rejects.toThrow('Native data is unavailable');
  expect(invoke).not.toHaveBeenCalled();
});
