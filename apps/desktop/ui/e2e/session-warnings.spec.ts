import { expect, test } from '@playwright/test';
import { answer, dismissedAnswer, warning } from './session-warnings.data';

for (const theme of ['light', 'dark'] as const) {
  test(`shared warning controls, focus and tooltips work in ${theme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/e2e/session-warnings.html');
    await expect(page.getByTestId('warnings-summary')).toHaveText(answer.summary);
    const marker = page.getByRole('link', { name: answer.count_label! });
    await marker.hover();
    const tip = page.getByTestId('warning-marker-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText(warning.reason);
    expect(
      await tip.evaluate((node) => node.closest('[data-theme]')?.getAttribute('data-theme')),
    ).toBe(theme);
    const tipStyle = await tip.evaluate((node) => {
      const style = getComputedStyle(node);
      return { background: style.backgroundColor, color: style.color };
    });
    expect(tipStyle.background).not.toBe('rgba(0, 0, 0, 0)');
    expect(tipStyle.color).not.toBe(tipStyle.background);
    await page.keyboard.press('Escape');
    await expect(tip).toBeHidden();
    await page.mouse.move(0, 0);
    await marker.focus();
    await expect(tip).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(tip).toBeHidden();

    const dot = page.getByRole('link', { name: warning.name });
    await dot.focus();
    await expect(page.getByTestId('warning-tip')).toBeVisible();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('status', { name: 'Current location' })).toHaveText(
      `/sessions/synthetic-session?warning=${encodeURIComponent(warning.id)}`,
    );
    const buttonDot = page.getByRole('button', { name: 'Open synthetic warning detail' });
    await buttonDot.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('status', { name: 'Opened count' })).toHaveText('1');
    await expect(page.getByRole('status', { name: 'Current location' })).toHaveText(
      `/sessions/synthetic-session?warning=${encodeURIComponent(warning.id)}`,
    );

    await page.getByRole('button', { name: 'Select supplied warning' }).click();
    const selected = page.getByTestId('warning-item').filter({ hasText: warning.reason });
    await expect(selected).toBeFocused();
    await expect(selected).toHaveAttribute('data-highlighted', 'true');
    const dismiss = selected.getByRole('button', { name: 'Dismiss', exact: true });
    await page.keyboard.press('Tab');
    await expect(dismiss).toBeFocused();
    expect(await dismiss.evaluate((node) => getComputedStyle(node).outlineStyle)).toBe('solid');
    await page.keyboard.press('Enter');
    const undo = page.getByTestId('dismissed-warning').getByRole('button', { name: 'Undo' });
    await expect(undo).toBeFocused();
    await expect(page.getByTestId('warnings-summary')).toHaveText(dismissedAnswer.summary);
    await page.keyboard.press('Enter');
    await expect(page.getByTestId('dismissed-warning')).toHaveCount(0);
    await expect(selected.getByRole('button', { name: 'Dismiss', exact: true })).toBeFocused();

    await page.getByText(answer.checks_summary, { exact: true }).click();
    await expect(page.getByText('Supplied incomplete check reason.')).toBeVisible();
    await expect(page.getByTestId('warnings-summary')).toHaveAttribute('data-incomplete', 'true');
    await page.setViewportSize({ width: 360, height: 720 });
    expect(await page.locator('body').evaluate((node) => node.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.setViewportSize({ width: 960, height: 720 });
    await page.screenshot({ path: test.info().outputPath(`shared-warnings-${theme}.png`) });
  });
}
