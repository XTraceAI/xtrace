import { expect, test } from '@playwright/test';
import { filters, session, title } from './mounted-session-warnings.data';
import { answer, dismissedAnswer, warning, second } from './session-warnings.data';

for (const theme of ['light', 'dark'] as const) {
  test(`Sessions markers open the warning panel with selection and readback in ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1120, height: 800 });
    await page.emulateMedia({ colorScheme: theme });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto('/e2e/mounted-session-warnings.html');
    const marker = page.getByRole('link', { name: answer.count_label! });
    await expect(marker).toBeVisible();
    await expect(marker).toHaveAttribute('href', `/sessions/${session.id}?${filters}&warning=`);
    const row = page.getByRole('row').filter({ has: marker });
    expect(Math.round((await row.boundingBox())!.height)).toBe(40);
    await marker.focus();
    await expect(page.getByTestId('warning-marker-tip')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('warning-marker-tip')).toBeHidden();
    await page.keyboard.press('Enter');
    const summary = page.getByTestId('warnings-summary');
    await expect(summary).toHaveText(answer.summary);
    await expect(summary).toBeFocused();
    await expect(page.getByRole('link', { name: '← All sessions' })).toHaveAttribute(
      'href',
      `/sessions?${filters}`,
    );
    await expect(page.getByRole('list', { name: 'Selected warning' })).toHaveCount(0);
    await expect(
      page.getByText('Synthetic conversation message 1.', { exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: warning.name }).focus();
    await page.keyboard.press('Enter');
    const selected = page.getByRole('list', { name: 'Selected warning' });
    await expect(selected.getByTestId('warning-item')).toBeFocused();
    await expect(selected).toContainText(warning.reason);
    await expect(page.getByTestId('fixture-location')).toContainText(
      `warning=${encodeURIComponent(warning.id)}`,
    );
    await page.keyboard.press('Tab');
    await expect(selected.getByRole('button', { name: 'Dismiss' })).toBeFocused();
    await page.keyboard.press('Enter');
    const undo = selected.getByRole('button', { name: 'Undo' });
    await expect(undo).toBeFocused();
    await expect(summary).toHaveText(dismissedAnswer.summary);
    await page.keyboard.press('Enter');
    await expect(selected.getByRole('button', { name: 'Dismiss' })).toBeFocused();
    await page.getByRole('button', { name: second.name }).click();
    await expect(selected).toContainText(second.reason);
    await expect(page.getByRole('button', { name: second.name })).toHaveAttribute(
      'aria-current',
      'true',
    );
    await page.getByText(answer.checks_summary, { exact: true }).click();
    await expect(page.getByText('Supplied incomplete check reason.')).toBeVisible();
    const body = page.getByRole('region', { name: 'Conversation content' });
    const boxes = await body.evaluate((node) => ({
      height: node.clientHeight,
      scrolls: node.scrollHeight > node.clientHeight,
      fits: document.documentElement.scrollWidth <= innerWidth,
    }));
    expect(boxes.height).toBeGreaterThan(100);
    expect(boxes.scrolls).toBe(true);
    expect(boxes.fits).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`mounted-warnings-${theme}.png`) });
    await page.getByRole('link', { name: '← All sessions' }).click();
    await expect(page.getByRole('searchbox', { name: 'Search sessions' })).toHaveValue(title);
    await expect(page.getByTestId('fixture-location')).toHaveText(`/sessions?${filters}`);
    expect(errors).toEqual([]);
  });
}

test('an exact warning link selects and focuses its warning on arrival', async ({ page }) => {
  const path = `/sessions/${session.id}?${filters}&warning=${encodeURIComponent(second.id)}`;
  await page.goto(`/e2e/mounted-session-warnings.html?path=${encodeURIComponent(path)}`);
  const selected = page.getByRole('list', { name: 'Selected warning' });
  await expect(selected).toContainText(second.reason);
  await expect(selected.getByTestId('warning-item')).toBeFocused();
  await expect(page.getByRole('button', { name: second.name })).toHaveAttribute(
    'aria-current',
    'true',
  );
});
