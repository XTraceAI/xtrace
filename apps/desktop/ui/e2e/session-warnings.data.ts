import type { SessionWarningItem } from '../src/data/generated/SessionWarningItem';
import type { SessionWarningsAnswer } from '../src/data/generated/SessionWarningsAnswer';

// Supplied synthetic DTOs test rendering; they do not detect or word warnings.
export const warning: SessionWarningItem = {
  id: 'correction:synthetic#0',
  kind: 'correction',
  label: 'Correction',
  at_ms: 0,
  day: 'Sep 4',
  reason: 'Synthetic correction reason (10:05)',
  compact_reason: 'Synthetic correction reason',
  name: 'Warning. Sep 4, Correction: Synthetic correction reason (10:05)',
  record_uuids: ['synthetic'],
  points_at: { record_uuid: 'synthetic', block: 0, sent_by: 'person' },
  returned: null,
};

export const second: SessionWarningItem = {
  ...warning,
  id: 'repeated_failure:other',
  kind: 'repeated_failure',
  label: 'Repeated failure',
  reason: 'Synthetic failure reason',
  name: 'Warning. Sep 4, Repeated failure: Synthetic failure reason',
  points_at: { record_uuid: 'other', block: null, sent_by: 'person' },
  returned: 'Synthetic returned warning note',
};

/** An admitted mistake: its saved point is the agent's reply, as the native answer says. */
export const admitted: SessionWarningItem = {
  ...warning,
  id: 'admitted_mistake:answer',
  kind: 'admitted_mistake',
  label: 'Admitted mistake',
  reason: 'Synthetic admission reason (10:06)',
  compact_reason: 'Synthetic admission reason',
  name: 'Warning. Sep 4, Admitted mistake: Synthetic admission reason (10:06)',
  record_uuids: ['prompt', 'answer'],
  points_at: { record_uuid: 'answer', block: null, sent_by: 'agent' },
};

/** The same kind whose saved point names a person's message: never shown as the reply. */
export const admittedOnPerson: SessionWarningItem = {
  ...admitted,
  id: 'admitted_mistake:followup',
  reason: 'Synthetic misplaced admission reason (10:07)',
  compact_reason: 'Synthetic misplaced admission reason',
  name: 'Warning. Sep 4, Admitted mistake: Synthetic misplaced admission reason (10:07)',
  record_uuids: ['followup'],
  points_at: { record_uuid: 'followup', block: null, sent_by: 'agent' },
};

export const answer: Extract<SessionWarningsAnswer, { state: 'read' }> = {
  state: 'read',
  warnings: [warning, second],
  dismissed: [],
  count_label: '2 supplied warnings',
  summary: 'Supplied summary: two warnings need review.',
  checks_summary: 'Supplied checks: one incomplete.',
  incomplete_checks: 1,
  checks: [
    { kind: 'correction', complete: true, text: 'Supplied completed check.' },
    { kind: 'repeated_failure', complete: false, text: 'Supplied incomplete check reason.' },
  ],
};

export const dismissedAnswer: typeof answer = {
  ...answer,
  warnings: [second],
  dismissed: [warning],
  count_label: '1 supplied warning',
  summary: 'Supplied summary after dismissal.',
};
