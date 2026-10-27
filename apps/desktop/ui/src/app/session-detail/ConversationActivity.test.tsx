import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { SessionTranscript } from './SessionTranscript';
import type { ConversationMarker } from './ConversationActivity';
import type { TranscriptRecord } from './transcript-view';

const call = (id: string, name = 'Read') => ({
  kind: 'tool_call' as const,
  id,
  name,
  outcome: { state: 'unrecorded' as const },
});
const activity = (id: string): TranscriptRecord => ({
  id,
  role: 'assistant',
  blocks: [call(`${id}:0`)],
});
const marker = (index: number, blockId: string): ConversationMarker => ({
  index,
  blockId,
  tool: 'Read',
  count: 4,
  circling: false,
  selected: false,
});
afterEach(cleanup);

it('retains every record in read order and keeps unsupported/system/user records visible', () => {
  const records: TranscriptRecord[] = [
    activity('a'),
    activity('b'),
    { id: 'empty', role: 'assistant', blocks: [] },
    {
      id: 'system',
      role: 'system',
      blocks: [{ kind: 'unsupported', id: 'system:0', label: 'image' }],
    },
    {
      id: 'user',
      role: 'user',
      blocks: [{ kind: 'text', id: 'user:0', text: 'Literal user prompt' }],
    },
  ];
  const { container } = render(
    <SessionTranscript conversation groupAdjacentActivity records={records} />,
  );
  expect(
    [...container.querySelectorAll('[data-record-id]')].map((node) =>
      node.getAttribute('data-record-id'),
    ),
  ).toEqual(records.map((record) => record.id));
  expect(screen.getAllByRole('button', { name: /Tool activity/ })).toHaveLength(1);
  expect(screen.queryByRole('separator')).toBeNull();
  expect(screen.getByText(/A block of type “image”/).closest('[hidden]')).toBeNull();
  expect(screen.getByText('Literal user prompt').closest('[hidden]')).toBeNull();
  fireEvent.click(screen.getAllByRole('button', { name: /Tool activity/ })[0]!);
  expect(
    screen.getByText('No content from this turn is shown here.').closest('[hidden]'),
  ).toBeNull();
});

it('splits marked activity records and exact mixed tool rows while keeping distinct notices for a shared locator', () => {
  const records: TranscriptRecord[] = [
    activity('a'),
    { ...activity('b'), blocks: [call('b:0', 'mcp__workspace__read_configuration')] },
    {
      id: 'mixed',
      role: 'assistant',
      blocks: [
        { kind: 'text', id: 'mixed:0', text: 'Before' },
        call('mixed:1'),
        call('mixed:2'),
        call('mixed:3', 'Bash'),
        call('mixed:4'),
        { kind: 'text', id: 'mixed:5', text: 'After' },
      ],
    },
  ];
  const markers = [
    { ...marker(0, 'b:0'), tool: 'mcp__workspace__read_configuration' },
    marker(1, 'mixed:2'),
    marker(2, 'mixed:2'),
  ];
  const { container } = render(
    <SessionTranscript conversation groupAdjacentActivity records={records} markers={markers} />,
  );
  expect(screen.getAllByRole('button', { name: /Tool activity/ })).toHaveLength(2);
  const notices = [...container.querySelectorAll('[data-repeat-index]')];
  expect(notices.map((node) => node.getAttribute('data-repeat-index'))).toEqual(['0', '1', '2']);
  expect(screen.getByText('Read configuration · 4 matching calls')).toBeTruthy();
  fireEvent.click(screen.getAllByRole('button', { name: /Tool activity/ })[1]!);
  fireEvent.click(screen.getByRole('button', { name: 'Read configuration' }));
  expect(container.querySelector('[data-block-id="b:0"]')?.getAttribute('data-tool')).toBe(
    'mcp__workspace__read_configuration',
  );
  expect(notices.every((node) => !node.closest('[hidden]'))).toBe(true);
  const mixed = container.querySelector('[data-record-id="mixed"]')!;
  expect(
    [...mixed.querySelectorAll('[data-block-id]')].map((node) =>
      node.getAttribute('data-block-id'),
    ),
  ).toEqual(['mixed:0', 'mixed:1', 'mixed:2', 'mixed:3', 'mixed:4', 'mixed:5']);
  expect(mixed.querySelectorAll('.xt-tool-row')).toHaveLength(4);
  expect(notices[1]!.nextElementSibling).toBe(notices[2]);
});

it('opens both exact owning disclosures once, leaves focus with its caller, and lets the reader close them again', () => {
  Element.prototype.scrollIntoView = vi.fn();
  const report = vi.fn();
  const records = [activity('a'), activity('b')];
  const view = render(
    <>
      <button>Caller</button>
      <SessionTranscript conversation groupAdjacentActivity records={records} onReveal={report} />
    </>,
  );
  const caller = screen.getByRole('button', { name: 'Caller' });
  caller.focus();
  view.rerender(
    <>
      <button>Caller</button>
      <SessionTranscript
        conversation
        groupAdjacentActivity
        records={records}
        reveal={{ token: 1, blockId: 'b:0' }}
        onReveal={report}
      />
    </>,
  );
  const group = screen.getByRole('button', { name: /Tool activity/ });
  expect(group.getAttribute('aria-expanded')).toBe('true');
  expect(document.activeElement).toBe(caller);
  expect(report).toHaveBeenCalledExactlyOnceWith(1, 'revealed');
  expect(
    document.querySelector('.xt-tool-card[data-highlighted]')?.getAttribute('data-block-id'),
  ).toBe('b:0');
  fireEvent.click(group);
  expect(group.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(group);
  expect(group.getAttribute('aria-expanded')).toBe('true');
  expect(report).toHaveBeenCalledOnce();
});

it('keeps unknown legacy multi-source activity per record without inventing source separators', () => {
  const records = [
    activity('a'),
    activity('b'),
    { id: 'empty', role: 'assistant' as const, blocks: [] },
  ];
  const { container } = render(
    <SessionTranscript conversation records={records} groupAdjacentActivity={false} />,
  );
  expect(screen.getAllByRole('button', { name: /Tool activity/ })).toHaveLength(3);
  expect(screen.queryByRole('separator')).toBeNull();
  expect(screen.queryByText('Next conversation source')).toBeNull();
  expect(
    [...container.querySelectorAll('[data-record-id]')].map((node) =>
      node.getAttribute('data-record-id'),
    ),
  ).toEqual(['a', 'b', 'empty']);
});
