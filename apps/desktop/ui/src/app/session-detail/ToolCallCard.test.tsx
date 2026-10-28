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

// The native side names an encrypted token in words
// (`transcript_dto::encrypted_placeholder`); the card prints that line with
// the rest of the input, in the transcript and in a warning's excerpt alike.
it('prints an encrypted message as its short native line beside the rest of the input', () => {
  const encrypted: ToolCallBlock = {
    ...call,
    name: 'send_message',
    input: {
      kind: 'json',
      value: { message: 'encrypted (812 characters, not readable)', target: '/root' },
    },
  };
  for (const excerpt of [false, true]) {
    const { container, unmount } = render(<ToolCallCard block={encrypted} excerpt={excerpt} />);
    expect(container.textContent).toContain(
      '"message": "encrypted (812 characters, not readable)"',
    );
    expect(container.textContent).toContain('"target": "/root"');
    unmount();
  }
});

// The header reads as the strip's row does; the recorded name stays on hover and on the node.
it('names a call by its readable tool name and keeps the recorded name on hover', () => {
  const plan: ToolCallBlock = { ...call, name: 'mcp__ccd_turn__update_plan' };
  const { container } = render(<ToolCallCard block={plan} excerpt />);
  const name = container.querySelector('.xt-tool-name')!;
  expect(name.textContent).toBe('Update plan');
  expect(name.getAttribute('title')).toBe('mcp__ccd_turn__update_plan');
  expect(container.querySelector('.xt-tool-card')!.getAttribute('data-tool')).toBe(
    'mcp__ccd_turn__update_plan',
  );
  cleanup();
  const { container: bash } = render(<ToolCallCard block={call} excerpt />);
  expect(bash.querySelector('.xt-tool-name')!.textContent).toBe('Bash');
  expect(bash.querySelector('.xt-tool-name')!.hasAttribute('title')).toBe(false);
});

// Which inputs are task lists is decided natively (`transcript_dto::task_list`); the card
// draws the tasks it is handed and leaves the JSON out.
it('draws a task list as a checklist in place of its JSON, in a transcript and an excerpt', () => {
  const plan: ToolCallBlock = {
    ...call,
    name: 'mcp__ccd_turn__update_plan',
    input: {
      kind: 'json',
      value: { items: [{ content: 'Fix the hover', status: 'completed' }] },
    },
    taskList: [
      { text: 'Fix the hover', status: 'done' },
      { text: 'Build the runner', status: 'in_progress' },
      { text: 'Add the screens', status: 'not_started' },
    ],
  };
  for (const excerpt of [false, true]) {
    const { container, unmount } = render(<ToolCallCard block={plan} excerpt={excerpt} />);
    const items = screen.getByRole('list', { name: 'Tasks' }).querySelectorAll('li');
    expect([...items].map((item) => item.textContent)).toEqual([
      '✓Fix the hover (done)',
      '◐Build the runner (in progress)',
      '○Add the screens (not started)',
    ]);
    expect([...items].map((item) => item.dataset.status)).toEqual([
      'done',
      'in_progress',
      'not_started',
    ]);
    expect(container.textContent).not.toContain('"items"');
    expect(container.querySelector('[data-payload="input"] pre')).toBeNull();
    unmount();
  }
});

it('shows the input as it is when there is no task list', () => {
  for (const taskList of [undefined, null, []]) {
    const { container, unmount } = render(<ToolCallCard block={{ ...call, taskList }} />);
    expect(screen.queryByRole('list', { name: 'Tasks' })).toBeNull();
    expect(container.textContent).toContain('pnpm test');
    unmount();
  }
});
