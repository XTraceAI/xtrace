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
      const navigation = page.getByRole('navigation', { name: 'Repeated activity' });
      const next = navigation.getByRole('button', { name: 'Next ↓' });
      await expect(navigation).toContainText('1 notice');
      const activity = owner.getByRole('button', { name: 'Tool activity · 2 saved messages' });
      await expect(activity).toHaveAttribute('aria-expanded', 'false');
      await next.focus();
      await next.press('Enter');
      const repeatedTarget = owner.locator('.xt-tool-card[data-block-id="exact-target:3"]');
      await expect(repeatedTarget).toHaveAttribute('data-highlighted', 'true');
      await expect(activity).toHaveAttribute('aria-expanded', 'true');
      await expect(next).toBeFocused();
      await expect(navigation).toContainText('1 of 1');
      await expect
        .poll(() =>
          repeatedTarget.evaluate((element) => {
            const box = element.getBoundingClientRect();
            const owner = element.closest('.xt-conversation-body')!.getBoundingClientRect();
            return box.top >= owner.top - 1 && box.top < owner.bottom;
          }),
        )
        .toBe(true);
      await activity.click();
      await expect(activity).toHaveAttribute('aria-expanded', 'false');
      await next.focus();
      await next.press('Enter');
      await expect(activity).toHaveAttribute('aria-expanded', 'false');
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
    await page.goto('/e2e/session-layout.html?mode=compact');
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

