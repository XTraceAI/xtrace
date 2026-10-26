import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

for (const router of ['browser', 'hash'] as const) {
  test(`${router} history buttons and shortcuts preserve filters and clear abandoned pages`, async ({
    page,
  }) => {
    await page.goto(router === 'hash' ? '/e2e/page-history.html#/dashboard' : '/dashboard');
    const back = page.getByRole('button', { name: 'Back', exact: true });
    const forward = page.getByRole('button', { name: 'Forward', exact: true });
    await expect(back).toBeDisabled();
    await expect(forward).toBeDisabled();
    // A bounded shortcut must not return to the browser's prior document.
    await page.keyboard.press('Meta+[');
    await expect(page.getByTestId('dashboard-summary')).toBeVisible();
    await page.getByRole('button', { name: 'Sessions', exact: true }).click();
    await page.getByLabel('Search sessions').fill(fixture.sessions[0].rows[0].id);
    await expect(page).toHaveURL(/q=/);
    await page
      .getByRole('link', { name: `Open session ${fixture.sessions[0].rows[0].id}` })
      .click();
    await expect(forward).toBeDisabled();
    await back.click();
    await expect(page.getByLabel('Search sessions')).toHaveValue(fixture.sessions[0].rows[0].id);
    await expect(forward).toBeEnabled();
    // Replacing filters while in the middle preserves the Forward destination.
    await page.getByLabel('Search sessions').fill('missing');
    await expect(page).toHaveURL(/q=missing/);
    await forward.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByText('Measured over the last 7 days', { exact: true })).toBeVisible();
    await page.keyboard.press('Meta+[');
    await expect(page.getByLabel('Search sessions')).toHaveValue('missing');
    await page.keyboard.press('Meta+[');
    await expect(page.getByTestId('dashboard-summary')).toBeVisible();
    await expect(back).toBeDisabled();
    await page.keyboard.press('Meta+]');
    await expect(page.getByLabel('Search sessions')).toHaveValue('missing');
    await page.getByRole('button', { name: 'Scores', exact: true }).click();
    await expect(forward).toBeDisabled();
    await page.keyboard.press('Meta+,');
    await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
    await back.click();
    await expect(page.getByRole('heading', { name: 'Scores', exact: true })).toBeVisible();
  });
}
