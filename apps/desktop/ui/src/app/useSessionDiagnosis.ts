import { skipToken, useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useData } from '../data/DataProvider';
import type { DiagnosisControls } from '../data/DataSource';
import type { SessionDiagnosis } from '../data/generated/SessionDiagnosis';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';

/** What the page is doing with Diagnose: asking first, or running one. */
type Stage = { readonly sessionId: string } & (
  | { readonly kind: 'confirm'; readonly warningId: string }
  | { readonly kind: 'running'; readonly warningId: string; readonly readId: string }
);

/** A kept answer, or that the diagnosis could not be asked for at all. */
export type Kept = { readonly readId: string } & (
  { readonly answer: SessionDiagnosis } | { readonly failed: true }
);

const keptKey = (sessionId: string, warningId: string) =>
  ['session-diagnosis', sessionId, warningId] as const;

let counter = 0;
/** A name for one run, unique in this window. */
function nextReadId(): string {
  counter += 1;
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? `diagnose-${crypto.randomUUID()}`
    : `diagnose-${Date.now()}-${counter}`;
}

export interface Diagnose {
  readonly controls: DiagnosisControls;
  readonly sessionId: string;
  readonly stage: Stage | null;
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
 * Diagnose for one session's page, or `null` where no coding tool can be run
 * (the browser fixture without its test controls).
 */
export function useDiagnose(
  sessionId: string,
  timezone: string,
  warnings: readonly SessionWarningItem[] | undefined,
  selectedWarningId?: string | null,
): Diagnose | null {
  const { source } = useData();
  const client = useQueryClient();
  const controls = source.diagnosis;
  const [stage, setStage] = useState<Stage | null>(null);
  const running = useRef<{
    readId: string;
    controls: DiagnosisControls;
    disposed: boolean;
  } | null>(null);
  useEffect(
    () => () => {
      const run = running.current;
      if (!run) return;
      run.disposed = true;
      running.current = null;
      // A view that disappears cannot keep Stop on screen. Stop its own run;
      // a disposed answer must not reach a later view through the cache.
      void run.controls.cancel(run.readId).catch(() => false);
    },
    [],
  );
  const currentStage =
    stage?.kind === 'confirm' &&
    (stage.sessionId !== sessionId ||
      (selectedWarningId !== undefined && stage.warningId !== selectedWarningId) ||
      !warnings?.some((warning) => warning.id === stage.warningId))
      ? null
      : stage;
  useEffect(() => {
    if (stage !== currentStage) setStage(currentStage);
  }, [stage, currentStage]);
  const ask = useCallback(
    (warningId: string) => {
      if (running.current || !warnings?.some((warning) => warning.id === warningId)) return;
      setStage({ kind: 'confirm', sessionId, warningId });
    },
    [sessionId, warnings],
  );
  const notNow = useCallback(
    () => setStage((current) => (current?.kind === 'confirm' ? null : current)),
    [],
  );
  const send = useCallback(
    (warningId: string) => {
      if (
        !controls ||
        running.current ||
        currentStage?.kind !== 'confirm' ||
        currentStage.warningId !== warningId
      )
        return;
      const readId = nextReadId();
      const run = { readId, controls, disposed: false };
      running.current = run;
      setStage({ kind: 'running', sessionId, warningId, readId });
      controls
        .diagnose(sessionId, warningId, timezone, readId)
        .then(
          (answer): Kept => ({ readId, answer }),
          (): Kept => ({ readId, failed: true }),
        )
        .then((kept) => {
          if (run.disposed) return;
          client.setQueryData(keptKey(sessionId, warningId), kept);
          running.current = null;
          setStage((current) =>
            current?.kind === 'running' && current.readId === readId ? null : current,
          );
        });
    },
    [client, controls, sessionId, timezone, currentStage],
  );
  const stop = useCallback(() => {
    const run = running.current;
    return run ? run.controls.cancel(run.readId).catch(() => false) : Promise.resolve(false);
  }, []);
  // Put away, not removed: the answer's observers on the page let go of it
  // at once.
  const close = useCallback(
    (warningId: string) => client.setQueryData<Kept | null>(keptKey(sessionId, warningId), null),
    [client, sessionId],
  );
  return useMemo(
    () =>
      controls
        ? { controls, sessionId, stage: currentStage, ask, notNow, send, stop, close }
        : null,
    [controls, sessionId, currentStage, ask, notNow, send, stop, close],
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
