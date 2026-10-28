import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { SessionTranscript } from './SessionTranscript';
import { ToolCallCard } from './ToolStrip';
import type { ToolCallBlock, TranscriptRecord } from './transcript-view';

afterEach(cleanup);
const NOTE = 'Results for this call are not checked across the whole conversation.';
const call: ToolCallBlock = {
  kind: 'tool_call',
  id: 'call-block',
  name: 'Bash',
  callId: 'synthetic-call',
  input: { kind: 'json', value: { command: 'pnpm test' } },
  outcome: { state: 'not_checked' },
};

it('keeps the "not checked" note on a transcript card and leaves it out of an excerpt card', () => {
  const { rerender } = render(<ToolCallCard block={call} />);
  expect(screen.getByText(NOTE)).toBeTruthy();
  rerender(<ToolCallCard block={call} excerpt />);
  expect(screen.queryByText(NOTE)).toBeNull();
  expect(screen.getByText('pnpm test')).toBeTruthy();
});

it('keeps the note in a session transcript, which never passes the excerpt flag', () => {
  const record: TranscriptRecord = { id: 'record', role: 'assistant', blocks: [call] };
  render(<SessionTranscript records={[record]} />);
  // The strip's row for the call, opened to its card.
  if (!screen.queryByText(NOTE)) fireEvent.click(screen.getAllByRole('button')[0]!);
  expect(screen.getByText(NOTE)).toBeTruthy();
});
