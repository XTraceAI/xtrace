import type { QueryClient } from '@tanstack/react-query';
import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useSyncExternalStore } from 'react';
import { gradingRunDetailsPrefix } from './query-client';

/**
 * How many committed content purges this QueryClient has heard of. Kept in
 * the client's own cache, never read from a source and never refetched, so
 * no invalidation can reset it.
 */
const purgeKey = ['content-purges'] as const;

/** The prefix of every lane span's detail. */
const spanDetails = ['metrics', 'span-detail'] as const;
/** The prefix of every day card's sessions, which carry saved titles. */
const daySessions = ['metrics', 'day-sessions'] as const;

export const purgeEpoch = (client: QueryClient) => client.getQueryData<number>(purgeKey) ?? 0;

/**
 * A committed "Delete stored content": span details may hold deleted words,
 * and a day card's sessions may hold saved titles. Both are keyed by the
 * epoch they began in; raising it stops those reads and drops their answers,
 * cached or still to come, so the next open reads again. A grade's details
 * hold its summary and findings: every cached one is dropped too, so a closed
 * details dialog reopens on a fresh read.
 */
export function discardStoredWords(client: QueryClient) {
  client.setQueryData<number>(purgeKey, purgeEpoch(client) + 1);
  for (const queryKey of [spanDetails, daySessions, gradingRunDetailsPrefix]) {
    void client.cancelQueries({ queryKey });
    client.removeQueries({ queryKey });
  }
}

/** The current purge epoch, re-rendering when a purge raises it. */
export function usePurgeEpoch() {
  const client = useQueryClient();
  const subscribe = useCallback(
    (notify: () => void) => client.getQueryCache().subscribe(notify),
    [client],
  );
  return useSyncExternalStore(subscribe, () => purgeEpoch(client));
}
