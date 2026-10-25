import { expect, test, type Page } from '@playwright/test';

const reads = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __fill: { reads: { search: string; after: string | null; sort: string }[] };
        }
      ).__fill.reads,
  );
const gap = (page: Page) =>
  page
    .locator('.xt-sessions .xt-table-scroll')
    .evaluate(
      (scroll) =>
        scroll.clientHeight -
        scroll.querySelector('.xt-data-table')!.getBoundingClientRect().height,
    );

test('fills collapsed groups, stops at the viewport, and loads more after resizing', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 1440, height: 720 });
  await page.goto('/e2e/sessions-fill.html');
  await expect.poll(() => gap(page)).toBeLessThanOrEqual(1);
  const first = await reads(page);
  expect(first.map((read) => read.after)).toEqual([null, '50', '100', '150']);
  await expect(page.locator('.xt-table-block')).toHaveCount(11);
  await page.screenshot({ path: info.outputPath('sessions-filled.png') });
  // Scrolling to the end of a full table does not ask for more sessions.
  await page.locator('.xt-table-scroll').evaluate((scroll) => {
    scroll.scrollTop = scroll.scrollHeight;
  });
  await page.waitForTimeout(150);
  expect(await reads(page)).toEqual(first);
  await page.setViewportSize({ width: 1440, height: 1200 });
  await expect.poll(() => gap(page)).toBeLessThanOrEqual(1);
  expect((await reads(page)).length).toBeGreaterThan(first.length);
  await page.screenshot({ path: info.outputPath('sessions-taller-filled.png') });
  // There are only 26 main sessions. A very tall window exhausts the source.
  await page.setViewportSize({ width: 1440, height: 1800 });
  await expect(page.getByRole('button', { name: 'Load more sessions' })).toHaveCount(0);
  await expect(page.locator('.xt-table-block')).toHaveCount(26);
  expect((await reads(page)).map((read) => read.after)).toEqual([
    null,
    '50',
    '100',
    '150',
    '200',
    '250',
    '300',
  ]);
});

test('counts expanded details and refills when they collapse in a narrow table', async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 720 });
  await page.goto('/e2e/sessions-fill.html');
  await expect.poll(() => gap(page)).toBeLessThanOrEqual(1);
  const scroll = page.locator('.xt-sessions .xt-table-scroll');
  expect(await scroll.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
  const before = await reads(page);
  await page.getByRole('button', { name: 'Expand main', exact: true }).click();
  const details = await page
    .locator('.xt-expanded-row')
    .evaluate((element) => element.getBoundingClientRect().height);
  expect(details).toBeGreaterThan(0);
  // The larger viewport still fits because the open details occupy the new space.
  await page.setViewportSize({ width: 900, height: 720 + Math.floor(details) });
  await expect.poll(() => gap(page)).toBeLessThanOrEqual(1);
  await page.waitForTimeout(150);
  expect(await reads(page)).toEqual(before);
  await page.getByRole('button', { name: 'Collapse main', exact: true }).click();
  await expect.poll(async () => (await reads(page)).length).toBeGreaterThan(before.length);
  await expect.poll(() => gap(page)).toBeLessThanOrEqual(1);
});
