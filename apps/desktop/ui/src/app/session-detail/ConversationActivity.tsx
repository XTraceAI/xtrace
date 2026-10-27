import { useContext, useId, useState } from 'react';
import { Button } from '../../kit/Button';
import { RevealContext } from './transcript-reveal';
import { TranscriptRecordView } from './TranscriptRecordView';
import {
  humanizeToolName,
  isDrawableBlock,
  isToolBlock,
  type TranscriptRecord,
} from './transcript-view';

/** Only confirmed representatives reach the conversation, never an inferred repeat. */
export interface ConversationMarker {
  readonly index: number;
  readonly blockId: string;
  readonly tool: string;
  readonly count: number;
  readonly circling: boolean;
  readonly selected: boolean;
}

export function RepeatNotice({ marker }: { marker: ConversationMarker }) {
  return (
    <aside
      className="xt-conversation-repeat"
      data-circling={marker.circling || undefined}
      data-current={marker.selected || undefined}
      data-repeat-index={marker.index}
      role="note"
    >
      <strong>
        {humanizeToolName(marker.tool)} · {marker.count.toLocaleString()} matching calls
      </strong>
      <p>Calls match by command, path or pattern. This marks the earliest matching call.</p>
    </aside>
  );
}

export function RepeatNavigation({
  indexes,
  selected,
  onSelect,
}: {
  indexes: readonly number[];
  selected: number | null;
  onSelect: (index: number) => void;
}) {
  const current = selected === null ? -1 : indexes.indexOf(selected);
  return (
    <nav className="xt-conversation-navigation" aria-label="Repeated activity">
      <span>Repeated activity</span>
      <Button
        variant="outline"
        height={28}
        aria-disabled={current <= 0}
        onClick={() => current > 0 && onSelect(indexes[current - 1]!)}
      >
        ↑ Previous
      </Button>
      <span>
        {current < 0
          ? `${indexes.length} ${indexes.length === 1 ? 'notice' : 'notices'}`
          : `${current + 1} of ${indexes.length}`}
      </span>
      <Button
        variant="outline"
        height={28}
        aria-disabled={current >= indexes.length - 1}
        onClick={() => current < indexes.length - 1 && onSelect(indexes[current + 1]!)}
      >
        Next ↓
      </Button>
    </nav>
  );
}

function activityOnly(record: TranscriptRecord): boolean {
  return (
    (record.role === 'assistant' || record.role === 'tool') &&
    record.blocks.every((block) => !isDrawableBlock(block) || isToolBlock(block))
  );
}

function Activity({ records }: { records: readonly TranscriptRecord[] }) {
  const [state, setState] = useState({ open: false, token: -1 });
  const id = useId();
  const { request } = useContext(RevealContext);
  const targeted =
    request !== null &&
    records.some((record) => record.blocks.some((block) => block.id === request.blockId));
  if (targeted && request.token !== state.token) {
    setState({ open: true, token: request.token });
  }
  const expanded = state.open;
  return (
    <div className="xt-conversation-activity">
      <button
        type="button"
        aria-expanded={expanded}
        aria-controls={id}
        onClick={() => setState({ ...state, open: !expanded })}
      >
        <span aria-hidden="true">{expanded ? '▾' : '▸'}</span> Tool activity · {records.length}{' '}
        saved {records.length === 1 ? 'message' : 'messages'}
      </button>
      <div id={id} hidden={!expanded}>
        {records.map((record, index) => (
          <TranscriptRecordView key={`${index}:${record.id}`} record={record} conversation />
        ))}
      </div>
    </div>
  );
}

/** Unknown source boundaries keep activity per record; identity and read order stay intact. */
export function ConversationRecords({
  records,
  markers,
  groupAdjacent,
  sourceStarts,
}: {
  records: readonly TranscriptRecord[];
  markers: readonly ConversationMarker[];
  groupAdjacent: boolean;
  sourceStarts: readonly number[];
}) {
  const boundaries = new Set(sourceStarts);
  const groups: { records: TranscriptRecord[]; activity: boolean; start: number }[] = [];
  records.forEach((record, index) => {
    const activity = activityOnly(record);
    const marked = record.blocks.some((block) => markers.some((mark) => mark.blockId === block.id));
    const last = groups.at(-1);
    if (groupAdjacent && activity && last?.activity && !marked && !boundaries.has(index))
      last.records.push(record);
    else groups.push({ records: [record], activity, start: index });
  });
  return groups.map((group) => (
    <div key={`${group.start}:${group.records[0]!.id}`} className="xt-conversation-entry">
      {group.activity ? (
        <>
          {markers
            .filter((mark) => group.records[0]!.blocks.some((block) => block.id === mark.blockId))
            .map((marker) => (
              <RepeatNotice key={marker.index} marker={marker} />
            ))}
          <Activity records={group.records} />
        </>
      ) : (
        <TranscriptRecordView record={group.records[0]!} conversation markers={markers} />
      )}
    </div>
  ));
}
