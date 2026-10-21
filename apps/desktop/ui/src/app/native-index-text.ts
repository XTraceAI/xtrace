import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';

export const phaseText = (status: NativeIndexStatus) => {
  const { phase } = status;
  switch (phase.phase) {
    case 'disabled':
      return `Disabled: ${phase.reason}`;
    case 'scanning':
      return `Scanning (${status.files_scanned} Claude transcripts read)`;
    case 'ready':
      return `Ready (${status.reconciles} reconciliations)`;
    case 'stopped':
      return 'Stopped';
  }
};
export const freshnessText = (status: NativeIndexStatus) => {
  const { freshness } = status;
  switch (freshness.freshness) {
    case 'unknown':
      return 'Not watching yet';
    case 'live':
      return 'Live: new activity is added as it happens';
    case 'degraded':
      return `Degraded: ${freshness.reason}`;
  }
};

const plural = (count: number, one: string, many: string) => (count === 1 ? one : many);

/**
 * The Codex histories the app's search for sub-session launches skipped, in
 * words, or `null` when it skipped none (or has not looked at them yet). The
 * counts are the app's (`codex_history_gaps`); only the wording is decided here.
 * Each sentence says what was skipped and what follows from it.
 */
export const codexHistoryGapsText = (status: NativeIndexStatus): string | null => {
  const gaps = status.codex_history_gaps;
  if (!gaps) return null;
  const {
    unsearched,
    unfinished_line_skipped: unfinished,
    forks_not_searched: forks,
    folders_not_read: folders,
    too_many_files: tooMany,
    exec_launches_without_result: runs,
  } = gaps;
  const sentences: string[] = [];
  if (unsearched + unfinished > 0) {
    const parts: string[] = [];
    if (unsearched > 0)
      parts.push(
        `${unsearched} Codex ${plural(unsearched, 'history', 'histories')} could not be searched for sub-session launches`,
      );
    if (unfinished > 0)
      parts.push(
        `${unfinished}${unsearched > 0 ? '' : ` Codex ${plural(unfinished, 'history', 'histories')}`} ${plural(unfinished, 'was', 'were')} read without a last line that was never finished`,
      );
    sentences.push(
      `${parts.join(', and ')}. Conversations ${plural(unsearched + unfinished, 'it', 'they')} may have started are shown as main conversations, not sub-sessions.`,
    );
  }
  if (folders > 0)
    sentences.push(
      `${folders} ${plural(folders, 'folder', 'folders')} in the Codex history folder could not be read. Conversations started from histories in ${plural(folders, 'it', 'them')} may be shown as main conversations.`,
    );
  if (tooMany)
    sentences.push(
      'The Codex history folder has too many files, so only part of it was read. Conversations started from the rest may be shown as main conversations.',
    );
  if (runs > 0)
    sentences.push(
      `${runs} codex exec ${plural(runs, 'run', 'runs')} started from Codex never reported back. The conversations ${plural(runs, 'it', 'they')} opened may be shown as main conversations.`,
    );
  if (forks > 0)
    sentences.push(
      `${forks} forked Codex ${plural(forks, 'conversation is', 'conversations are')} not searched for sub-session launches, because ${plural(forks, 'its', 'their')} copied part cannot be told apart from ${plural(forks, 'its', 'their')} own. Sub-sessions started from ${plural(forks, 'it', 'them')} show as main conversations.`,
    );
  return sentences.length === 0 ? null : sentences.join(' ');
};
