import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 1440, height: 900 } });

test('sidebar geometry, controlled state, theme and Hub dismissal', async ({ page }, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/e2e/sidebar.html');
  await page.evaluate(() => document.fonts.ready);
  const sidebar = page.getByRole('complementary', { name: 'Workspace' });
  await expect(sidebar).toHaveCSS('width', '228px');
  await expect(sidebar).toHaveCSS('height', '900px');
  await expect(sidebar).toHaveCSS('padding-top', '16px');
  const active = page.getByRole('button', { name: 'Dashboard' });
  await expect(active).toHaveCSS('height', '32px');
  await expect(active).toHaveCSS('color', 'rgb(207, 201, 255)');
  await expect(sidebar.locator('.xt-brand-mark img')).toHaveCSS('width', '22px');
  await expect(sidebar.locator('.xt-nav-group h2').first()).toHaveCSS('line-height', '13.775px');
  await expect(sidebar.locator('.xt-host-glyph img')).toHaveCount(2);
  const claudeUsage = sidebar.getByLabel(
    'Claude account usage: 0% remaining · Weekly · limit reached',
  );
  await expect(claudeUsage).toContainText('Limit reached');
  await expect(claudeUsage.locator('.xt-account-summary-track > span')).toHaveCSS('width', '0px');
  const codexUsage = sidebar.getByLabel('Codex account usage: 74% remaining · Weekly');
  await expect(codexUsage).toBeVisible();
  await expect(codexUsage.locator('.xt-account-summary-track > span')).toHaveAttribute(
    'style',
    'width: 74%;',
  );
  for (const logo of await sidebar.locator('img').all())
    await expect(logo).toHaveJSProperty('complete', true);
  expect(
    await sidebar
      .locator('img')
      .evaluateAll((es) => es.every((e) => (e as HTMLImageElement).naturalWidth > 0)),
  ).toBe(true);
  const footer = sidebar.locator('.xt-sidebar-status');
  const settings = sidebar.getByRole('button', { name: 'Settings' });
  const footerBox = (await footer.boundingBox())!;
  const settingsBox = (await settings.boundingBox())!;
  expect(settingsBox.y).toBeGreaterThanOrEqual(footerBox.y);
  expect(settingsBox.y + settingsBox.height).toBeLessThanOrEqual(footerBox.y + footerBox.height);
  expect(settingsBox.x + settingsBox.width).toBeLessThanOrEqual(228 - 12);
  await page
    .getByTestId('sidebar-preview')
    .screenshot({ path: info.outputPath('sidebar-dark.png') });
  await page.getByRole('button', { name: 'Sessions' }).click();
  await expect(page.getByRole('button', { name: 'Sessions' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.getByRole('button', { name: 'Switch to light appearance' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(claudeUsage).toBeVisible();
  await active.click();
  await page.mouse.move(600, 500);
  await page
    .getByTestId('sidebar-preview')
    .screenshot({ path: info.outputPath('sidebar-light.png') });
  const trigger = page.getByRole('button', { name: 'XTrace Hub', exact: true });
  await expect(trigger).toHaveText('Cloud and Team');
  await expect(trigger).toHaveCSS('width', '204px');
  const hub = page.getByRole('dialog', { name: 'Connect this desktop to your team.' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(hub).toBeVisible();
  await expect(hub).toHaveCSS('width', '264px');
  // Visibility can precede the positioner's final placement. Measure the popup
  // and its sidebar anchor together, keeping the original alignment tolerance.
  await expect
    .poll(() =>
      hub.evaluate((element) => {
        const sidebarBounds = document.querySelector('.xt-sidebar')!.getBoundingClientRect();
        const footerBounds = document.querySelector('.xt-sidebar-status')!.getBoundingClientRect();
        const hubBounds = element.getBoundingClientRect();
        return {
          xOffset: hubBounds.x - (sidebarBounds.x + sidebarBounds.width - 12 + 14),
          bottomOffset: hubBounds.bottom - (footerBounds.bottom + 6),
        };
      }),
    )
    .toEqual({ xOffset: 0, bottomOffset: expect.closeTo(0, 0) });
  await expect(hub).toHaveCSS('background-color', 'rgb(241, 241, 245)');
  await expect(page.getByRole('button', { name: 'Connect XTrace Hub' })).toBeDisabled();
  await page.screenshot({ path: info.outputPath('sidebar-hub.png') });
  await page.keyboard.press('Escape');
  await expect(hub).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.getByRole('button', { name: 'Close XTrace Hub' }).click();
  await expect(hub).toBeHidden();
  await trigger.click();
  await page.getByTestId('outside').click();
  await expect(hub).toBeHidden();
  await trigger.click();
  await page.getByRole('button', { name: 'Sessions' }).click();
  await expect(hub).toBeHidden();
  await trigger.click();
  await page.getByRole('button', { name: 'Switch to dark appearance' }).click();
  await expect(hub).toBeHidden();
  await page.getByRole('button', { name: 'Switch to light appearance' }).click();
  await active.click();
  const capture = page.getByRole('button', { name: 'Capture status', exact: true });
  const coverage = page.getByRole('dialog', { name: 'Capture by surface' });
  await expect(coverage).toBeHidden();
  await capture.focus();
  await page.keyboard.press('Enter');
  await expect(coverage).toBeVisible();
  await expect(coverage.getByText('Codex · cli').locator('..')).toContainText('Capturing');
  await expect(coverage.getByText('Codex · desktop').locator('..')).toContainText('Not capturing');
  await expect(coverage.getByText('Other host · new-surface').locator('..')).toContainText(
    'Unknown',
  );
  await page.keyboard.press('Escape');
  await expect(coverage).toBeHidden();
  await expect(capture).toBeFocused();
  await capture.click();
  await trigger.click();
  await expect(coverage).toBeHidden();
  await expect(hub).toBeVisible();
  // And the reverse: opening the status panel closes the Hub.
  await capture.click();
  await expect(hub).toBeHidden();
  await expect(coverage).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Reserve native controls' }).click();
  await expect(sidebar).toHaveCSS('padding-top', '74px');
  await page.screenshot({ path: info.outputPath('brand-sizes-native-inset.png') });
  expect(errors).toEqual([]);
});

test('remaining bars fit the compact sidebar at 1120×720 in both themes', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/e2e/sidebar.html');
  await page.getByTestId('sidebar-preview').evaluate((element) => {
    element.style.height = '720px';
  });
  const sidebar = page.getByRole('complementary', { name: 'Workspace' });
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await expect(
      sidebar.getByLabel('Claude account usage: 0% remaining · Weekly · limit reached'),
    ).toBeVisible();
    const codex = sidebar.getByLabel('Codex account usage: 74% remaining · Weekly');
    await expect(codex.locator('.xt-account-summary-track > span')).toHaveAttribute(
      'style',
      'width: 74%;',
    );
    expect(await sidebar.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(
      true,
    );
    expect(await sidebar.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
      true,
    );
    await sidebar.screenshot({ path: info.outputPath(`remaining-${scheme}-1120x720.png`) });
  }
});

test('a stale or signed-out usage row is one warning line that fits the sidebar', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  const issues = [
    'none',
    'reading',
    'signed_out',
    'no_login',
    'credential_access_required',
    'credential_unavailable',
    'unauthorized',
    'rate_limited',
    'timeout',
    'network',
    'invalid_response',
    'source_unavailable',
  ];
  const cases = [
    ...issues.map((issue) => ({ host: 'claude', value: issue })),
    ...issues.map((issue) => ({ host: 'claude', value: `reset-${issue}` })),
    { host: 'claude', value: 'missing-signed_out' },
    { host: 'codex', value: 'none' },
    { host: 'codex', value: 'reset-invalid_response' },
  ];
  for (const { host, value } of cases) {
    const name = `${host}=${value}`;
    await page.goto(`/e2e/sidebar.html?${name}`);
    const sidebar = page.getByRole('complementary', { name: 'Workspace' });
    const group = sidebar.locator('.xt-account-provider-group', {
      has: page.locator(`.xt-host-${host}`),
    });
    const warning = group.locator('.xt-account-row-warning');
    await expect(warning, name).toBeVisible();
    // Only the warning line is under the numbers (a signed-out line is a
    // button after the summary), on one line, not cut off.
    const sub = group.locator('summary .xt-account-sub');
    if (value.endsWith('signed_out')) await expect(sub, name).toHaveCount(0);
    else await expect(sub, name).toHaveText((await warning.textContent()) ?? '');
    const fits = await warning.evaluate((element) => ({
      whole: element.scrollWidth <= element.clientWidth,
      lines: Math.round(
        element.getBoundingClientRect().height / parseFloat(getComputedStyle(element).lineHeight),
      ),
    }));
    expect(fits, name).toEqual({ whole: true, lines: 1 });
  }
});

test('the signed-out line signs in by click or key without opening the row', async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 720 });
  const signIns = () => page.evaluate(() => (window as unknown as { signIns: string[] }).signIns);
  await page.goto('/e2e/sidebar.html?claude=signed_out');
  const sidebar = page.getByRole('complementary', { name: 'Workspace' });
  const row = sidebar.locator('details.xt-host-claude');
  const line = sidebar.getByRole('button', {
    name: 'Signed out · claude auth login: sign in in Terminal',
  });
  await expect(line).toHaveText('Signed out · claude auth login');
  await line.click();
  expect(await signIns()).toEqual(['claude']);
  expect(await row.evaluate((element) => (element as HTMLDetailsElement).open)).toBe(false);
  // From the row by Tab, then Enter and Space.
  await row.locator('summary').focus();
  await page.keyboard.press('Tab');
  await expect(line).toBeFocused();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Space');
  await expect.poll(signIns).toEqual(['claude', 'claude', 'claude']);
  expect(await row.evaluate((element) => (element as HTMLDetailsElement).open)).toBe(false);
  // Opened, the row offers the same sign-in in its details.
  await row.locator('summary').click();
  await expect(line).toHaveCount(0);
  await row.getByRole('button', { name: 'Sign in in Terminal' }).click();
  await expect.poll(signIns).toHaveLength(4);
  // A failure shows on the same focused button, on one line, then goes.
  await page.goto('/e2e/sidebar.html?claude=signed_out&signin=fail');
  const button = sidebar.locator('.xt-account-sign-in-row button');
  await button.focus();
  await page.keyboard.press('Enter');
  await expect(button).toHaveText('Claude Code not found');
  await expect(button).toBeFocused();
  await expect(sidebar.locator('.xt-account-provider-group [role="status"]')).toHaveText(
    'Claude Code not found',
  );
  expect(await button.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  await expect(button).toHaveText('Signed out · claude auth login', { timeout: 10_000 });
  await expect(button).toBeFocused();
  // While the app waits for the sign-in, the line says so and opens Terminal again.
  await page.goto('/e2e/sidebar.html?claude=signed_out&watch=1');
  const waiting = sidebar.getByRole('button', {
    name: 'Waiting for sign-in…: open Terminal again',
  });
  await expect(waiting).toHaveText('Waiting for sign-in…');
  expect(await waiting.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
    true,
  );
  await waiting.click();
  await expect.poll(signIns).toEqual(['claude']);
  await row.locator('summary').click();
  await expect(row.getByRole('button', { name: 'Open Terminal again' })).toBeVisible();
});