for (const theme of ['light', 'dark'] as const) {
  test(`unavailable repeat feedback stays on screen after reading to the end, ${theme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/e2e/session-layout.html?mode=unavailable-repeat');
    const owner = page.getByRole('region', { name: 'Conversation content' });
    await expect(owner.getByText('Synthetic message 0.', { exact: false })).toBeVisible();
    const recordOrder = () =>
      owner
        .locator('[data-record-id]')
        .evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-record-id')));
    const order = await recordOrder();
    await owner.focus();
    await owner.press('End');
    await expect
      .poll(() => owner.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
      .toBe(0);
    const beforeHeight = await owner.evaluate((node) => node.clientHeight);
    const before = await owner.evaluate((node) => ({
      scrollTop: node.scrollTop,
      outer: node.closest('.xt-shell-outlet')!.scrollTop,
      page: window.scrollY,
      headerTop: node
        .closest('.xt-section-card')!
        .querySelector('.xt-section-header')!
        .getBoundingClientRect().top,
    }));
    const next = page
      .getByRole('navigation', { name: 'Repeated activity' })
      .getByRole('button', { name: 'Next ↓' });
    await next.focus();
    await next.press('Enter');
    const status = page.locator('.xt-session-conversation [role="status"][aria-live="polite"]');
    await expect(status).toHaveCount(1);
    await expect(status).toHaveText(
      'The repeated call of activity period 1 cannot be shown. That tool call is not in the transcript that was read.',
    );
    await expect(next).toBeFocused();
    expect(await recordOrder()).toEqual(order);
    const painted = await status.evaluate((node) => {
      const box = node.getBoundingClientRect();
      const clip = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
      const ancestors = [];
      for (let parent = node.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent);
        const bounds = parent.getBoundingClientRect();
        const x = /auto|scroll|hidden|clip/.test(style.overflowX);
        const y = /auto|scroll|hidden|clip/.test(style.overflowY);
        if (x) {
          clip.left = Math.max(clip.left, bounds.left);
          clip.right = Math.min(clip.right, bounds.right);
        }
        if (y) {
          clip.top = Math.max(clip.top, bounds.top);
          clip.bottom = Math.min(clip.bottom, bounds.bottom);
        }
        if (x || y)
          ancestors.push({
            className: parent.className,
            overflowX: style.overflowX,
            overflowY: style.overflowY,
            top: bounds.top,
            bottom: bounds.bottom,
          });
      }
      return {
        box: { left: box.left, top: box.top, right: box.right, bottom: box.bottom },
        clip,
        ancestors,
        fullyPainted:
          box.width > 0 &&
          box.height > 0 &&
          box.left >= clip.left &&
          box.right <= clip.right &&
          box.top >= clip.top &&
          box.bottom <= clip.bottom,
      };
    });
    const after = await owner.evaluate((node) => ({
      scrollTop: node.scrollTop,
      outer: node.closest('.xt-shell-outlet')!.scrollTop,
      page: window.scrollY,
      headerTop: node
        .closest('.xt-section-card')!
        .querySelector('.xt-section-header')!
        .getBoundingClientRect().top,
    }));
    await info.attach('feedback-geometry', {
      body: JSON.stringify(
        {
          before,
          after,
          beforeHeight,
          afterHeight: await owner.evaluate((node) => node.clientHeight),
          painted,
        },
        null,
        2,
      ),
      contentType: 'application/json',
    });
    await page.screenshot({ path: info.outputPath(`feedback-${theme}.png`) });
    expect(painted.fullyPainted).toBe(true);
    expect(after).toEqual(before);
  });
}

for (const theme of ['light', 'dark'] as const) {
  test(`bounded pages reveal oversized exact cards inside visible and clipped readers, ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/e2e/session-layout.html?mode=compact&paged');
    const owner = page.getByRole('region', { name: 'Conversation content' });
    await expect(owner).toContainText('Showing messages 1–3 of 3.');
    const position = () =>
      owner.evaluate((node) => [node.closest('.xt-shell-outlet')!.scrollTop, window.scrollY]);
    const before = await position();
    expect(
      await owner.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const shell = node.closest('.xt-shell-outlet')!.getBoundingClientRect();
        return box.top >= shell.top && box.bottom <= shell.bottom;
      }),
    ).toBe(true);
    const next = page
      .getByRole('navigation', { name: 'Repeated activity' })
      .getByRole('button', { name: 'Next ↓' });
    await next.focus();
    await next.press('Enter');
    const target = owner.locator('.xt-tool-card[data-block-id="exact-target:3"]');
    const painted = () =>
      target.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const body = node.closest('.xt-conversation-body')!.getBoundingClientRect();
        const shell = node.closest('.xt-shell-outlet')!.getBoundingClientRect();
        return (
          box.top >= Math.max(0, body.top, shell.top) - 1 &&
          box.top < Math.min(innerHeight, body.bottom, shell.bottom)
        );
      });
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect.poll(painted).toBe(true);
    await expect(next).toBeFocused();
    expect(await position()).toEqual(before);
    expect(
      await target.evaluate(
        (node) => node.clientHeight > node.closest('.xt-conversation-body')!.clientHeight,
      ),
    ).toBe(true);
    const activity = owner.getByRole('button', { name: 'Tool activity · 2 saved messages' });
    await activity.click();
    await next.focus();
    await next.press('Enter');
    await expect(activity).toHaveAttribute('aria-expanded', 'false');
    await page.locator('.xt-session-stretches > summary').click();
    await owner.evaluate((node) => {
      const shell = node.closest('.xt-shell-outlet')!;
      const spacer = document.createElement('div');
      spacer.style.height = '200vh';
      shell.append(spacer);
      shell.scrollTop += node.getBoundingClientRect().top - shell.getBoundingClientRect().top + 20;
    });
    expect(
      await owner.evaluate(
        (node) =>
          node.getBoundingClientRect().top <
          node.closest('.xt-shell-outlet')!.getBoundingClientRect().top,
      ),
    ).toBe(true);
    const stretch = page.locator('.xt-timeline-segment').first();
    await stretch.evaluate((node: HTMLElement) => node.focus({ preventScroll: true }));
    await page.keyboard.press('Enter');
    await expect(target).toHaveAttribute('data-highlighted', 'true');
    await expect.poll(painted).toBe(true);
    await expect(stretch).toBeFocused();
    await expect(
      page.locator('.xt-session-conversation [role="status"][aria-live="polite"]'),
    ).toHaveText('Showing the first tool call of activity period 1: Read.');
  });

  test(`bounded pages keep keyboard scrolling, paging and unavailable feedback in the compact reader, ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1120, height: 720 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/e2e/session-layout.html?mode=unavailable-repeat&paged');
    const owner = page.getByRole('region', { name: 'Conversation content' });
    await expect(owner).toContainText('Showing messages 1–31 of 62.');
    const element = await owner.elementHandle();
    const outer = () =>
      owner.evaluate((node) => [node.closest('.xt-shell-outlet')!.scrollTop, window.scrollY]);
    const before = await outer();
    await owner.focus();
    await owner.press('End');
    await expect
      .poll(() => owner.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
      .toBe(0);
    expect(await owner.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    await expect(owner).toBeFocused();
    expect(await outer()).toEqual(before);
    await owner.press('Home');
    await expect.poll(() => owner.evaluate((node) => node.scrollTop)).toBe(0);
    const later = owner.getByRole('button', { name: 'Later messages' });
    await later.focus();
    await later.press('Enter');
    await expect(owner).toContainText('Showing messages 32–62 of 62.');
    await expect(later).toBeFocused();
    const earlier = owner.getByRole('button', { name: 'Earlier messages' });
    await earlier.focus();
    await earlier.press('Enter');
    await expect(owner).toContainText('Showing messages 1–31 of 62.');
    await expect(earlier).toBeFocused();
    expect(await owner.evaluate((node, original) => node === original, element)).toBe(true);
    await owner.focus();
    await owner.press('End');
    await expect
      .poll(() => owner.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
      .toBe(0);
    const next = page
      .getByRole('navigation', { name: 'Repeated activity' })
      .getByRole('button', { name: 'Next ↓' });
    await next.focus();
    await next.press('Enter');
    const status = page.locator('.xt-session-conversation [role="status"][aria-live="polite"]');
    await expect(status).toHaveCount(1);
    await expect(status).toContainText(
      'The saved tool call could not be found in this conversation.',
    );
    await expect(next).toBeFocused();
    await expect(owner.locator('[data-record-id]')).toHaveCount(0);
    expect(
      await status.evaluate((node) => {
        const box = node.getBoundingClientRect();
        let top = 0,
          bottom = innerHeight;
        for (let parent = node.parentElement; parent; parent = parent.parentElement) {
          if (/auto|scroll|hidden|clip/.test(getComputedStyle(parent).overflowY)) {
            const bounds = parent.getBoundingClientRect();
            top = Math.max(top, bounds.top);
            bottom = Math.min(bottom, bounds.bottom);
          }
        }
        return box.height > 0 && box.top >= top && box.bottom <= bottom;
      }),
    ).toBe(true);
    expect(await outer()).toEqual(before);
  });
}
