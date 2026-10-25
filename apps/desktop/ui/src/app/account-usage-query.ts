import type { DataSource } from '../data/DataSource';
import type { AccountUsage } from '../data/generated/AccountUsage';
import { queryKeys } from '../data/query-client';

export const accountUsageRefreshMs = 5 * 60_000;
/** While the app's own Claude read is running, look for its result this often. */
export const accountUsageReadingMs = 3_000;

/**
 * Passive mount, timer and focus reads never request Claude credentials or
 * start a Claude read; the app reads Claude on its own schedule. While that
 * read is still running, or while the app watches for a sign-in the user
 * started, the window checks back sooner for its result.
 */
export function accountUsageQueryOptions(source: DataSource) {
  return {
    queryKey: queryKeys.accountUsage,
    queryFn: () => source.accountUsage(),
    enabled: source.kind !== 'preview',
    staleTime: 4 * 60_000,
    refetchInterval: (query?: { state: { data?: AccountUsage } }) =>
      document.visibilityState !== 'visible'
        ? false
        : query?.state.data?.claude.issue === 'reading' ||
            (query?.state.data?.claude.sign_in_watch_until ?? 0) * 1000 > Date.now()
          ? accountUsageReadingMs
          : accountUsageRefreshMs,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    retry: false,
  } as const;
}
