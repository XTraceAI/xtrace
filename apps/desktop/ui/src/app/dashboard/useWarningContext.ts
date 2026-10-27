import { useCallback } from 'react';
import { useData } from '../../data/DataProvider';
import type { SessionWarningPoint } from '../../data/generated/SessionWarningPoint';
import { useSourceRead } from '../session-detail/useSessionTranscript';

export function useWarningContext(sessionId: string, point: SessionWarningPoint) {
  const { source } = useData();
  const identity = JSON.stringify([sessionId, point.record_uuid, point.block]);
  const load = useCallback(
    (id: string) => source.sessionWarningContext(sessionId, point.record_uuid, point.block, id),
    [source, sessionId, point.record_uuid, point.block],
  );
  const cancel = useCallback((id: string) => source.cancelSessionTranscript(id), [source]);
  return useSourceRead(identity, load, cancel);
}
