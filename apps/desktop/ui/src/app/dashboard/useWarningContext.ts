import { useCallback } from 'react';
import { useData } from '../../data/DataProvider';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { useSourceRead } from '../session-detail/useSessionTranscript';

/**
 * The original messages around one warning, read once for as long as the
 * caller stays mounted on the same warning. The warning's ID and the zone
 * go with the read: the native side looks the warning up and marks a
 * correction's words in its target message.
 */
export function useWarningContext(
  sessionId: string,
  warning: Pick<SessionWarningItem, 'id' | 'points_at'>,
  timezone: string,
) {
  const { source } = useData();
  const { id, points_at: point } = warning;
  const identity = JSON.stringify([sessionId, point.record_uuid, point.block, id, timezone]);
  const load = useCallback(
    (readId: string) =>
      source.sessionWarningContext(sessionId, point.record_uuid, point.block, readId, {
        id,
        timezone,
      }),
    [source, sessionId, point.record_uuid, point.block, id, timezone],
  );
  const cancel = useCallback((readId: string) => source.cancelSessionTranscript(readId), [source]);
  return useSourceRead(identity, load, cancel);
}
