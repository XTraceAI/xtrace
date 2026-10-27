import fixture from '../fixtures/F1.json' with { type: 'json' };
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { answer, dismissedAnswer } from './session-warnings.data';

export const session = fixture.sessions[0]!.rows[0]!;
export const title = 'Synthetic warning conversation';
export const filters = new URLSearchParams({
  q: title,
  host: session.host,
  range: '7d',
  sort: 'recently_active',
}).toString();

export function warningSource() {
  const exported = structuredClone(fixture) as FixtureExport;
  for (const page of exported.sessions)
    for (const row of page.rows) if (row.id === session.id) row.title = title;
  const source = new FixtureDataSource(exported);
  let current = answer;
  source.sessionWarnings = async (ids, timezone) => ({
    timezone,
    sessions: ids.map((session_id) => ({
      session_id,
      answer: session_id === session.id ? current : { state: 'missing' as const },
    })),
  });
  source.sessionTranscript = async () => ({
    state: 'loaded',
    generation: { generation: 'indexed', appended: false },
    sources: 1,
    dropped_records: 0,
    gaps: [],
    records: Array.from({ length: 30 }, (_, index) => ({
      id: `synthetic-message-${index}`,
      role: 'user',
      at: null,
      blocks: [{ kind: 'text', index: 0, text: `Synthetic conversation message ${index + 1}.` }],
    })),
  });
  Object.defineProperty(source, 'warningDismissals', {
    value: {
      dismiss: async (session_id: string, warning_id: string) => {
        current = dismissedAnswer;
        return { session_id, warning_id, dismissed: true };
      },
      restore: async (session_id: string, warning_id: string) => {
        current = answer;
        return { session_id, warning_id, dismissed: false };
      },
    },
  });
  return source;
}
