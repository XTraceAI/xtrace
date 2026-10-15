import type { QueryClient } from '@tanstack/react-query';
/** Cache acceptance cancellation only: native commands may continue running. */
export function invalidateDataQueries(client: QueryClient, prefix: string): Promise<void> {
  // The installed public cancelQueries API reverts query state synchronously.
  // Do not defer invalidation in a promise continuation that could outlive disposal.
  void client.cancelQueries({
    queryKey: [prefix],
    predicate: (query) => query.state.data === undefined && query.state.fetchStatus !== 'idle',
  });
  return client.invalidateQueries({ queryKey: [prefix] });
}
