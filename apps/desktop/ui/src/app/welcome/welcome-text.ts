import type { NativeHostState } from '../../data/generated/NativeHostState';
import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import type { ControlTone } from '../../kit/control-tone';
import type { IconName } from '../../kit/icons';
import { plural } from '../dashboard/present';

/** A host's display name is the app's one name for it (the kit's `hostName`). */
export { hostName } from '../../kit/hosts';
/** Counted nouns are the app's one `plural` (numbers through the same en-US grouping). */
export { plural };

/** Hosts whose last scan needs attention, as the app decides it (`needs_attention`). */
export const attentionCount = (hosts: readonly NativeHostStatus[]) =>
  hosts.filter((host) => host.needs_attention).length;

/** How a source's history is read; Claude transcripts need no interpreter. */
export const readerNote = (host: string) =>
  host === 'claude'
    ? 'Transcripts are read directly; no Python needed.'
    : 'Read with the bundled reader and Python 3.';

export function phasePill(status: NativeIndexStatus): { label: string; tone: ControlTone } {
  switch (status.phase.phase) {
    case 'scanning':
      return { label: 'Scanning', tone: 'accent' };
    case 'ready':
      return status.needs_attention
        ? { label: 'Ready with problems', tone: 'warning' }
        : { label: 'Ready', tone: 'success' };
    case 'disabled':
      return { label: 'Not running', tone: 'meta' };
    case 'stopped':
      return { label: 'Stopped', tone: 'meta' };
  }
}

/**
 * What a first-time user can do about one source's last scan, in plain words:
 * a next step, never the raw error (that stays under Details). `null` means
 * the source was read and its session count says the rest. Whether a state is
 * a problem is the app's `needs_attention`, never decided here.
 *
 * Python is looked for again on each later scan of a source, but a scan
 * starts only when that history changes or XTrace opens, so reopening is the
 * step that always works.
 */
const nextSteps: Record<NativeHostState, (needsAttention: boolean) => string | null> = {
  pending: (needsAttention) =>
    needsAttention ? 'Not read yet. Reopen XTrace to scan again.' : 'Will be read in this scan.',
  complete: () => null,
  incomplete: (needsAttention) =>
    needsAttention ? 'History was found, but no session could be read. See Details.' : null,
  missing_source: () => 'No history from this tool on this Mac.',
  missing_runtime: () => 'Install Python 3.10 or newer, then reopen XTrace to read this history.',
  pin_mismatch: () =>
    'The reader bundled with XTrace could not be verified. See Details, or Settings → Native index.',
  reader_failed: () => 'This history could not be read. See Details, or Settings → Native index.',
  cancelled: () => 'The scan stopped before it finished. Reopen XTrace to scan again.',
};

/** One source's line on the welcome page: its next step, or how many sessions were read. */
export const hostSummary = (
  host: Pick<NativeHostStatus, 'state' | 'needs_attention' | 'sessions_imported'>,
): string =>
  nextSteps[host.state](host.needs_attention) ?? plural(host.sessions_imported, 'session');

/**
 * The pages a first-time user is pointed to, by route: each title is the
 * page's own (`pages` in routes.ts) and each icon its sidebar icon; only this
 * one-line description is added.
 */
export const welcomeTour: readonly { path: string; icon: IconName; description: string }[] = [
  {
    path: '/dashboard',
    icon: 'dashboard',
    description: 'Hours, tokens and trends across your agents.',
  },
  { path: '/sessions', icon: 'sessions', description: 'Every conversation, one row each.' },
  { path: '/scores', icon: 'scores', description: 'Grades for the projects on this Mac.' },
  { path: '/prs', icon: 'prs', description: 'The agent effort behind each pull request.' },
];
