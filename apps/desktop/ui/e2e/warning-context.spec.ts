import { expect, test } from '@playwright/test';
import { SESSION_WARNINGS_TEXT } from '../src/data/generated/SessionWarningsText';
import { warning } from './session-warnings.data';

type Calls = { reads: unknown[][]; cancels: string[] };
for (const scheme of ['light', 'dark'] as const) {
  for (const variant of ['', 'long', 'omitted', 'refusal', 'lead', 'lead-adjacent'] as const) {
    test(`standalone warning context ${variant || 'bounded'} in ${scheme}`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width: variant === 'long' ? 430 : 1120, height: 850 });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto(`/e2e/warning-context.html?${variant}`);
      const context = page.getByTestId('warning-context');
      await expect(context).toBeVisible();
      if (variant === 'refusal') {
        await expect(context.getByRole('status')).toHaveText(
          'More than one message matches this issue. The original context cannot be identified safely.',
        );
        await expect(context.locator('[data-record-id]')).toHaveCount(0);
      } else {
        await expect(context.getByTestId('warning-context-flag')).toHaveText(
          SESSION_WARNINGS_TEXT.message_tags[warning.kind],
        );
        await expect(context).not.toContainText(
          'This unrelated message must stay outside the excerpt.',
        );
        if (variant === 'omitted') {
          await expect(context).toContainText(
            'Other blocks in this message are not shown in this excerpt.',
          );
          await expect(context).toContainText('Synthetic tool failure.');
          await expect(context).not.toContainText('Person');
          await expect(context.locator('[data-highlighted]')).toHaveCount(1);
        } else if (variant === 'lead' || variant === 'lead-adjacent') {
          // The person's message the reply answered comes first, as "You".
          const records = context.locator('[data-record-id]');
          await expect(records).toHaveCount(4);
          await expect(records.first()).toHaveAttribute('data-record-id', 'asked');
          await expect(records.first()).toHaveAttribute('data-own', 'true');
          await expect(records.first()).toContainText(SESSION_WARNINGS_TEXT.you);
          await expect(context.getByTestId('warning-context-gap')).toHaveCount(
            variant === 'lead' ? 1 : 0,
          );
          if (variant === 'lead')
            await expect(context.getByRole('separator')).toHaveText(
              SESSION_WARNINGS_TEXT.context_gap,
            );
          await expect(context).not.toContainText('Synthetic skipped agent step.');
          await expect(context.locator('.xt-warning-context-caption')).toHaveText(
            SESSION_WARNINGS_TEXT.context_caption_with_lead,
          );
        } else {
          await expect(context.locator('[data-record-id]')).toHaveCount(3);
          await expect(context).toContainText(SESSION_WARNINGS_TEXT.you);
          await expect(context.getByTestId('warning-context-gap')).toHaveCount(0);
          await expect(context.locator('.xt-warning-context-caption')).toHaveText(
            SESSION_WARNINGS_TEXT.context_caption,
          );
        }
      }
      expect(await context.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      if (process.env.CONTEXT_SHOTS)
        await page.screenshot({
          path: `${process.env.CONTEXT_SHOTS}/context-${variant || 'bounded'}-${testInfo.project.name}-${scheme}.png`,
        });
      const before = await page.evaluate(
        () => (window as unknown as { __context: Calls }).__context,
      );
      expect(before.reads).toHaveLength(1);
      expect(before.reads[0]!.slice(0, 3)).toEqual([
        'synthetic-session',
        'target',
        variant === 'omitted' ? 99 : null,
      ]);
      await page.getByRole('button', { name: 'Close context' }).click();
      await expect(context).toHaveCount(0);
      const after = await page.evaluate(
        () => (window as unknown as { __context: Calls }).__context,
      );
      expect(after.cancels).toEqual([before.reads[0]![3]]);
    });
  }
}
