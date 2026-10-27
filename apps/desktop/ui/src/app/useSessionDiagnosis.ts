import { skipToken, useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useData, useDiagnosisRun } from '../data/DataProvider';
import type { DiagnosisControls } from '../data/DataSource';
import { keptKey, type DiagnosisRun, type Kept } from '../data/diagnosis-runs';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';

export type { Kept } from '../data/diagnosis-runs';

/**
 * What a view is doing with Diagnose: asking first (this view's own
 * question), or the app's one running diagnosis, wherever it was sent from.
 */
type Stage =
  | { readonly kind: 'confirm'; readonly sessionId: string; readonly warningId: string }
  | ({ readonly kind: 'running' } & DiagnosisRun);

export interface Diagnose {
  readonly controls: DiagnosisControls;
  readonly sessionId: string;
  readonly stage: Stage | null;
  /** The run this view sent, if it is the one running: it keeps focus on Stop. */
  readonly sentHere: string | null;
  /** Shows the sentence before sending for this warning. */
  ask(warningId: string): void;
  /** Closes the sentence before sending; nothing was sent. */
  notNow(): void;
  /** Sends: the confirmed click. */
  send(warningId: string): void;
  /** Stops the running diagnosis. */
  stop(): Promise<boolean>;
  /** Puts a warning's answer away. */
  close(warningId: string): void;
}

/**
 * Diagnose for one session's view, or `null` where no coding tool can be run
 * (the browser fixture without its test controls).
 *
 * The running diagnosis is the source runtime's (`DiagnosisRuns`), not this
 * view's: a view that closes or navigates away leaves it running, and the
 * next view to show its warning shows Stop, then its answer.
 */
export function useDiagnose(
  sessionId: string,
  timezone: string,
  warnings: readonly SessionWarningItem[] | undefined,
  selectedWarningId?: string | null,
): Diagnose | null {
  const { source, diagnosisRuns } = useData();
  const client = useQueryClient();
  const controls = source.diagnosis;
  const run = useDiagnosisRun();
  const [asked, setAsked] = useState<{ sessionId: string; warningId: string } | null>(null);
  const [sentHere, setSentHere] = useState<string | null>(null);
  const currentAsked =
    asked &&
    (asked.sessionId !== sessionId ||
      (selectedWarningId !== undefined && asked.warningId !== selectedWarningId) ||
      !warnings?.some((warning) => warning.id === asked.warningId))
      ? null
      : asked;
  useEffect(() => {
    if (asked !== currentAsked) setAsked(currentAsked);
  }, [asked, currentAsked]);
  const stage = useMemo<Stage | null>(
    () =>
      run
        ? { kind: 'running', ...run }
        : currentAsked
          ? { kind: 'confirm', ...currentAsked }
          : null,
    [run, currentAsked],
  );
  const ask = useCallback(
    (warningId: string) => {
      if (run || !warnings?.some((warning) => warning.id === warningId)) return;
      setAsked({ sessionId, warningId });
    },
    [run, sessionId, warnings],
  );
  const notNow = useCallback(() => setAsked(null), []);
  const send = useCallback(
    (warningId: string) => {
      if (!controls || run || currentAsked?.warningId !== warningId) return;
      if (!diagnosisRuns.start(sessionId, warningId, timezone)) return;
      setAsked(null);
      setSentHere(diagnosisRuns.getRun()?.readId ?? null);
    },
    [controls, run, diagnosisRuns, sessionId, timezone, currentAsked],
  );
  const stop = useCallback(() => diagnosisRuns.stop(), [diagnosisRuns]);
  // Put away, not removed: the answer's observers on the page let go of it
  // at once.
  const close = useCallback(
    (warningId: string) => client.setQueryData<Kept | null>(keptKey(sessionId, warningId), null),
    [client, sessionId],
  );
  const mine = run && run.readId === sentHere ? sentHere : null;
  return useMemo(
    () =>
      controls
        ? { controls, sessionId, stage, sentHere: mine, ask, notNow, send, stop, close }
        : null,
    [controls, sessionId, stage, mine, ask, notNow, send, stop, close],
  );
}

/** The kept answer for one warning, if any. */
export function useKept(sessionId: string, warningId: string) {
  return (
    useQuery<Kept | null>({
      queryKey: keptKey(sessionId, warningId),
      queryFn: skipToken,
      staleTime: Infinity,
    }).data ?? undefined
  );
}
