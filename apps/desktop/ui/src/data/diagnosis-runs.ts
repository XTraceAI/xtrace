import type { QueryClient } from '@tanstack/react-query';
import { purgeEpoch } from './content-purges';
import type { DataSource, DiagnosisControls } from './DataSource';
import type { SessionDiagnosis } from './generated/SessionDiagnosis';
import { diagnosisAnswersPrefix } from './query-client';

/**
 * Where a running diagnosis stands with Stop: not asked, asked and not yet
 * answered, or accepted. Stop waits in every view once it has been asked, and
 * stays waiting once accepted until the run ends.
 */
export type StopState = 'none' | 'asking' | 'accepted';

/** The one diagnosis running in this app, and when it was sent. */
export type DiagnosisRun = {
  readonly sessionId: string;
  readonly warningId: string;
  readonly readId: string;
  /** When it was sent, in this window's clock (`Date.now()`). */
  readonly startedAt: number;
  readonly stop: StopState;
};

/** A kept answer, or that the diagnosis could not be asked for at all. */
export type Kept = { readonly readId: string } & (
  { readonly answer: SessionDiagnosis } | { readonly failed: true }
);

/** Where one warning's kept answer lives in the query cache. */
export const keptKey = (sessionId: string, warningId: string) =>
  [...diagnosisAnswersPrefix, sessionId, warningId] as const;

/**
 * Whether `run` is the diagnosis of this session's warning: the one rule
 * every view uses to tell its own warning's run from another one. A warning
 * ID is unique only within its session.
 */
export const isRunFor = (
  run: Pick<DiagnosisRun, 'sessionId' | 'warningId'> | null | undefined,
  sessionId: string,
  warningId: string,
) => run != null && run.sessionId === sessionId && run.warningId === warningId;

export interface DiagnosisRuns {
  getRun(): DiagnosisRun | null;
  subscribe(listener: () => void): () => void;
  /**
   * Sends one warning's diagnosis; false, and nothing is sent, while one is
   * running or where no coding tool can be run.
   */
  start(sessionId: string, warningId: string, timezone: string): boolean;
  /**
   * Asks the running diagnosis to stop: true once accepted. False, and
   * nothing is asked, when none runs or Stop was already asked; false too
   * when it was refused or could not be asked, and Stop may be asked again.
   */
  stop(): Promise<boolean>;
  /** While detached (after its runtime unmounts), late answers change nothing. */
  attach(): void;
  detach(): void;
}

/** How long a purge waits before asking its refused Stop again. */
const PURGE_RETRY_MS = 1000;

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
 * an ingest re-reads, until it is put away; nothing is stored.
 *
 * The runtime ending (its source replaced, or the app closing) stops it,
 * through the controls it was sent with, and drops its late answer. So does
 * a committed "Delete stored content" (the purge epoch rising): the answer
 * could quote deleted words.
 */
export function createDiagnosisRuns(source: DataSource, client: QueryClient): DiagnosisRuns {
  // An answer can arrive while no view shows its warning: it is kept until
  // it is put away or its content purged, not dropped once unwatched.
  client.setQueryDefaults(diagnosisAnswersPrefix, { gcTime: Infinity });
  let run: DiagnosisRun | null = null;
  /** The controls the running diagnosis was sent with: Stop goes to them. */
  let sentWith: DiagnosisControls | null = null;
  /** The purge epoch the running diagnosis was sent in. */
  let sentInEpoch = 0;
  /** The purge epoch the running diagnosis was last told about. */
  let heardEpoch = 0;
  /**
   * The run a purge still owes a Stop: set when a purge comes after it was
   * sent, and cleared only once a cancel for it is accepted or it ends. A
   * refused or failed cancel (the user's or the purge's own) leaves it owed.
   */
  let purgeOwes: string | null = null;
  /** The next try of a purge's refused Stop, paced so a refusal cannot spin. */
  let retry: ReturnType<typeof setTimeout> | undefined;
  let live = true;
  let unwatch: (() => void) | null = null;
  const listeners = new Set<() => void>();
  const set = (next: DiagnosisRun | null) => {
    run = next;
    if (!next) {
      sentWith = null;
      purgeOwes = null;
      clearTimeout(retry);
      retry = undefined;
    }
    listeners.forEach((listener) => listener());
  };
  /** Changes the Stop state of `current` while it is still the running one. */
  const stopState = (current: DiagnosisRun, stop: StopState) => {
    if (run?.readId === current.readId && run.stop !== stop) set({ ...run, stop });
  };
  /** Asks the purge's owed Stop, if Stop is free to be asked now. */
  const payPurge = () => {
    if (live && purgeOwes !== null && run?.readId === purgeOwes && run.stop === 'none')
      void ask(true);
  };
  const ask = (byPurge: boolean): Promise<boolean> => {
    const current = run;
    const controls = sentWith;
    if (!current || !controls || current.stop !== 'none') return Promise.resolve(false);
    stopState(current, 'asking');
    const settle = (accepted: boolean) => {
      stopState(current, accepted ? 'accepted' : 'none');
      if (accepted) {
        if (purgeOwes === current.readId) purgeOwes = null;
      } else if (purgeOwes === current.readId) {
        // Still owed. After the user's refused Stop, ask at once; after the
        // purge's own, try again shortly.
        if (byPurge) {
          clearTimeout(retry);
          retry = setTimeout(payPurge, PURGE_RETRY_MS);
        } else payPurge();
      }
      return accepted;
    };
    return controls.cancel(current.readId).then(settle, () => settle(false));
  };
  const stop = () => ask(false);
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
      const started: DiagnosisRun = {
        sessionId,
        warningId,
        readId,
        startedAt: Date.now(),
        stop: 'none',
      };
      sentWith = controls;
      sentInEpoch = purgeEpoch(client);
      heardEpoch = sentInEpoch;
      const epoch = sentInEpoch;
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
          if (!live || run?.readId !== readId) return;
          // Sent before a purge: its answer may quote deleted words.
          if (purgeEpoch(client) === epoch)
            client.setQueryData(keptKey(sessionId, warningId), kept);
          set(null);
        });
      return true;
    },
    stop,
    attach() {
      live = true;
      unwatch ??= client.getQueryCache().subscribe(() => {
        const epoch = purgeEpoch(client);
        if (epoch === heardEpoch) return;
        heardEpoch = epoch;
        if (!run || epoch === sentInEpoch) return;
        purgeOwes = run.readId;
        payPurge();
      });
    },
    detach() {
      const current = run;
      const controls = sentWith;
      live = false;
      unwatch?.();
      unwatch = null;
      clearTimeout(retry);
      retry = undefined;
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
