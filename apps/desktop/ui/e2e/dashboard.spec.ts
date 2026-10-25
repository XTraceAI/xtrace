import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

const reports = Object.fromEntries(
  fixture.dashboards.map((report) => [report.window.days, report]),
);
const labels = ['Leverage', 'Concurrency', 'Merged PRs', 'Hands-off median'];

async function open(page: Page, width: number, height: number, scheme: 'light' | 'dark') {
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('What your agents did');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

test('range presets update the Dashboard while account usage stays separate', async ({ page }) => {
  await open(page, 1440, 900, 'light');
  const range = page.getByRole('radiogroup', { name: 'Date range' });
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(page.getByRole('button', { name: 'Custom range' })).toBeDisabled();
  const usage = page.getByRole('region', { name: 'Account usage' });
  await expect(usage).toBeVisible();
  const bars = page.getByRole('group', { name: 'Output tokens per day' }).getByRole('img');
  const period = page.getByTestId('report-period');
  for (const days of [30, 14, 7] as const) {
    await range.getByRole('radio', { name: `${days}d` }).click();
    await expect(range.getByRole('radio', { name: `${days}d` })).toBeChecked();
    await expect(usage).toContainText('Usage source unavailable');
    // The daily chart is behind Effort's ⓘ, one dialog away, and follows the range.
    await page.getByRole('button', { name: 'Effort definition', exact: true }).click();
    await expect(bars).toHaveCount(reports[days].days.length);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    const [first, last] = [reports[days].days[0].date, reports[days].days.at(-1)!.date];
    const short = (date: string) =>
      new Date(`${date}T00:00:00Z`).toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        timeZone: 'UTC',
      });
    await expect(period).toContainText(short(first));
    await expect(period).toContainText(short(last).split(' ')[1]);
    await expect(page.getByTestId('report-period')).toBeVisible();
  }
  // Lanes keep their fixed recent axis whatever range is selected; its header
  // names it. No caption under the rows and no line under the card's title.
  await expect(page.locator('.xt-lanes-activity-label')).toHaveText('Activity');
  await expect(page.getByTestId('lanes-disclosure')).toHaveCount(0);
  await expect(page.getByText('One row per session')).toHaveCount(0);
  // Sessions measures the same window, so it keeps the control and the period.
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(usage).toContainText('Usage source unavailable');
  await expect(period).toHaveCount(1);
  // The pull requests report measures it too; its cached inventory view,
  // like any route that measures no window, keeps the selection without it.
  await page.getByRole('button', { name: 'Pull requests', exact: true }).click();
  await expect(range.getByRole('radio', { name: '7d' })).toBeChecked();
  await expect(period).toHaveCount(1);
  await page.getByRole('radio', { name: 'Cached inventory' }).click();
  await expect(page.getByRole('radiogroup', { name: 'Date range' })).toHaveCount(0);
  await expect(usage).toContainText('Usage source unavailable');
  await expect(period).toHaveCount(0);
});

/** Each control Tab reaches inside the Dashboard, in order, by its accessible name or text. */
async function dashboardTabOrder(page: Page) {
  await page
    .getByRole('radiogroup', { name: 'Date range' })
    .getByRole('radio', { checked: true })
    .focus();
  const inside = () =>
    page.evaluate(() => {
      const active = document.activeElement;
      if (!active?.closest('.xt-dashboard')) return null;
      return (active.getAttribute('aria-label') ?? active.textContent ?? '').trim();
    });
  const order: string[] = [];
  for (let presses = 0; presses < 200; presses += 1) {
    await page.keyboard.press('Tab');
    const name = await inside();
    if (name === null) {
      if (order.length > 0) break;
      continue;
    }
    order.push(name);
  }
  return order;
}

for (const scheme of ['light'] as const)
  test(`Tab skips the hidden panels and reaches the remaining controls at 1120x720 ${scheme}`, async ({
    page,
  }) => {
    await open(page, 1120, 720, scheme);
    const range = page.getByRole('radiogroup', { name: 'Date range' });
    for (const days of [7, 30] as const) {
      await range.getByRole('radio', { name: `${days}d` }).click();
      await expect(page.getByTestId('report-period')).toBeVisible();
      const order = await dashboardTabOrder(page);
      // The favorite model leads; each Overview tile's definition is a tab
      // stop, in reading order; nothing of the hidden pair is one.
      expect(order[0]).toBe('fixture-model-v1');
      expect(
        order
          .filter((name) => labels.some((label) => name === `${label} definition`))
          .map((name) => name.replace(/ definition$/, '')),
      ).toEqual(labels);
      for (const name of ['Effort definition', 'Overview definition', 'Sessions definition'])
        expect(order).toContain(name);
      expect(order.some((name) => name.startsWith('Coverage'))).toBe(true);
      expect(order.join(' | ')).not.toMatch(
        /Agent \/ human hours|Caught by your rules|Agent hours|Human-in-the-loop|Agent to human ratio|Daily values/,
      );
    }
  });

test('hovering the favorite model lists its share of output tokens', async ({ page }) => {
  await open(page, 1120, 720, 'light');
  await page.getByTestId('dashboard-summary').getByRole('button').hover();
  const tip = page.getByRole('tooltip');
  const table = tip.locator('table');
  // Only the table and the previous period are drawn: the definition and the
  // table's title are words for the description alone.
  await expect(tip.getByText('The model that wrote the most output')).toHaveClass('sr-only');
  await expect(table.locator('caption')).toHaveCount(0);
  const top = (await tip.boundingBox())!.y;
  expect((await table.boundingBox())!.y - top).toBeLessThanOrEqual(12);
  await expect(table.locator('th')).toHaveText(['Model', 'Share']);
  await expect(table.locator('tbody td')).toHaveText(['fixture-model-v1', '100%']);
  await expect(tip).toContainText('Previous period: No measured output tokens in this range.');
  // The table is drawn for the eye; the description reads each row once, as words.
  await expect(
    page.getByTestId('dashboard-summary').getByRole('button'),
  ).toHaveAccessibleDescription(
    'The model that wrote the most output in the selected range. ' +
      'Share of output tokens: fixture-model-v1 100%. ' +
      'Previous period: No measured output tokens in this range.',
  );
});

test('keyboard focus opens tile definitions and reaches the Sessions link', async ({ page }) => {
  await open(page, 1120, 720, 'dark');
  const tile = page.getByRole('button', { name: 'Hands-off median definition' });
  await tile.focus();
  const tip = page.getByRole('tooltip');
  await expect(tip).toContainText('The median time an agent worked on its own');
  await expect(tip).not.toContainText('M-09');
  await page.keyboard.press('Escape');
  const link = page.getByRole('link', { name: 'View sessions' });
  await link.focus();
  await expect(link).toBeFocused();
  await link.press('Enter');
  // Sessions measures over the selected window, so the link carries it.
  await expect(page).toHaveURL(/\/sessions\?range=7d$/);
});
