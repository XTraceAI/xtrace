import { expect, test } from '@playwright/test';

for (const size of [
  { width: 1120, height: 720 },
  { width: 1120, height: 900 },
  { width: 1440, height: 900 },
]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`bounded session ${size.width}x${size.height} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/e2e/session-layout.html');
      const owner = page.getByRole('region', { name: 'Conversation content' });
      await expect(owner.getByText('Synthetic message 0.', { exact: false })).toBeVisible();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.getByText('1,200 indexed records · whole session')).toBeVisible();
      await expect(page.getByText('Last 30 days', { exact: true })).toBeVisible();
      await expect(owner.getByText(/has been written to/)).toBeVisible();
      await expect(owner.getByText(/no longer on this Mac/)).toBeVisible();
      const shape = () =>
        owner.evaluate((element) => ({
          height: element.clientHeight,
          scroll: element.scrollTop,
          outer: element.closest('.xt-shell-outlet')!.scrollTop,
          width: document.documentElement.scrollWidth,
        }));
      const before = await shape();
      expect(before.height).toBeGreaterThan(200);
      expect(before.width).toBeLessThanOrEqual(size.width);
      await owner.focus();
      await page.keyboard.press('End');
      await expect
        .poll(() =>
          owner.evaluate(
            (element) => element.scrollHeight - element.clientHeight - element.scrollTop,
          ),
        )
        .toBe(0);
      expect((await shape()).outer).toBe(before.outer);
      await owner.press('Home');
      await expect.poll(async () => (await shape()).scroll).toBe(0);
      await page.locator('.xt-session-detail-meta > summary').click();
      await expect(page.getByText('synthetic-layout', { exact: true })).toBeVisible();
      await page.locator('.xt-session-detail-meta > summary').click();
      const prs = page.locator('.xt-session-detail-prs > summary');
      await prs.focus();
      await page.keyboard.press('Enter');
      const links = page.getByRole('list', { name: 'Linked PRs' });
      await expect(links.getByRole('listitem')).toHaveCount(24);
      expect(await links.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
        true,
      );
      await prs.focus();
      await page.keyboard.press('Tab');
      await expect(links).toBeFocused();
      const positions = () =>
        links.evaluate((element) => [
          element.closest('.xt-shell-outlet')!.scrollTop,
          window.scrollY,
        ]);
      const beforeLinks = await positions();
      await page.keyboard.press('End');
      await expect
        .poll(() =>
          links.evaluate(
            (element) => element.scrollHeight - element.clientHeight - element.scrollTop,
          ),
        )
        .toBe(0);
      expect(await links.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
      expect(await positions()).toEqual(beforeLinks);
      await expect(links).toBeFocused();
      const ring = await links.evaluate((element) => {
        const box = element.getBoundingClientRect();
        const shell = element.closest('.xt-shell-outlet')!.getBoundingClientRect();
        return {
          fits:
            box.left >= 0 &&
            box.right <= innerWidth &&
            box.top >= shell.top &&
            box.bottom <= shell.bottom,
          width: parseFloat(getComputedStyle(element).outlineWidth),
        };
      });
      expect(ring.fits).toBe(true);
      expect(ring.width).toBeGreaterThan(0);
      await page.screenshot({ path: info.outputPath(`focused-prs-${theme}.png`) });
      await prs.click();
      await page.screenshot({ path: info.outputPath(`layout-${theme}.png`) });
      await page.locator('.xt-session-stretches > summary').click();
      const stretch = page
        .getByRole('list', { name: 'Hands-off stretches, in order' })
        .getByRole('button')
        .first();
      await stretch.focus();
      await stretch.press('Enter');
      const target = page.locator('.xt-tool-card[data-block-id="exact-target:3"]');
      await expect(target).toHaveAttribute('data-highlighted', 'true');
      expect(
        await target.evaluate(
          (element) =>
            element.clientHeight > element.closest('.xt-conversation-body')!.clientHeight,
        ),
      ).toBe(true);
      await expect(stretch).toBeFocused();
      await expect
        .poll(async () =>
          target.evaluate((element) => {
            const box = element.getBoundingClientRect();
            const owner = element.closest('.xt-conversation-body')!.getBoundingClientRect();
            const shell = element.closest('.xt-shell-outlet')!.getBoundingClientRect();
            return (
              box.top >= Math.max(0, owner.top, shell.top) - 1 &&
              box.top < Math.min(innerHeight, owner.bottom, shell.bottom)
            );
          }),
        )
        .toBe(true);
      const after = await shape();
      expect(after.scroll).toBeGreaterThan(0);
      await page.getByRole('link', { name: '← All sessions' }).click();
      await expect(page.getByLabel('Search sessions')).toHaveValue('synthetic');
      await expect(page.getByRole('radio', { name: '30d' })).toHaveAttribute(
        'aria-checked',
        'true',
      );
    });
  }
}

test('short and unavailable histories keep their notices reachable', async ({ page }) => {
  for (const mode of ['short', 'failed']) {
    await page.goto(`/e2e/session-layout.html?mode=${mode}`);
    const owner = page.getByRole('region', { name: 'Conversation content' });
    await expect(owner).toBeVisible();
    await expect(
      owner.getByText(
        mode === 'short' ? /Synthetic message 0/ : 'This transcript is not available to show.',
      ),
    ).toBeVisible();
  }
});

for (const motion of ['reduce', 'no-preference'] as const) {
  test(`keyboard reveal stays visible above the shell with ${motion} motion`, async ({ page }) => {
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.emulateMedia({ reducedMotion: motion });
    await page.goto('/e2e/session-layout.html');
    const owner = page.getByRole('region', { name: 'Conversation content' });
    await expect(owner.getByText(/Synthetic message 0/)).toBeVisible();
    await page.locator('.xt-session-stretches > summary').click();
    // Extra surrounding fixture content makes the upper clipping boundary scrollable.
    await owner.evaluate((element) => {
      const shell = element.closest('.xt-shell-outlet')!;
      const spacer = document.createElement('div');
      spacer.style.height = '200vh';
      shell.append(spacer);
      shell.scrollTop +=
        element.getBoundingClientRect().top - shell.getBoundingClientRect().top + 20;
    });
    const clipped = await owner.evaluate((element) => ({
      top: element.getBoundingClientRect().top,
      shellTop: element.closest('.xt-shell-outlet')!.getBoundingClientRect().top,
    }));
    expect(clipped.top).toBeGreaterThan(0);
    expect(clipped.top).toBeLessThan(clipped.shellTop);
    const stretch = page.locator('.xt-timeline-segment').first();
    await stretch.evaluate((element: HTMLElement) => element.focus({ preventScroll: true }));
    await page.keyboard.press('Enter');
    const target = page.locator('.xt-tool-card[data-block-id="exact-target:3"]');
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect(stretch).toBeFocused();
    await expect
      .poll(() =>
        target.evaluate((element) => {
          const box = element.getBoundingClientRect();
          const owner = element.closest('.xt-conversation-body')!.getBoundingClientRect();
          const shell = element.closest('.xt-shell-outlet')!.getBoundingClientRect();
          return (
            box.top >= Math.max(0, owner.top, shell.top) - 1 &&
            box.top < Math.min(innerHeight, owner.bottom, shell.bottom)
          );
        }),
      )
      .toBe(true);
  });
}

test('a fully visible owner reveals its exact target without moving the page', async ({ page }) => {
  await page.goto('/e2e/session-layout.html?mode=compact');
  const owner = page.getByRole('region', { name: 'Conversation content' });
  await expect(owner.getByText(/Synthetic message 0/)).toBeVisible();
  await page.locator('.xt-session-stretches > summary').click();
  const stretch = page.locator('.xt-timeline-segment').first();
  await stretch.focus();
  const position = () =>
    owner.evaluate((element) => [element.closest('.xt-shell-outlet')!.scrollTop, window.scrollY]);
  const before = await position();
  expect(
    await owner.evaluate((element) => {
      const box = element.getBoundingClientRect();
      const shell = element.closest('.xt-shell-outlet')!.getBoundingClientRect();
      return box.top >= shell.top && box.bottom <= shell.bottom;
    }),
  ).toBe(true);
  await page.keyboard.press('Enter');
  const target = page.locator('.xt-tool-card[data-block-id="exact-target:3"]');
  await expect(target).toHaveAttribute('data-highlighted', 'true');
  await expect(stretch).toBeFocused();
  await expect
    .poll(() =>
      target.evaluate((element) => {
        const box = element.getBoundingClientRect();
        const owner = element.closest('.xt-conversation-body')!.getBoundingClientRect();
        return box.top >= owner.top - 1 && box.top < owner.bottom;
      }),
    )
    .toBe(true);
  expect(await position()).toEqual(before);
});
