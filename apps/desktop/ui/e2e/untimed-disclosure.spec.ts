import { expect, test, type Locator, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * Synthetic untimed history (not real history): F1 indexes nothing without a
 * timestamp, so the disclosure has nothing to say over the development export.
 * Only `untimed_history` is replaced — every measured number stays F1's, which
 * is the point: this notice explains what the dated numbers cannot hold and
 * changes none of them.
 */
const disclosed = structuredClone(fixture as FixtureExport);
for (const report of disclosed.dashboards)
  report.untimed_history = {
    records: 2318,
    by_surface: [
      { host: 'claude', surface: 'cli', records: 1204 },
      { host: 'claude', surface: 'desktop', records: 806 },
      { host: 'cursor', surface: null, records: 308 },
    ],
  };

const serve = (page: Page, body: FixtureExport) =>
  page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(body)};`,
    }),
  );

const NOTICE =
  'Some indexed history has no timestamps and cannot contribute to date-based measurements.';

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light'] as const)
    for (const path of ['/settings', '/sessions'] as const)
      test(`untimed disclosure reads on ${path} at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, disclosed);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto(path);
        await page.evaluate(() => document.fonts.ready);
        const settings = path === '/settings';
        // On Settings the limit is a section of Data coverage, shown in full;
        // on Sessions it is the heading's warning triangle, whose name carries
        // the count and which opens the explanation over the page. Neither is
        // a banner, and neither states a share of anything.
        let body: Locator;
        if (settings) {
          const coverage = page.getByRole('region', { name: 'Data coverage', exact: true });
          body = coverage.getByTestId('coverage-untimed');
          await expect(body).toBeVisible();
          await expect(body.getByRole('heading', { name: 'Untimed history' })).toBeVisible();
          await expect(body.getByTestId('untimed-summary')).toHaveText(
            '2,318 records · outside dated measurements',
          );
          await expect(body).not.toContainText('%');
          // Measured as part of the card it is drawn in.
          body = coverage.locator('.xt-section-panel');
        } else {
          const notice = page.locator('.xt-sessions-heading');
          await expect(notice).toBeVisible();
          const summary = notice.getByTestId('sessions-issues');
          await expect(summary).toHaveAccessibleName('History limits: 2,318 untimed records');
          await expect(page.getByRole('region', { name: 'Untimed indexed history' })).toHaveCount(
            0,
          );
          await expect(notice).not.toContainText('%');
          const line = await summary.evaluate((element) => element.getBoundingClientRect().height);
          expect(line).toBeLessThanOrEqual(40);
          // The explanation and breakdown are closed by default and open from
          // the keyboard alone.
          body = page.getByRole('dialog', { name: 'Untimed history' });
          await expect(body.getByText(NOTICE)).toBeHidden();
          await summary.focus();
          await expect(summary).toBeFocused();
          await page.keyboard.press('Enter');
        }
        const rows = body.getByRole('list', {
          name: 'Untimed indexed history by host and surface',
        });
        await expect(body.getByText(NOTICE)).toBeVisible();
        const count = body.getByTestId('untimed-count');
        await expect(count).toHaveText(
          '2,318 records in all indexed history, independent of the selected dates and of any filter on the table.',
        );
        await expect(rows).toBeVisible();
        await expect(rows.getByRole('listitem')).toHaveText([
          'Claude Code · cli1,204 records',
          'Claude Code · desktop806 records',
          'Cursor · unknown surface308 records',
        ]);

        // Readable where it is drawn: inside its own box, in the viewport, on
        // this theme's own foreground and surface colors, and without pushing
        // the page sideways at either native width.
        const measured = await body.evaluate((element) => {
          const box = element.getBoundingClientRect();
          const text = [...element.querySelectorAll<HTMLElement>('p, summary, li')];
          return {
            inside: text
              .map((node) => node.getBoundingClientRect())
              .every((rect) => rect.left >= box.left - 0.5 && rect.right <= box.right + 0.5),
            clipped: text.some((node) => node.scrollHeight > node.clientHeight + 1),
            within: box.left >= -0.5 && box.right <= window.innerWidth + 0.5,
            colors: text.map((node) => getComputedStyle(node).color),
            background: getComputedStyle(element).backgroundColor,
          };
        });
        await info.attach('untimed', { body: JSON.stringify(measured, null, 2) });
        await body.screenshot({
          path: info.outputPath(`untimed-${path.slice(1)}-${width}-${scheme}.png`),
        });
        expect(measured.inside).toBe(true);
        expect(measured.clipped).toBe(false);
        expect(measured.within).toBe(true);
        // Themed from tokens: no transparent text, and a surface of its own.
        for (const color of measured.colors) expect(color).not.toMatch(/rgba\(0, 0, 0, 0\)/);
        expect(measured.background).not.toMatch(/rgba\(0, 0, 0, 0\)/);
        expect(new Set(measured.colors).size).toBeGreaterThan(1);
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        if (!settings) {
          await page.keyboard.press('Escape');
          await expect(body).toHaveCount(0);
          await expect(page.getByTestId('sessions-issues')).toBeFocused();
        }
      });
