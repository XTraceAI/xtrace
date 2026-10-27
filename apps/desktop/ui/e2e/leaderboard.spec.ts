import { expect, test } from '@playwright/test';

/**
 * The Leaderboard page over synthetic controls (`leaderboard.fixture.tsx`):
 * joining with the keyboard, the leaderboard's refusal under the form, the
 * member's details, and Leave behind its confirmation, in both themes.
 * Nothing is saved or sent.
 */
test.use({ viewport: { width: 1120, height: 720 } });

for (const scheme of ['light', 'dark'] as const) {
  test(`joins and leaves by keyboard in ${scheme}`, async ({ page }, info) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.name));
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/e2e/leaderboard.html');
    await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
    const nickname = page.getByLabel('Nickname', { exact: true });
    await expect(nickname).toBeVisible();
    await expect(page.getByText(/We also keep your email so we can reach you/)).toBeVisible();
    // The leaderboard refuses a taken nickname; its words show under the form.
    await nickname.fill('taken');
    await page.getByLabel('Email').fill('ada@example.com');
    await page.getByLabel('Email').press('Enter');
    await expect(page.getByRole('alert')).toHaveText('That nickname is taken. Try another one.');
    await expect(nickname).toHaveAttribute('aria-invalid', 'true');
    await expect(page.getByLabel('Email')).not.toHaveAttribute('aria-invalid', 'true');
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-join-${scheme}.png`),
    });
    await nickname.fill('ada');
    await nickname.press('Enter');
    const summary = page.getByTestId('leaderboard-summary');
    await expect(summary).toContainText('12.3×');
    const heading = page.getByRole('heading', { level: 1 });
    await expect(heading).toBeFocused();
    await expect(summary).toContainText('#2 on the public page');
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-joined-${scheme}.png`),
    });
    await page.getByRole('button', { name: 'Open leaderboard.xtrace.ai' }).click();
    expect(await page.evaluate(() => (window as { pageOpened?: number }).pageOpened)).toBe(1);
    // Leave asks first; Escape keeps the membership and returns focus.
    const leave = page.getByRole('button', { name: 'Leave leaderboard' });
    await leave.focus();
    await page.keyboard.press('Enter');
    const dialog = page.getByRole('dialog', { name: 'Leave the leaderboard?' });
    await expect(dialog).toContainText(
      'This removes you from the public page and deletes your nickname and email from it.',
    );
    await page.screenshot({ path: info.outputPath(`leaderboard-leave-${scheme}.png`) });
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(leave).toBeFocused();
    await expect(summary).toBeVisible();
    await page.keyboard.press('Enter');
    await dialog.getByRole('button', { name: 'Leave leaderboard' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Join' })).toBeVisible();
    await expect(heading).toBeFocused();
    expect(errors).toEqual([]);
  });
}

test('changes the nickname in place', async ({ page }) => {
  await page.goto('/e2e/leaderboard.html?joined=1');
  await page.getByRole('button', { name: 'Change nickname' }).click();
  const input = page.getByLabel('New nickname');
  await expect(input).toBeFocused();
  await input.fill('Ada_2');
  await input.press('Enter');
  await expect(page.getByTestId('leaderboard-summary')).toContainText('Ada_2');
  await expect(input).toHaveCount(0);
});
