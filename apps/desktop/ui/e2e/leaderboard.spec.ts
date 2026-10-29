import { expect, test } from '@playwright/test';

/**
 * The Leaderboard page over synthetic controls (`leaderboard.fixture.tsx`):
 * the locked page (made-up rows blurred behind the join card), joining with
 * the keyboard through the emailed-code step, the leaderboard's refusal under
 * the form, the member's details over the public list with their row marked,
 * and Leave behind its confirmation, in both themes. Nothing is saved or sent.
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
    await expect(page.getByText(/we email you a code to confirm it's you/)).toBeVisible();
    // The leaderboard refuses the email; its words show under the form.
    await nickname.fill('taken');
    await page.getByLabel('Email').fill('ada');
    await page.getByLabel('Email').press('Enter');
    await expect(page.getByRole('alert')).toHaveText('That email address does not look right.');
    await expect(page.getByLabel('Email')).toHaveAttribute('aria-invalid', 'true');
    await expect(nickname).not.toHaveAttribute('aria-invalid', 'true');
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-join-${scheme}.png`),
    });
    await page.getByLabel('Email').fill('ada@example.com');
    await page.getByLabel('Email').press('Enter');
    // The code step: the code field takes the focus; a wrong code is marked.
    const code = page.getByLabel('Code', { exact: true });
    await expect(page.getByTestId('leaderboard-code-sent')).toHaveText(
      'We sent a 6-digit code to ada@example.com. Enter it below. It expires in 15 minutes.',
    );
    await expect(code).toBeFocused();
    await page.keyboard.type('000000');
    await page.keyboard.press('Enter');
    await expect(page.getByRole('alert')).toHaveText(
      'That code is not right. Check the email and try again.',
    );
    await expect(code).toHaveAttribute('aria-invalid', 'true');
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-code-${scheme}.png`),
    });
    // A right code with a taken nickname: the same code, another nickname.
    await code.fill('123456');
    await code.press('Enter');
    const chosen = page.getByLabel('Nickname', { exact: true });
    await expect(page.getByRole('alert')).toHaveText(
      'That nickname is taken. Choose another one and confirm again.',
    );
    await expect(chosen).toBeFocused();
    await expect(chosen).toHaveAttribute('aria-invalid', 'true');
    await expect(code).toHaveValue('123456');
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-nickname-${scheme}.png`),
    });
    await chosen.fill('ada');
    await chosen.press('Enter');
    const summary = page.getByTestId('leaderboard-summary');
    await expect(summary).toContainText('12.3×');
    const heading = page.getByRole('heading', { level: 1 });
    await expect(heading).toBeFocused();
    await expect(summary).toContainText('#2 on the public page');
    // The real list replaces the blurred one, with the member's row marked.
    const table = page.getByRole('table', { name: 'Public leaderboard' });
    await expect(table.getByRole('row')).toHaveCount(6);
    await expect(table.locator('tr[data-you]')).toHaveText(/^🥈2ada\s*You \(this is you\)12\.3×$/);
    await expect(page.getByTestId('leaderboard-sample')).toHaveCount(0);
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

test('goes back from the code step to change the email, keeping the nickname', async ({ page }) => {
  await page.goto('/e2e/leaderboard.html');
  await page.getByLabel('Nickname', { exact: true }).fill('ada');
  await page.getByLabel('Email').fill('ada@exmaple.com');
  await page.getByRole('button', { name: 'Join' }).click();
  await page.getByRole('button', { name: 'Send a new code' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'A new code was sent' })).toBeVisible();
  await page.getByRole('button', { name: 'Use a different email' }).click();
  const email = page.getByLabel('Email');
  await expect(email).toBeFocused();
  await expect(page.getByLabel('Nickname', { exact: true })).toHaveValue('ada');
  await expect(page.getByLabel('Code', { exact: true })).toHaveCount(0);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`draws the locked page and the joined list in ${scheme}`, async ({ page }, info) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.name));
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.goto('/e2e/leaderboard.html');
    await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
    const card = page.getByRole('region', { name: 'Join the leaderboard' });
    await expect(card).toBeVisible();
    const backdrop = page.getByTestId('leaderboard-sample');
    await expect(backdrop).toHaveAttribute('aria-hidden', 'true');
    await expect(backdrop).toHaveAttribute('inert', '');
    await expect(backdrop.locator('tbody tr')).toHaveCount(36);
    // Blurred, dimmed, and out of reach of the pointer and of selection.
    const drawn = await backdrop.evaluate((element) => {
      const frame = getComputedStyle(element);
      const sample = getComputedStyle(element.firstElementChild!);
      return {
        filter: sample.filter,
        opacity: Number(sample.opacity),
        pointer: frame.pointerEvents,
        select: frame.userSelect || frame.webkitUserSelect,
        animation: sample.animationName,
      };
    });
    expect(drawn.filter).toBe('blur(6px)');
    expect(drawn.opacity).toBeLessThan(1);
    expect(drawn.pointer).toBe('none');
    expect(drawn.select).toBe('none');
    expect(drawn.animation).toBe('none');
    // The list fills the rest of the page, and the card sits centered in
    // front of it.
    const area = (await backdrop.boundingBox())!;
    const front = (await card.boundingBox())!;
    const outlet = await page.locator('.xt-shell-outlet').evaluate((element) => {
      const box = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return {
        right: box.right - parseFloat(style.paddingRight),
        bottom: box.bottom - parseFloat(style.paddingBottom),
        scrolls: element.scrollHeight > element.clientHeight,
      };
    });
    expect(Math.abs(area.x + area.width - outlet.right)).toBeLessThanOrEqual(1);
    expect(Math.abs(area.y + area.height - outlet.bottom)).toBeLessThanOrEqual(1);
    expect(outlet.scrolls).toBe(false);
    const centerX = front.x + front.width / 2 - (area.x + area.width / 2);
    const centerY = front.y + front.height / 2 - (area.y + area.height / 2);
    expect(Math.abs(centerX)).toBeLessThanOrEqual(1);
    expect(Math.abs(centerY)).toBeLessThanOrEqual(1);
    expect(front.x).toBeGreaterThan(area.x);
    expect(front.x + front.width).toBeLessThan(area.x + area.width);
    expect(front.y).toBeGreaterThanOrEqual(area.y);
    expect(front.y + front.height).toBeLessThanOrEqual(area.y + area.height + 1);
    // A click on the list's corner lands on nothing in it.
    const hit = await page.evaluate(
      ({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-testid="leaderboard-sample"]'),
      { x: area.x + 12, y: area.y + 12 },
    );
    expect(hit).toBeNull();
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-locked-${scheme}.png`),
    });
    // Tab moves from the page heading straight into the card's fields.
    await page.getByRole('heading', { level: 1 }).focus();
    await page.keyboard.press('Tab');
    await expect(page.getByLabel('Nickname', { exact: true })).toBeFocused();
    // The disclosure opens from the keyboard.
    const shared = card.locator('summary', { hasText: 'What is shared' });
    await shared.focus();
    await page.keyboard.press('Enter');
    await expect(card.getByText(/sees your IP address/)).toBeVisible();
    await page.keyboard.press('Enter');
    await expect(card.getByText(/sees your IP address/)).toBeHidden();
    expect(await page.evaluate(() => (window as { tableReads?: number }).tableReads ?? 0)).toBe(0);
    // Joined: the real list under the controls, with the member's row marked.
    await page.goto('/e2e/leaderboard.html?joined=1');
    const table = page.getByRole('table', { name: 'Public leaderboard' });
    await expect(table.locator('tbody tr')).toHaveCount(5);
    const you = table.locator('tr[data-you]');
    await expect(you).toContainText('ada');
    await expect(you.getByText('You', { exact: true })).toBeVisible();
    await expect(page.getByText('The public list updates every minute or two.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Change nickname' })).toBeVisible();
    await page.locator('.xt-shell-main').screenshot({
      path: info.outputPath(`leaderboard-table-${scheme}.png`),
    });
    expect(errors).toEqual([]);
  });
}

test('keeps the membership controls when the public list cannot be read', async ({ page }) => {
  await page.goto('/e2e/leaderboard.html?joined=1&board=error');
  await expect(page.getByRole('alert')).toHaveText(
    'Could not reach the leaderboard. Check your internet connection and try again.',
  );
  await expect(page.getByTestId('leaderboard-summary')).toContainText('ada');
  await expect(page.getByRole('button', { name: 'Leave leaderboard' })).toBeVisible();
  await page.goto('/e2e/leaderboard.html?joined=1&board=empty');
  await expect(page.getByText('No one is listed yet.')).toBeVisible();
  await expect(page.getByRole('table')).toHaveCount(0);
});
