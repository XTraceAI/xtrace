import type { NativeHostState } from '../../data/generated/NativeHostState';
import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import type { ControlTone } from '../../kit/control-tone';
import type { IconName } from '../../kit/icons';
import { count } from '../../kit/format';
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
    : 'Read with the bundled reader and Python 3.10 or newer.';

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
 * a next step, never the raw error (that stays under Details), and nothing
 * when a step would only repeat the source's label (no local history, waiting
 * in a scan). Whether a state is a problem is the app's `needs_attention`,
 * never decided here.
 *
 * Python is looked for again on each later scan of a source, but a scan
 * starts only when that history changes or XTrace opens, so reopening is the
 * step that always works.
 */
const nextSteps: Record<NativeHostState, (needsAttention: boolean) => string | null> = {
  pending: (needsAttention) => (needsAttention ? 'Reopen XTrace to scan again.' : null),
  complete: () => null,
  incomplete: (needsAttention) =>
    needsAttention ? 'No session could be read. See Details for the reason.' : null,
  missing_source: () => null,
  missing_runtime: () => 'Install Python 3.10 or newer, then reopen XTrace to read this history.',
  pin_mismatch: () => 'See Details for the reason, or Settings → Native index.',
  reader_failed: () => 'See Details for the reason, or Settings → Native index.',
  cancelled: () => 'Reopen XTrace to scan again.',
};

/**
 * The sessions a read source holds: those read whole, then those only partly
 * read (the app counts the two apart, so neither includes the other).
 */
const sessionsRead = ({
  sessions_imported: whole,
  sessions_partial: partly,
}: Pick<NativeHostStatus, 'sessions_imported' | 'sessions_partial'>) =>
  partly === 0
    ? plural(whole, 'session')
    : whole === 0
      ? `${plural(partly, 'session')}, partly read`
      : `${plural(whole, 'session')}, ${count(partly)} more partly read`;

/**
 * One source's line on the welcome page: its next step, else how many
 * sessions were read, else nothing (its label says it all).
 */
export const hostSummary = (
  host: Pick<
    NativeHostStatus,
    'state' | 'needs_attention' | 'sessions_imported' | 'sessions_partial'
  >,
): string | null =>
  nextSteps[host.state](host.needs_attention) ??
  (host.state === 'complete' || host.state === 'incomplete' ? sessionsRead(host) : null);

/** The host states each check is the cause of: Python, or the bundled readers. */
const checkCauses: Record<'python' | 'readers', readonly NativeHostState[]> = {
  python: ['missing_runtime'],
  readers: ['pin_mismatch', 'reader_failed'],
};

/**
 * The colour of a missing Python or unverified readers on the quiet checks
 * line and in Details: a warning only while the app flags a source
 * (`needs_attention`) whose state that check causes, since a missing Python
 * is no problem when no history needs it.
 */
export const checkTone = (
  status: Pick<NativeIndexStatus, 'hosts'>,
  check: 'python' | 'readers',
  missing: boolean,
): 'warning' | undefined =>
  missing &&
  status.hosts.some((host) => host.needs_attention && checkCauses[check].includes(host.state))
    ? 'warning'
    : undefined;

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
