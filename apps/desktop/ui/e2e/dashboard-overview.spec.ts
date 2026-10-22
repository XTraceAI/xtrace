import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import { syntheticMergedPage, syntheticOverview } from '../src/app/dashboard/overview.synthetic';
import type { FixtureExport } from '../src/data/generated/FixtureExport';

/**
 * The Overview card and the Effort card's "human h" timeline over the
 * test-only synthetic report (not real history) served in place of the F1
 * export. `OVERVIEW_SHOTS=DIR` also saves a screenshot of each case there.
 */
const synthetic = structuredClone(fixture as FixtureExport);
synthetic.dashboards = synthetic.dashboards.map((report) => syntheticOverview(report));
synthetic.pr_analytics = synthetic.pr_analytics.map(syntheticMergedPage);
const shots = process.env.OVERVIEW_SHOTS;

async function open(page: Page, scheme: 'light' | 'dark', width = 1440, height = 900) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(synthetic)};`,
    }),
  );
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/dashboard');
  await expect(page.getByTestId('overview-grid')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}
const range = (page: Page, value: '7d' | '14d' | '30d') =>
  page.getByRole('radio', { name: value, exact: true }).click();
const measure = (page: Page, name: 'human + agent' | 'agent h' | 'human h' | 'cost') =>
  page.getByRole('radio', { name, exact: true }).click();
const effortCard = (page: Page) =>
  page.locator('section.xt-dash-card').filter({
    has: page.getByRole('heading', { level: 2, name: 'Effort', exact: true }),
  });

/** The timeline's geometry: rows, their height, the scroll position and the bars. */
async function timeline(page: Page) {
  return page.getByTestId('human-timeline-body').evaluate((body) => {
    const rows = [...body.querySelectorAll<HTMLElement>('[data-testid="human-day"]')];
    const tracks = rows.map((row) => row.querySelector('.xt-human-track')!.getBoundingClientRect());
    const axis = document.querySelector('.xt-human-axis')!.getBoundingClientRect();
    return {
      rows: rows.length,
      rowHeight: rows[0]!.getBoundingClientRect().height,
      trackHeight: Math.max(...tracks.map((track) => track.height)),
      clientHeight: body.clientHeight,
      scrollHeight: body.scrollHeight,
      scrollTop: body.scrollTop,
      axisAboveBody: axis.bottom <= body.getBoundingClientRect().top + 1,
      lastVisible:
        rows.at(-1)!.getBoundingClientRect().bottom <= body.getBoundingClientRect().bottom + 1,
      lastDate: rows.at(-1)!.dataset.date,
    };
  });
}

for (const scheme of ['light', 'dark'] as const)
  test(`the Overview card and the human h timeline at 7d and 30d, ${scheme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    // The tile row and the Environment card are gone; Overview holds the four.
    await expect(page.locator('.xt-dash-tiles')).toHaveCount(0);
    await expect(page.getByRole('heading', { level: 2, name: 'Environment' })).toHaveCount(0);
    await expect(page.getByTestId('overview-tile')).toHaveCount(4);
    await expect(
      page.getByTestId('overview-tile').evaluateAll((tiles) => tiles.map((t) => t.dataset.label)),
    ).resolves.toEqual(['Leverage', 'Concurrency', 'Merged PRs', 'Hands-off median']);
    await expect(page.getByTestId('overview-prs').locator('li').first()).toBeVisible();
    await expect(page.getByTestId('day-line')).toHaveCount(3);
    const heights: number[] = [];
    for (const preset of ['7d', '30d'] as const) {
      await range(page, preset);
      await measure(page, 'agent h');
      await expect(page.getByTestId('effort-chart')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-agent-${scheme}.png` });
      await measure(page, 'human h');
      await expect(page.getByTestId('human-timeline')).toBeVisible();
      heights.push((await effortCard(page).boundingBox())!.height);
      const geometry = await timeline(page);
      const days = synthetic.dashboards.find((r) => r.window.days === Number(preset.slice(0, -1)))!
        .human_hours.current.by_day;
      expect(geometry.rows).toBe(days.length);
      // The newest day is the last row, in view; the hour axis stays above the rows.
      expect(geometry.lastDate).toBe(days.at(-1)!.date);
      expect(geometry.lastVisible).toBe(true);
      expect(geometry.axisAboveBody).toBe(true);
      // Eight rows fill the plot; the bars keep space between rows.
      expect(geometry.rowHeight).toBeCloseTo(
        Math.max(28, Math.floor(geometry.clientHeight / 8)),
        0,
      );
      expect(geometry.trackHeight).toBeLessThanOrEqual(30);
      expect(geometry.trackHeight).toBeLessThan(geometry.rowHeight);
      if (preset === '30d') {
        // Longer ranges scroll, opened on today.
        expect(geometry.scrollHeight).toBeGreaterThan(geometry.clientHeight);
        expect(geometry.scrollTop + geometry.clientHeight).toBeGreaterThanOrEqual(
          geometry.scrollHeight - 1,
        );
      } else expect(geometry.scrollHeight).toBeLessThanOrEqual(geometry.clientHeight + 1);
      if (shots)
        await page.screenshot({ path: `${shots}/dashboard-${preset}-human-${scheme}.png` });
    }
    // The Effort card keeps one height in every range and view.
    for (const height of heights) expect(height).toBeCloseTo(heights[0]!, 0);
    expect(errors).toEqual([]);
  });

test('the Effort switch and a day of the timeline work from the keyboard', async ({ page }) => {
  await open(page, 'light');
  const agent = page.getByRole('radio', { name: 'agent h', exact: true });
  await agent.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: 'human h', exact: true })).toBeChecked();
  await expect(page.getByTestId('human-timeline')).toBeVisible();
  // One tab stop: today's row. Tab reaches it, arrows move between days,
  // and the focused day's card opens.
  const timeline = page.getByTestId('human-timeline');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  const days = page.getByTestId('human-day');
  const last = days.last();
  await expect(last).toHaveAttribute('tabindex', '0');
  await expect(last).toHaveAttribute('aria-label', /^Sep 7: /);
  await last.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(last).toBeFocused();
  // These visual-only day popups are aria-hidden and their triggers carry
  // no described-by/controls link. Select the active card for the fixed day,
  // so another day's closing animation cannot make the query ambiguous.
  const activeDayCard = (day: string) =>
    page.locator('.xt-effort-tip[data-open]').filter({
      has: page.locator('.xt-effort-tip-head > span', { hasText: new RegExp(`^${day}$`) }),
    });
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toHaveCount(1);
  await expect(activeDayCard('Sep 7')).toBeVisible();
  await expect(activeDayCard('Sep 7').locator('.xt-effort-tip-head')).toContainText('Sep 7');
  await page.keyboard.press('ArrowUp');
  const before = days.nth(-2);
  await expect(before).toBeFocused();
  await expect(before).toHaveAttribute('aria-label', /^Sep 6: /);
  await expect(page.locator('.xt-effort-tip[data-open]')).toHaveCount(1);
  await expect(activeDayCard('Sep 6')).toHaveCount(1);
  await expect(activeDayCard('Sep 6').locator('.xt-effort-tip-head')).toContainText('Sep 6');
  await expect(timeline.locator('[tabindex="0"]')).toHaveCount(1);
  await page.keyboard.press('Home');
  await expect(days.first()).toBeFocused();
});

for (const scheme of ['light', 'dark'] as const)
  test(`the human + agent tab: first option, one track per day with amber bands and an agents line, icon keys, totals and the plain human h view unchanged, ${scheme}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    const options = page.getByRole('radiogroup', { name: 'Effort measure' }).getByRole('radio');
    await expect(options).toHaveText(['human + agent', 'agent h', 'human h', 'cost']);
    // The default stays agent h.
    await expect(page.getByRole('radio', { name: 'agent h', exact: true })).toBeChecked();
    await expect(page.getByTestId('effort-chart')).toBeVisible();
    const report = synthetic.dashboards.find((r) => r.window.days === 7)!;
    const top = report.leverage.agents_scale_top;
    await measure(page, 'human + agent');
    const timelineView = page.getByTestId('human-timeline');
    await expect(timelineView).toHaveAttribute('data-variant', 'paired');
    const days = page.getByTestId('human-day');
    await expect(days).toHaveCount(report.leverage.by_day.length);
    // One track per day; a line on every day with agent time; no filled
    // ridge or strip.
    await expect(timelineView.locator('.xt-human-track')).toHaveCount(
      report.leverage.by_day.length,
    );
    await expect(page.getByTestId('agent-line')).toHaveCount(
      report.leverage.by_day.filter((day) => day.quarter_agents.some((agents) => agents > 0))
        .length,
    );
    await expect(page.locator('.xt-agent-strip, .xt-agent-ridge')).toHaveCount(0);
    const fills = await page
      .getByTestId('agent-line')
      .locator('path')
      .evaluateAll((paths) => paths.map((path) => getComputedStyle(path).fill));
    for (const fill of fills) expect(fill).toBe('none');
    // Headline: named icon keys instead of the words, then the totals and
    // the leverage; no break threshold.
    const headline = page.getByTestId('effort-headline');
    await expect(headline).toHaveText(
      /^\d+ h \d+ m\d+ h \d+ m[\d.]+×agent time per hour of yours$/,
    );
    await expect(headline.getByRole('img', { name: 'Your time' })).toBeVisible();
    await expect(headline.getByRole('img', { name: 'Agent time' })).toBeVisible();
    // The person icon heads the totals column, in your colour.
    const head = page.getByTestId('human-total-head');
    await expect(head.locator('svg')).toBeVisible();
    const columns = await page.evaluate(() => {
      const head = document.querySelector('[data-testid="human-total-head"]')!;
      const total = document.querySelector('[data-testid="human-day"] .xt-human-total')!;
      const swatch = document.querySelector('.xt-paired-swatch[data-key="band"]')!;
      return {
        headRight: head.getBoundingClientRect().right,
        totalRight: total.getBoundingClientRect().right,
        headColor: getComputedStyle(head).color,
        human: getComputedStyle(swatch).getPropertyValue('--human').trim(),
      };
    });
    expect(Math.abs(columns.headRight - columns.totalRight)).toBeLessThanOrEqual(1);
    expect(columns.human).toBe(scheme === 'dark' ? '#e6a23c' : '#b7791f');
    expect(columns.headColor).toBe(scheme === 'dark' ? 'rgb(230, 162, 60)' : 'rgb(183, 121, 31)');
    await expect(page.getByTestId('effort-leverage')).toHaveText(/^[\d.]+×$/);
    await expect(headline).not.toContainText('break threshold');
    // The right column is your time only.
    await expect(page.getByTestId('agent-day-total')).toHaveCount(0);
    const humanTotals = await days.locator('.xt-human-total').allTextContents();
    expect(humanTotals).toHaveLength(report.leverage.by_day.length);
    for (const total of humanTotals) expect(total).toMatch(/^(\d+ h \d+ m|—)$/);
    // The line's highest point is the day's busiest average on the report's
    // one scale, in its box from 25 (no agents) up to 2 (the top); your time
    // is full-height amber bands behind it.
    const last = report.leverage.by_day.at(-1)!;
    const geometry = await days.last().evaluate((row) => {
      const track = row.querySelector('.xt-human-track')!;
      const box = track.getBoundingClientRect();
      const line = row.querySelector<SVGPathElement>('[data-testid="agent-line"] path')!.getBBox();
      const bars = [...row.querySelectorAll('.xt-human-bar')].map((bar) =>
        bar.getBoundingClientRect(),
      );
      return {
        row: row.getBoundingClientRect().height,
        track: box.height,
        lineTop: line.y,
        lineBottom: line.y + line.height,
        lineLast: track.lastElementChild?.getAttribute('data-testid'),
        barHeights: bars.map((bar) => bar.height),
        barTops: bars.map((bar) => bar.top - box.top),
        barColor: bars.length
          ? getComputedStyle(row.querySelector('.xt-human-bar')!).backgroundColor
          : '',
      };
    });
    expect(geometry.row).toBeGreaterThanOrEqual(34);
    expect(geometry.track).toBeGreaterThanOrEqual(26);
    expect(geometry.lineLast).toBe('agent-line');
    expect(geometry.lineBottom).toBeCloseTo(25, 1);
    expect((25 - geometry.lineTop) / 23).toBeCloseTo(
      Math.min(Math.max(...last.quarter_agents), top) / top,
      2,
    );
    expect(geometry.barHeights.length).toBeGreaterThan(0);
    for (const height of geometry.barHeights) expect(height).toBeCloseTo(geometry.track, 0);
    for (const top of geometry.barTops) expect(Math.abs(top)).toBeLessThanOrEqual(1);
    // A see-through colour mix (amber at about a fifth), not a solid fill.
    expect(geometry.barColor).toMatch(/^(color\(srgb|rgba\(|oklab)/);
    // Pointing at a day's track draws the hairline and names that quarter;
    // the card keeps the day's agent total.
    const track = days.last().locator('.xt-human-track');
    const box = (await track.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.5 + 2, box.y + box.height / 2);
    await expect(page.getByTestId('agent-hairline')).toBeVisible();
    await expect(page.getByTestId('agent-quarter')).toBeVisible();
    const card = page.locator('.xt-effort-tip[data-open]');
    await expect(card).toContainText('Agent time');
    await expect(card.getByTestId('agent-day-total')).toHaveText(/^\d+ h \d+ m$/);
    if (shots) await page.screenshot({ path: `${shots}/dashboard-7d-paired-${scheme}.png` });
    await page.mouse.move(0, 0);
    // 30 days scroll, opened on today.
    await range(page, '30d');
    const body = page.getByTestId('human-timeline-body');
    const scroll = await body.evaluate((element) => ({
      top: element.scrollTop,
      client: element.clientHeight,
      height: element.scrollHeight,
    }));
    expect(scroll.height).toBeGreaterThan(scroll.client);
    expect(scroll.top + scroll.client).toBeGreaterThanOrEqual(scroll.height - 1);
    // The plain human h view has no line, bands or icons and keeps its
    // headline and inset blue bars.
    await measure(page, 'human h');
    await expect(timelineView).not.toHaveAttribute('data-variant', 'paired');
    await expect(page.getByTestId('agent-line')).toHaveCount(0);
    await expect(page.getByTestId('human-total-head')).toHaveCount(0);
    await expect(page.getByTestId('effort-headline')).toContainText('break threshold');
    const plainBar = await page
      .getByTestId('human-day')
      .last()
      .locator('.xt-human-bar')
      .first()
      .evaluate((bar) => bar.getBoundingClientRect().height);
    expect(plainBar).toBeGreaterThan(4);
    expect(errors).toEqual([]);
  });
