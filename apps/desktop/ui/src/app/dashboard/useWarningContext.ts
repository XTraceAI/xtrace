import { useCallback } from 'react';
import { useData } from '../../data/DataProvider';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { useSourceRead } from '../session-detail/useSessionTranscript';

/**
 * The original messages around one warning, read once for as long as the
 * caller stays mounted on the same warning. `kind` asks the native side to
 * mark a correction's words in its target message.
 */
export function useWarningContext(
  sessionId: string,
  warning: Pick<SessionWarningItem, 'points_at' | 'kind'>,
) {
  const { source } = useData();
  const { points_at: point, kind } = warning;
  const identity = JSON.stringify([sessionId, point.record_uuid, point.block, kind]);
  const load = useCallback(
    (id: string) =>
      source.sessionWarningContext(sessionId, point.record_uuid, point.block, id, kind),
    [source, sessionId, point.record_uuid, point.block, kind],
  );
  const cancel = useCallback((id: string) => source.cancelSessionTranscript(id), [source]);
  return useSourceRead(identity, load, cancel);
}
