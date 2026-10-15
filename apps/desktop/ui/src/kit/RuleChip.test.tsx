import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { RuleChip } from './RuleChip';
import { RulePopover } from './RulePopover';
import { rules } from './rules';

afterEach(cleanup);

it('associates a clearly named definition trigger with current rule text on focus', async () => {
  render(<RuleChip ruleId="M-06" subject="Concurrency" size="sm" />);
  const trigger = screen.getByRole('button', { name: 'About Concurrency' });
  expect(trigger.textContent).toBe('Info');
  expect(trigger.getAttribute('data-size')).toBe('sm');
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  expect(tooltip.textContent).toBe(rules['M-06']);
  fireEvent.blur(trigger, { relatedTarget: document.body });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
});

it('preserves caller text and report context containing identifiers', async () => {
  const context = 'Saved rule M-19; SHA-256; external reference CUSTOM-42';
  render(
    <RulePopover ruleId="M-08" context={context}>
      <button>Saved rule M-19</button>
    </RulePopover>,
  );
  const trigger = screen.getByRole('button', { name: 'Saved rule M-19' });
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(tooltip.querySelector('.xt-rule-context')?.textContent).toBe(context);
  expect(tooltip.firstElementChild?.textContent).toBe(
    'Agent-to-human ratio = agent minutes ÷ human-in-the-loop minutes over the window. No assumed workday.',
  );
});
