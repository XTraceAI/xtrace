import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { createDataSource } from './createDataSource';
import { FixtureDataSource } from './FixtureDataSource';
import { TauriDataSource } from './TauriDataSource';
import type { DataSource } from './DataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { SessionDiagnosis } from './generated/SessionDiagnosis';
import type { SavedSkill } from './generated/SavedSkill';
import { SESSION_WARNINGS_TEXT } from './generated/SessionWarningsText';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: vi.fn() }));
afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllEnvs();
});

it('passes only exact native run and queued-cancel arguments and returns native answers unchanged', async () => {
  const source = new TauriDataSource();
  expect(invoke).not.toHaveBeenCalled();
  const answer: SessionDiagnosis = { state: 'busy', text: 'Synthetic busy answer' };
  vi.mocked(invoke).mockResolvedValueOnce(answer).mockResolvedValueOnce(true);
  expect(
    await source.diagnosis.diagnose(
      'session-a',
      'correction:record-a',
      'Pacific/Honolulu',
      'run-a',
    ),
  ).toBe(answer);
  expect(invoke).toHaveBeenNthCalledWith(1, 'diagnose_session_warning', {
    sessionId: 'session-a',
    warningId: 'correction:record-a',
    timezone: 'Pacific/Honolulu',
    readId: 'run-a',
  });
  expect(await source.diagnosis.cancel('run-a')).toBe(true);
  expect(invoke).toHaveBeenNthCalledWith(2, 'cancel_session_diagnosis', { readId: 'run-a' });
});

it('saves the complete draft only on the explicit method, with no path argument or truncation', async () => {
  const source = new TauriDataSource();
  const draft =
    '---\nname: synthetic-skill\ndescription: Synthetic draft\n---\n\n' +
    'Full body.\n'.repeat(2000);
  const saved: SavedSkill = { state: 'saved', path: 'synthetic/SKILL.md', text: 'Synthetic saved' };
  vi.mocked(invoke).mockResolvedValue(saved);
  expect(invoke).not.toHaveBeenCalled();
  expect(await source.diagnosis.saveSkill('session-a', 'synthetic-skill', draft)).toBe(saved);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('save_suggested_skill', {
    sessionId: 'session-a',
    name: 'synthetic-skill',
    draft,
  });
});

it('leaves ordinary fixture controls absent and uses the native confirmation wording', () => {
  const source: DataSource = new FixtureDataSource(fixture as unknown as FixtureExport);
  expect(source.diagnosis).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
  expect(SESSION_WARNINGS_TEXT.diagnose_before_sending).toBe(
    "About 20 messages around this warning (at most 24,000 characters) will be sent to Claude Code's hosted model, or Codex's if Claude Code is not available.",
  );
});

it('offers no native Diagnose controls in browser preview', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('DEV', false);
  const source = await createDataSource();
  expect(source.kind).toBe('preview');
  expect(source.diagnosis).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
});
