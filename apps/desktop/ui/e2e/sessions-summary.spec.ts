import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
const SUMMARY_SCOPE =
  'Counts cover all matching indexed sessions. Messages and agent working time cover the selected range. The table hides sessions still being checked and sub-sessions without a verified parent.';
const SUMMARY_SCOPE_SHORT = 'Matching indexed sessions · messages/time in selected range';

/** Four totals over every indexed match, including rows not loaded yet. */
const LABELS = ['Sessions', 'Your input', 'Agent working time', 'Sessions with PRs'];

for (const [width, height] of [
  [1440, 900],
  [1250, 768],
  [1120, 720],
] as const)
  for (const scheme of ['light'] as const)
    test(`Sessions summary tiles stay readable at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/sessions');
      const summary = page.getByRole('region', { name: 'Range summary' });
      await expect(summary.locator('.xt-stat-tile')).toHaveCount(4);
      // What the tiles cover reads in view as the heading's one short line;
      // the whole sentence is the summary's description.
      const line = page.locator('.xt-sessions-heading p');
      await expect(line).toHaveText(SUMMARY_SCOPE_SHORT);
      await expect(summary).toHaveAccessibleDescription(SUMMARY_SCOPE);
      await page.evaluate(() => document.fonts.ready);
      const subtitle = await line.evaluate((element) => ({
        height: element.getBoundingClientRect().height,
        lineHeight: parseFloat(getComputedStyle(element).lineHeight),
        clipped: element.scrollWidth > element.clientWidth,
      }));
      const overflow = await page.evaluate(() => {
        const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
        return {
          outlet: outlet.scrollHeight - outlet.clientHeight,
          page: document.documentElement.scrollHeight - window.innerHeight,
        };
      });
      await info.attach('subtitle', { body: JSON.stringify({ subtitle, overflow }, null, 2) });
      // One whole line, neither wrapped nor cut short, and the page itself does not scroll.
      expect(subtitle.height).toBeLessThanOrEqual(subtitle.lineHeight + 0.5);
      expect(subtitle.clipped).toBe(false);
      expect(overflow.outlet).toBeLessThanOrEqual(0);
      expect(overflow.page).toBeLessThanOrEqual(0);
      // The filters and the warning slot stay usable beside or under it.
      await expect(page.getByRole('searchbox', { name: 'Search sessions' })).toBeVisible();
      await expect(page.getByRole('switch', { name: 'With PRs only' })).toBeVisible();
      const tiles = await page.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>('.xt-sessions-tiles .xt-stat-tile')].map(
          (tile) => {
            const fits = (selector: string) => {
              const element = tile.querySelector<HTMLElement>(selector);
              return element ? element.scrollWidth <= element.clientWidth : true;
            };
            const box = tile.getBoundingClientRect();
            return {
              label: tile.querySelector<HTMLElement>('.xt-stat-label')!.textContent,
              text: tile.textContent,
              labelFits: fits('.xt-stat-label'),
              asideFits: fits('.xt-stat-aside'),
              valueFits: fits('.xt-stat-value .xt-metric-cell'),
              inside: [...tile.querySelectorAll<HTMLElement>('.xt-stat-label, .xt-stat-row')]
                .map((element) => element.getBoundingClientRect())
                .every((rect) => rect.left >= box.left && rect.right <= box.right + 0.5),
            };
          },
        ),
      );
      await info.attach('tiles', { body: JSON.stringify(tiles, null, 2) });
      await page
        .locator('.xt-sessions-tiles')
        .screenshot({ path: info.outputPath(`sessions-tiles-${width}-${scheme}.png`) });
      expect(tiles.map((tile) => tile.label)).toEqual(LABELS);
      // These are the Rust-exported F1 answers, not totals calculated from visible rows.
      const values = summary.locator('.xt-stat-value .xt-metric-cell');
      await expect(values).toHaveText(['1', '5', '0 h 23 m', '1']);
      await expect(summary.locator('.xt-stat-aside')).toHaveText([
        '0 sub · 0 checking',
        '5 / main',
        'Parallel sessions add together',
      ]);
      for (const tile of tiles) {
        expect(tile.labelFits, tile.label ?? '').toBe(true);
        expect(tile.asideFits, tile.label ?? '').toBe(true);
        expect(tile.valueFits, tile.label ?? '').toBe(true);
        expect(tile.inside, tile.label ?? '').toBe(true);
      }
      // The summary sits above the list; it never stands in for it.
      await expect(page.getByRole('table', { name: 'Indexed sessions' })).toBeVisible();
      await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    });

/** Synthetic measurements use the same summary and row field, never dashboard hours. */
test('a short active time and incomplete input keep their meaning', async ({ page }, info) => {
  const small = structuredClone(fixture as FixtureExport);
  for (const entry of small.sessions_summaries ?? []) {
    if (!entry.session_ids.length) continue;
    entry.summary.agent_ms = 2000;
    entry.summary.human_messages = null;
    entry.summary.messages_per_main_session = null;
  }
  for (const sessionPage of small.sessions) {
    if (sessionPage.summary)
      Object.assign(sessionPage.summary, {
        agent_ms: 2000,
        human_messages: null,
        messages_per_main_session: null,
      });
    for (const row of sessionPage.rows)
      if (row.metrics.state === 'indexed') {
        row.metrics.agent_ms = 2000;
        row.metrics.human_messages = null;
      }
  }
  await page.route('**/fixtures/F1.json*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(small)};`,
    }),
  );
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto('/sessions');
  const summary = page.getByRole('region', { name: 'Range summary' });
  await expect(summary.locator('.xt-stat-value .xt-metric-cell').nth(2)).toHaveText('<1 m');
  const input = summary.locator('.xt-stat-tile').filter({ hasText: 'Your input' });
  await expect(input.locator('.xt-stat-aside')).toHaveCount(0);
  await input.focus();
  await expect(page.getByRole('tooltip')).toContainText('Some messages have not been classified');
  await page.keyboard.press('Escape');
  const cell = page
    .getByRole('row')
    .filter({ hasText: 'Session 00000000' })
    .getByText('<1 m', { exact: true });
  await expect(cell).toBeVisible();
  expect(await cell.evaluate((element) => element.scrollWidth <= element.clientWidth + 0.5)).toBe(
    true,
  );
  await page
    .locator('.xt-sessions-tiles')
    .screenshot({ path: info.outputPath('sessions-tiles-small.png') });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1120);
});
