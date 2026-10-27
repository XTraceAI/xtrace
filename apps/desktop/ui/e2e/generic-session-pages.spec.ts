import { expect, test } from '@playwright/test';

for (const host of ['claude', 'codex'] as const) {
  for (const theme of ['light', 'dark'] as const) {
    test(`${host} generic paging feedback and refusal ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width: 1120, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/e2e/session-conversation.html?paged&host=${host}`);
      const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
      await expect(conversation).toContainText('Showing messages 1–2 of 6.');
      const later = conversation.getByRole('button', { name: 'Later messages' });
      await later.focus();
      await later.press('Enter');
      await expect(conversation).toContainText('Showing messages 3–6 of 6.');
      await expect(later).toBeFocused();
      await conversation.getByRole('button', { name: 'Earlier messages' }).click();
      await expect(conversation).toContainText('Showing messages 1–2 of 6.');
      const navigation = conversation.getByRole('navigation', { name: 'Repeated activity' });
      const next = navigation.getByRole('button', { name: 'Next ↓' });
      await next.focus();
      await next.press('Enter');
      await expect(conversation.locator('.xt-tool-card[data-block-id="tools:0"]')).toHaveAttribute(
        'data-highlighted',
        'true',
      );
      await expect(next).toBeFocused();
      await next.press('Enter');
      const target = conversation.locator('.xt-tool-card[data-block-id="mixed:3"]');
      await expect(target).toHaveAttribute('data-highlighted', 'true');
      await expect(next).toBeFocused();
      await expect(
        conversation.locator('.xt-conversation-repeat[data-current="true"]'),
      ).toHaveCount(1);
      await expect(
        page
          .locator('[role="status"][aria-live="polite"]')
          .filter({ hasText: 'Showing the earliest matching call: mcp__workspace__update_plan.' }),
      ).toHaveCount(1);
      await page.screenshot({
        path: info.outputPath(`${host}-${theme}-feedback.png`),
        fullPage: true,
      });
      await page.goto(`/e2e/session-conversation.html?paged&paged-changed&host=${host}`);
      await expect(conversation).toContainText('Showing messages 1–2 of 6.');
      await conversation.getByRole('button', { name: 'Next ↓' }).click();
      await expect(conversation).toContainText(
        'This conversation changed while you were reading it.',
      );
      await expect(conversation.locator('[data-record-id]')).toHaveCount(0);
      await page.screenshot({
        path: info.outputPath(`${host}-${theme}-refusal.png`),
        fullPage: true,
      });
      await conversation.getByRole('button', { name: 'Load conversation again' }).click();
      await expect(conversation).toContainText('Showing messages 1–2 of 6.');
    });
  }
}
