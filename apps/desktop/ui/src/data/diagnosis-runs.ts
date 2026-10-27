import type { QueryClient } from '@tanstack/react-query';
import type { DataSource, DiagnosisControls } from './DataSource';
import type { SessionDiagnosis } from './generated/SessionDiagnosis';

/** The one diagnosis running in this app, and when it was sent. */
export type DiagnosisRun = {
  readonly sessionId: string;
  readonly warningId: string;
  readonly readId: string;
  /** When it was sent, in this window's clock (`Date.now()`). */
  readonly startedAt: number;
};

/** A kept answer, or that the diagnosis could not be asked for at all. */
export type Kept = { readonly readId: string } & (
  { readonly answer: SessionDiagnosis } | { readonly failed: true }
);

/** Where one warning's kept answer lives in the query cache. */
export const keptKey = (sessionId: string, warningId: string) =>
  ['session-diagnosis', sessionId, warningId] as const;

export interface DiagnosisRuns {
  getRun(): DiagnosisRun | null;
  subscribe(listener: () => void): () => void;
  /**
   * Sends one warning's diagnosis; false, and nothing is sent, while one is
   * running or where no coding tool can be run.
   */
  start(sessionId: string, warningId: string, timezone: string): boolean;
  /** Asks the running diagnosis to stop; false when none runs or it was refused. */
  stop(): Promise<boolean>;
  /** While detached (after its runtime unmounts), late answers change nothing. */
  attach(): void;
  detach(): void;
}

let counter = 0;
/** A name for one run, unique in this window. */
function nextReadId(): string {
  counter += 1;
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? `diagnose-${crypto.randomUUID()}`
    : `diagnose-${Date.now()}-${counter}`;
}

/**
 * The running diagnosis as state of the source runtime rather than of a
 * screen: closing the Dashboard's warning window or leaving a session page
 * leaves it running, and every view (both kinds of warning dot, the window,
 * the session page) reads the same run. One runs at a time. Its answer is
 * kept in the runtime's query cache under {@link keptKey}, outside every key
 * an ingest re-reads; nothing is stored.
 *
 * Only the runtime ending (its source replaced, or the app closing) stops
 * it, through the controls it was sent with, and drops its late answer.
 */
export function createDiagnosisRuns(source: DataSource, client: QueryClient): DiagnosisRuns {
  let run: DiagnosisRun | null = null;
  /** The controls the running diagnosis was sent with: Stop goes to them. */
  let sentWith: DiagnosisControls | null = null;
  let live = true;
  const listeners = new Set<() => void>();
  const set = (next: DiagnosisRun | null) => {
    run = next;
    listeners.forEach((listener) => listener());
  };
  return {
    getRun: () => run,
    subscribe(listener) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    start(sessionId, warningId, timezone) {
      const controls = source.diagnosis;
      if (!controls || run || !live) return false;
      const readId = nextReadId();
      const started: DiagnosisRun = { sessionId, warningId, readId, startedAt: Date.now() };
      sentWith = controls;
      set(started);
      let request: Promise<SessionDiagnosis>;
      try {
        request = controls.diagnose(sessionId, warningId, timezone, readId);
      } catch (error) {
        request = Promise.reject(error);
      }
      void request
        .then(
          (answer): Kept => ({ readId, answer }),
          (): Kept => ({ readId, failed: true }),
        )
        .then((kept) => {
          if (!live || run !== started) return;
          client.setQueryData(keptKey(sessionId, warningId), kept);
          sentWith = null;
          set(null);
        });
      return true;
    },
    stop() {
      const current = run;
      return current && sentWith
        ? sentWith.cancel(current.readId).catch(() => false)
        : Promise.resolve(false);
    },
    attach() {
      live = true;
    },
    detach() {
      const current = run;
      const controls = sentWith;
      live = false;
      if (!current || !controls) return;
      run = null;
      sentWith = null;
      // The runtime is gone: no view is left to show Stop, so stop it here.
      void controls.cancel(current.readId).catch(() => false);
    },
  };
}

/** Time since a run was sent, as `m:ss`. */
export function elapsedClock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
