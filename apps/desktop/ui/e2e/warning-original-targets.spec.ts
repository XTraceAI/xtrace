import { expect, test } from '@playwright/test';
import { admitted, admittedOnPerson, warning, second } from './session-warnings.data';
import type { SessionPageSelection } from '../src/data/generated/SessionPageSelection';

const anchors = (page: import('@playwright/test').Page) =>
  page.evaluate(() =>
    (
      window as unknown as { __warningPageSelections: SessionPageSelection[] }
    ).__warningPageSelections.filter((selection) => selection.kind === 'anchor'),
  );

for (const host of ['claude', 'codex'] as const)
  for (const theme of ['light', 'dark'] as const) {
    for (const paged of [false, true])
      test(`${host} ${paged ? 'paged' : 'full'} exact warning targets ${theme}`, async ({
        page,
      }, info) => {
        await page.setViewportSize({ width: 1120, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(
          `/e2e/session-conversation.html?warning-targets&host=${host}${paged ? '&paged' : ''}`,
        );
        const conversation = page.getByRole('region', { name: 'Conversation content' });
        const selected = page.getByRole('list', { name: 'Selected warning' });
        const card = selected.getByTestId('warning-item');
        const message = conversation.locator('[data-record-id="prompt"]');
        await expect(message).toHaveAttribute('data-highlighted', 'true');
        await expect(message).toBeInViewport();
        await expect(card).toBeFocused();
        await expect(
          page.getByText('Showing the original warning message.', { exact: true }),
        ).toBeVisible();
        const first = await anchors(page);
        expect(first).toHaveLength(paged ? 1 : 0);
        if (paged)
          expect(first[0]).toMatchObject({
            kind: 'anchor',
            expected: 'message',
            record_uuid: 'prompt',
            block: null,
          });
        await page.getByRole('button', { name: second.name }).click();
        const result = conversation.locator('.xt-tool-card[data-block-id="result:128"]');
        await expect(result).toHaveAttribute('data-highlighted', 'true');
        await expect(result).toBeInViewport();
        await expect(card).toBeFocused();
        await expect(
          page.getByText('Showing the warning’s original tool block.', { exact: true }),
        ).toBeVisible();
        const calls = await anchors(page);
        expect(calls).toHaveLength(paged ? 2 : 0);
        if (paged)
          expect(calls[1]).toMatchObject({
            kind: 'anchor',
            expected: 'tool_block',
            record_uuid: 'result',
            block: 128,
          });
        await page.screenshot({
          path: info.outputPath(`${host}-${theme}-${paged ? 'paged' : 'full'}-result.png`),
          fullPage: true,
        });
        // An admitted mistake points at the agent's reply, not the person's message.
        await page.getByRole('button', { name: admitted.name }).click();
        const reply = conversation.locator('[data-record-id="answer"]');
        await expect(reply).toHaveAttribute('data-highlighted', 'true');
        await expect(reply).toHaveAttribute('data-role', 'assistant');
        await expect(reply).toBeInViewport();
        await expect(card).toBeFocused();
        await expect(
          page.getByText('Showing the original warning message.', { exact: true }),
        ).toBeVisible();
        await expect(conversation.locator('[data-highlighted]')).toHaveCount(1);
        const replies = await anchors(page);
        expect(replies).toHaveLength(paged ? 3 : 0);
        if (paged)
          expect(replies[2]).toMatchObject({
            kind: 'anchor',
            expected: 'message',
            record_uuid: 'answer',
            block: null,
          });
        await page.screenshot({
          path: info.outputPath(`${host}-${theme}-${paged ? 'paged' : 'full'}-reply.png`),
          fullPage: true,
        });
        // The same kind saved on a person's message is refused, never shown as the reply.
        await page.getByRole('button', { name: admittedOnPerson.name }).click();
        await expect(
          page
            .getByText('The saved warning position does not point to an agent reply.', {
              exact: false,
            })
            .first(),
        ).toBeVisible();
        await expect(conversation.locator('[data-highlighted]')).toHaveCount(0);
        await page.getByRole('button', { name: warning.name }).click();
        await expect(message).toHaveAttribute('data-highlighted', 'true');
        const count = (await anchors(page)).length;
        await selected.getByRole('button', { name: 'Dismiss', exact: true }).click();
        await selected.getByRole('button', { name: 'Undo', exact: true }).click();
        await expect(selected.getByRole('button', { name: 'Dismiss', exact: true })).toBeFocused();
        expect(await anchors(page)).toHaveLength(count);
      });
    test(`${host} changed warning source stays refused ${theme}`, async ({ page }, info) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(
        `/e2e/session-conversation.html?warning-targets&paged&warning-changed&host=${host}`,
      );
      const conversation = page.getByRole('region', { name: 'Conversation content' });
      await expect(conversation.locator('[data-record-id="prompt"]')).toHaveAttribute(
        'data-highlighted',
        'true',
      );
      await page.getByRole('button', { name: second.name }).click();
      await expect(conversation).toContainText(
        'This conversation changed while you were reading it.',
      );
      const count = (await anchors(page)).length;
      await page.getByRole('button', { name: warning.name }).click();
      await expect(page.getByRole('list', { name: 'Selected warning' })).toContainText(
        warning.reason,
      );
      expect(await anchors(page)).toHaveLength(count);
      await expect(conversation.locator('[data-highlighted]')).toHaveCount(0);
      await page.screenshot({
        path: info.outputPath(`${host}-${theme}-changed-refusal.png`),
        fullPage: true,
      });
      await conversation.getByRole('button', { name: 'Load conversation again' }).click();
      await expect(conversation).toContainText('Showing messages 1–2 of 6.');
    });
  }
