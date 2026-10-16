import { expect, it } from 'vitest';
import fixture from '../../fixtures/F1.json';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { codexHistoryGapsText } from './native-index-text';

const base = (fixture as FixtureExport).native_index;
const none = {
  unsearched: 0,
  unfinished_line_skipped: 0,
  forks_not_searched: 0,
  folders_not_read: 0,
  too_many_files: false,
  exec_launches_without_result: 0,
};
const text = (gaps: Partial<typeof none> | null) =>
  codexHistoryGapsText({
    ...base,
    codex_history_gaps: gaps && { ...none, ...gaps },
  } satisfies NativeIndexStatus);

it('words the Codex histories the launch search skipped from the counts the app sends', () => {
  // Not looked at yet, or nothing skipped: nothing to say.
  expect(text(null)).toBeNull();
  expect(text({})).toBeNull();
  expect(text({ unsearched: 1 })).toBe(
    '1 Codex history could not be searched for sub-session launches. Conversations it may have started are shown as main conversations, not sub-sessions.',
  );
  expect(text({ unsearched: 3 })).toBe(
    '3 Codex histories could not be searched for sub-session launches. Conversations they may have started are shown as main conversations, not sub-sessions.',
  );
  expect(text({ unfinished_line_skipped: 1 })).toBe(
    '1 Codex history was read without a last line that was never finished. Conversations it may have started are shown as main conversations, not sub-sessions.',
  );
  expect(text({ unsearched: 2, unfinished_line_skipped: 2 })).toBe(
    '2 Codex histories could not be searched for sub-session launches, and 2 were read without a last line that was never finished. Conversations they may have started are shown as main conversations, not sub-sessions.',
  );
  expect(text({ folders_not_read: 1 })).toBe(
    '1 folder in the Codex history folder could not be read. Conversations started from histories in it may be shown as main conversations.',
  );
  expect(text({ folders_not_read: 2 })).toBe(
    '2 folders in the Codex history folder could not be read. Conversations started from histories in them may be shown as main conversations.',
  );
  expect(text({ too_many_files: true })).toBe(
    'The Codex history folder has too many files, so only part of it was read. Conversations started from the rest may be shown as main conversations.',
  );
  expect(text({ exec_launches_without_result: 1 })).toBe(
    '1 codex exec run started from Codex never reported back. The conversations it opened may be shown as main conversations.',
  );
  expect(text({ exec_launches_without_result: 2 })).toBe(
    '2 codex exec runs started from Codex never reported back. The conversations they opened may be shown as main conversations.',
  );
  // Forks are said apart, plainly, and never as histories that could not be searched.
  expect(text({ forks_not_searched: 11 })).toBe(
    '11 forked Codex conversations are not searched for sub-session launches, because their copied part cannot be told apart from their own. Sub-sessions started from them show as main conversations.',
  );
  expect(text({ unsearched: 1, forks_not_searched: 1 })).toBe(
    '1 Codex history could not be searched for sub-session launches. Conversations it may have started are shown as main conversations, not sub-sessions. 1 forked Codex conversation is not searched for sub-session launches, because its copied part cannot be told apart from its own. Sub-sessions started from it show as main conversations.',
  );
});
