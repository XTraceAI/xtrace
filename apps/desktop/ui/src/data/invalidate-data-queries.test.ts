import { onlineManager, QueryObserver } from '@tanstack/react-query';
import { expect, it, vi } from 'vitest';
import { invalidateDataQueries } from './invalidate-data-queries';
import { createQueryClient } from './query-client';

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};

it('cancels a cold paused request and reads once after reconnect without reviving the old retryer', async () => {
  const client = createQueryClient();
  const queryFn = vi.fn(async () => 222);
  onlineManager.setOnline(false);
  client.mount();
  const observer = new QueryObserver(client, { queryKey: ['metrics', 'paused', 14], queryFn });
  const stop = observer.subscribe(() => {});
  try {
    const query = client.getQueryCache().find({ queryKey: ['metrics', 'paused', 14] })!;
    const originalPromise = query.promise;
    expect(query.state.fetchStatus).toBe('paused');
    expect(queryFn).not.toHaveBeenCalled();
    await invalidateDataQueries(client, 'metrics');
    expect(query.promise).not.toBe(originalPromise);
    expect(query.state.data).toBeUndefined();
    expect(query.state.fetchStatus).toBe('paused');
    expect(query.state.isInvalidated).toBe(true);
    expect(queryFn).not.toHaveBeenCalled();
    onlineManager.setOnline(true);
    await vi.waitFor(() => expect(query.state.data).toBe(222));
    expect(queryFn).toHaveBeenCalledTimes(1);
    expect(query.state.status).toBe('success');
    expect(query.state.fetchStatus).toBe('idle');
    expect(query.state.isInvalidated).toBe(false);
  } finally {
    stop();
    client.unmount();
    client.clear();
    onlineManager.setOnline(true);
  }
});
it('keeps the cached-data restart policy when an older background request is pending', async () => {
  const client = createQueryClient();
  const old = deferred<number>();
  const queryFn = vi
    .fn(async () => 111)
    .mockImplementationOnce(async () => 111)
    .mockImplementationOnce(() => old.promise)
    .mockResolvedValue(222);
  const key = ['metrics', 'cached', 7];
  const observer = new QueryObserver(client, { queryKey: key, queryFn });
  const stop = observer.subscribe(() => {});
  try {
    await vi.waitFor(() => expect(client.getQueryData(key)).toBe(111));
    void client.refetchQueries({ queryKey: key });
    expect(queryFn).toHaveBeenCalledTimes(2);
    expect(client.getQueryState(key)?.fetchStatus).toBe('fetching');
    await invalidateDataQueries(client, 'metrics');
    expect(queryFn).toHaveBeenCalledTimes(3);
    expect(client.getQueryData(key)).toBe(222);
    old.resolve(111);
    await new Promise((done) => setTimeout(done, 20));
    expect(client.getQueryData(key)).toBe(222);
    expect(client.getQueryState(key)?.status).toBe('success');
    expect(client.getQueryState(key)?.fetchStatus).toBe('idle');
    expect(client.getQueryState(key)?.isInvalidated).toBe(false);
  } finally {
    stop();
    client.clear();
  }
});
