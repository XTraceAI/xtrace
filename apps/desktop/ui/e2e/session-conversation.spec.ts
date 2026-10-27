import { expect, test, type Page } from '@playwright/test';

async function capture(page: Page, path: string, width: number, height: number) {
  const viewport = page.viewportSize()!;
  await page.screenshot({ path: path.replace('.png', '-viewport.png'), fullPage: true });
  const outlet = page.locator('.xt-shell-outlet');
  const scroll = await outlet.evaluate((node) => node.scrollTop);
  await page.setViewportSize({ width, height });
  await outlet.evaluate((node) => node.scrollTo(0, 0));
  await page.screenshot({ path, fullPage: true });
  await page.setViewportSize(viewport);
  await outlet.evaluate((node, scroll) => node.scrollTo(0, scroll), scroll);
}

test.use({ timezoneId: 'UTC' });
for (const theme of ['light', 'dark'] as const) {
  test(`keeps one conversation while navigating repeated activity, ${theme}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 1120, height: 900 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/e2e/session-conversation.html');
    const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
    const navigation = conversation.getByRole('navigation', { name: 'Repeated activity' });
    const next = navigation.getByRole('button', { name: 'Next ↓' });
    const previous = navigation.getByRole('button', { name: '↑ Previous' });
    await expect(navigation).toContainText('2 notices');
    await expect(previous).toHaveAttribute('aria-disabled', 'true');
    await expect(conversation.locator('[data-highlighted]')).toHaveCount(0);
    await expect(conversation.locator('.xt-conversation-repeat')).toHaveCount(2);
    await expect(page.getByText('Hands-off stretches', { exact: true })).not.toBeVisible();
    await expect(conversation.getByText(/Please rename/)).toBeVisible();
    await expect(conversation.getByText('Keep the saved settings, too.')).toBeVisible();
    const ids = await conversation
      .locator('[data-record-id]')
      .evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-record-id')));
    const user = conversation.locator('[data-record-id="prompt"]');
    expect(
      await user.evaluate((node) => parseFloat(getComputedStyle(node).borderRadius)),
    ).toBeGreaterThan(0);
    expect(
      await conversation
        .locator('[data-record-id="answer"]')
        .evaluate((node) => getComputedStyle(node).borderTopWidth),
    ).toBe('0px');
    if (process.env.CONVERSATION_SHOTS)
      await capture(
        page,
        `${process.env.CONVERSATION_SHOTS}/conversation-${testInfo.project.name}-${theme}.png`,
        1120,
        1550,
      );
    await next.focus();
    await page.keyboard.press('Enter');
    const target = conversation.locator('.xt-tool-card[data-block-id="tools:0"]');
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect(target).toBeInViewport();
    await expect(next).toBeFocused();
    if (process.env.CONVERSATION_SHOTS)
      await capture(
        page,
        `${process.env.CONVERSATION_SHOTS}/conversation-${testInfo.project.name}-${theme}-selected.png`,
        1120,
        2200,
      );
    const group = conversation.getByRole('button', { name: /Tool activity/ });
    await group.click();
    await expect(group).toHaveAttribute('aria-expanded', 'false');
    await group.click();
    await expect(conversation.getByText('Synthetic configuration content.')).not.toBeVisible();
    await conversation.locator('[data-record-id="result"]').getByRole('button').click();
    await expect(conversation.getByText('Synthetic configuration content.')).toBeVisible();
    await next.focus();
    await page.keyboard.press('Enter');
    await expect(conversation.locator('.xt-tool-card[data-block-id="mixed:3"]')).toHaveAttribute(
      'data-highlighted',
      'true',
    );
    await expect(next).toBeFocused();
    await expect(next).toHaveAttribute('aria-disabled', 'true');
    await previous.focus();
    await page.keyboard.press('Enter');
    await expect(previous).toBeFocused();
    await expect(previous).toHaveAttribute('aria-disabled', 'true');
    expect(
      await conversation
        .locator('[data-record-id]')
        .evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-record-id'))),
    ).toEqual(ids);
    await page.setViewportSize({ width: 760, height: 900 });
    await expect(navigation).toBeVisible();
    const overflow = await conversation.evaluate((node) => node.scrollWidth - node.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    if (process.env.CONVERSATION_SHOTS)
      await capture(
        page,
        `${process.env.CONVERSATION_SHOTS}/conversation-${testInfo.project.name}-${theme}-narrow.png`,
        760,
        2600,
      );
    await page.locator('.xt-session-stretches > summary').click();
    await expect(
      page
        .locator('[role="status"][aria-live="polite"]')
        .filter({ hasText: 'Showing the earliest matching call: Read.' }),
    ).toHaveCount(1);
  });
}

test('keeps keyboard focus when a single repeated-activity notice reaches both boundaries', async ({
  page,
}) => {
  await page.goto('/e2e/session-conversation.html?single-repeat');
  const navigation = page.getByRole('navigation', { name: 'Repeated activity' });
  const next = navigation.getByRole('button', { name: 'Next ↓' });
  await expect(navigation).toContainText('1 notice');
  await next.focus();
  await page.keyboard.press('Enter');
  await expect(navigation).toContainText('1 of 1');
  await expect(next).toHaveAttribute('aria-disabled', 'true');
  await expect(next).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(navigation).toContainText('1 of 1');
  await expect(next).toBeFocused();
});

for (const theme of ['light', 'dark'] as const) {
  test(`reads bounded conversation portions and exact repeated activity, ${theme}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 1120, height: 900 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/e2e/session-conversation.html?paged');
    const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
    const navigation = conversation.getByRole('navigation', { name: 'Repeated activity' });
    const next = navigation.getByRole('button', { name: 'Next ↓' });
    const previous = navigation.getByRole('button', { name: '↑ Previous' });
    const later = conversation.getByRole('button', { name: 'Later messages' });
    const earlier = conversation.getByRole('button', { name: 'Earlier messages' });
    await expect(conversation).toContainText('Showing messages 1–2 of 6.');
    await expect(navigation).toContainText('3 notices');
    await expect(conversation.locator('.xt-conversation-repeat')).toHaveCount(0);
    await expect(conversation.locator('[data-record-id="prompt"]')).toHaveClass(
      /xt-conversation-turn/,
    );
    await later.focus();
    await page.keyboard.press('Enter');
    await expect(conversation).toContainText('Showing messages 3–6 of 6.');
    await expect(later).toBeFocused();
    await expect(conversation.locator('.xt-conversation-repeat')).toHaveCount(2);
    await earlier.focus();
    await page.keyboard.press('Enter');
    await expect(conversation).toContainText('Showing messages 1–2 of 6.');
    await expect(earlier).toBeFocused();
    await next.focus();
    await page.keyboard.press('Enter');
    await expect(navigation).toContainText('1 of 3');
    await expect(conversation.locator('.xt-tool-card[data-block-id="tools:0"]')).toHaveAttribute(
      'data-highlighted',
      'true',
    );
    await expect(next).toBeFocused();
    await page.getByText(/Agent activity · Last/).click();
    await next.focus();
    await page.keyboard.press('Enter');
    const target = conversation.locator('.xt-tool-card[data-block-id="mixed:3"]');
    await expect(navigation).toContainText('2 of 3');
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect(target).toBeInViewport();
    await expect(next).toBeFocused();
    await expect(conversation.locator('.xt-conversation-repeat[data-current="true"]')).toHaveCount(
      1,
    );
    await expect(
      page
        .locator('[role="status"][aria-live="polite"]')
        .filter({ hasText: 'Showing the earliest matching call: mcp__workspace__update_plan.' }),
    ).toHaveCount(1);
    if (process.env.PAGED_CONVERSATION_SHOTS)
      await page.screenshot({
        path: `${process.env.PAGED_CONVERSATION_SHOTS}/large-codex-section-${testInfo.project.name}-${theme}.png`,
        fullPage: true,
      });
    await page.keyboard.press('Enter');
    await expect(navigation).toContainText('3 of 3');
    await expect(conversation).toContainText('The saved tool call could not be found');
    await expect(conversation.locator('[data-highlighted]')).toHaveCount(0);
    await expect(next).toBeFocused();
    await previous.focus();
    await page.keyboard.press('Enter');
    await expect(navigation).toContainText('2 of 3');
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect(previous).toBeFocused();
    await page.setViewportSize({ width: 760, height: 900 });
    expect(
      await conversation.evaluate((node) => node.scrollWidth - node.clientWidth),
    ).toBeLessThanOrEqual(1);
  });
}

test('retains a changed-source refusal until the bounded conversation is explicitly reloaded', async ({
  page,
}) => {
  await page.goto('/e2e/session-conversation.html?paged&paged-changed');
  const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
  await expect(conversation).toContainText('Showing messages 1–2 of 6.');
  await conversation.getByRole('button', { name: 'Next ↓' }).click();
  await expect(conversation).toContainText('This conversation changed while you were reading it.');
  await expect(
    conversation.getByText('Please rename the sample app and keep its project folder name.', {
      exact: true,
    }),
  ).toHaveCount(0);
  await conversation.getByRole('button', { name: 'Load conversation again' }).click();
  await expect(conversation).toContainText('Showing messages 1–2 of 6.');
});
