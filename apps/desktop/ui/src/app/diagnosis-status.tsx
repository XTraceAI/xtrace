import { useEffect, useState } from 'react';
import { useDiagnosisRun } from '../data/DataProvider';
import { elapsedClock } from '../data/diagnosis-runs';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { useKept } from './useSessionDiagnosis';

const WORDS = SESSION_WARNINGS_TEXT.diagnose;

/**
 * Whether one warning is being diagnosed, or has an answer kept: the one
 * answer every warning dot (the Effort chart's and the session lanes') reads
 * for its arc, its hint and its name.
 */
export type DiagnosisStatus =
  { readonly kind: 'running'; readonly startedAt: number } | { readonly kind: 'ready' } | null;

const READY = { kind: 'ready' } as const;

export function useDiagnosisStatus(sessionId: string, warningId: string): DiagnosisStatus {
  const run = useDiagnosisRun();
  const kept = useKept(sessionId, warningId);
  if (run && run.sessionId === sessionId && run.warningId === warningId)
    return { kind: 'running', startedAt: run.startedAt };
  // A run you stopped has nothing new to read.
  return kept && !('answer' in kept && kept.answer.state === 'cancelled') ? READY : null;
}

/** A dot's accessible name with its diagnosis state after it. */
export const withDiagnosisStatus = (name: string, status: DiagnosisStatus) =>
  status === null
    ? name
    : `${name}; ${status.kind === 'running' ? WORDS.dot_running_name : WORDS.dot_ready}`;

/** `Date.now()`, once a second while `active`. */
function useNow(active: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);
  return now;
}

/**
 * A dot hint's status line: how long the diagnosis has been running, ticking
 * while shown, or that its answer is ready. Nothing for a warning without one.
 */
export function DiagnosisStatusLine({ status }: { status: DiagnosisStatus }) {
  const now = useNow(status?.kind === 'running');
  if (status === null) return null;
  return (
    <span className="xt-diagnosis-status" data-testid="diagnosis-status" data-state={status.kind}>
      {status.kind === 'running'
        ? WORDS.dot_running.replace('{elapsed}', elapsedClock(now - status.startedAt))
        : WORDS.dot_ready}
    </span>
  );
}
