import { expect, test, type Page } from '@playwright/test';
import { SESSION_WARNINGS_TEXT } from '../src/data/generated/SessionWarningsText';
test.use({ timezoneId: 'UTC' });
const shots = process.env.WARNING_SHOTS;
async function open(page: Page, scheme: 'light' | 'dark') {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/e2e/dashboard-warning.html?path=/dashboard?range=7d');
  await page.evaluate(() => document.fonts.ready);
}
const laneRow = (page: Page, name: string) =>
  page
    .getByRole('table', { name: 'Session lanes' })
    .getByRole('row')
    .filter({ has: page.getByRole('link', { name: new RegExp(`Open session ${name}`) }) });
/** The centre of an element, for pointing at a dot that takes no pointer events itself. */
async function centre(page: Page, selector: ReturnType<Page['locator']>) {
  const box = (await selector.boundingBox())!;
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

const effortDay = (page: Page, date: string) =>
  page.locator(`[data-testid="human-day"][data-date="${date}"]`);

for (const scheme of ['light', 'dark'] as const) {
  test(`pointing at an Effort dot shows that warning, ${scheme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page, scheme);
    await page.getByRole('radio', { name: 'human + agent', exact: true }).click();
    const sep3 = effortDay(page, '2026-09-03');
    await expect(sep3.getByTestId('day-warning-dot')).toHaveCount(1);
    const { x, y } = await centre(page, sep3.getByTestId('day-warning-dot'));
    await page.mouse.move(x, y);
    const tip = page.getByTestId('day-warning-tip');
    // The session (host mark and name), the kind, then the short reason; no day.
    await expect(tip.getByTestId('day-warning-tip-line')).toHaveText([
      'Rename the config moduleCorrectionAfter an agent reply',
    ]);
    // The host mark is found by its element, not its role: the hint is
    // aria-hidden (the dot's own name says everything), so it has no image role.
    await expect(tip.locator('.xt-host-glyph-mark')).toHaveAttribute(
      'data-host',
      /^(claude|codex|cursor)$/,
    );
    await expect(tip).not.toContainText(SESSION_WARNINGS_TEXT.open_hint);
    // The day's own card gives way while a dot's warning shows.
    await expect(page.getByTestId('day-warning')).toHaveCount(0);
    if (shots) await page.screenshot({ path: `${shots}/effort-dot-hover-${scheme}.png` });
    // Dots drawn over one another (14:02 and 14:06): every warning under the pointer.
    const sep7 = effortDay(page, '2026-09-07');
    const failure = sep7.locator('[data-testid="day-warning-dot"][data-kind="repeated_failure"]');
    const at = await centre(page, failure);
    await page.mouse.move(at.x, at.y);
    const lines = tip.getByTestId('day-warning-tip-line');
    await expect(lines).toHaveText([
      'Fix the flaky buildRepeated failurethe same Bash call failed twice',
      'Fix the flaky buildInterruptionDeclined Edit',
    ]);
    if (shots) await page.screenshot({ path: `${shots}/effort-dots-overlap-${scheme}.png` });
    // Off the dots, on the same day: the day's card again.
    const track = (await sep7.locator('.xt-human-track').boundingBox())!;
    await page.mouse.move(track.x + track.width * 0.3, track.y + track.height / 2);
    await expect(tip).toHaveCount(0);
    await expect(page.getByTestId('day-warning')).toHaveCount(0);
    await expect(page.getByTestId('day-card')).toBeVisible();
    expect(errors).toEqual([]);
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`Dashboard dots open the selected issue in place by pointer, Enter and Space, ${scheme}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    await open(page, scheme);
    const location = page.url();
    const lane = laneRow(page, 'Fix the flaky build');
    const laneDot = lane.locator('[data-testid="warning-dot"][data-warning-id="correction:w-h3"]');
    const laneScroll = await lane
      .locator('xpath=ancestor::*[contains(@class,"xt-table-scroll")]')
      .evaluate((node) => node.scrollTop);
    await laneDot.click();
    const modal = page.getByTestId('dashboard-warning-modal');
    await expect(modal).toHaveAccessibleName('Correction Fix the flaky build');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    // A correction's reason is its whole message, the native marks shown.
    const reason = modal.getByTestId('warning-reason');
    await expect(reason).toContainText(
      `${SESSION_WARNINGS_TEXT.you_wrote}Still not fixed. The sample build fails at the same step.`,
    );
    await expect(reason.locator('mark')).toHaveText(['Still', 'not']);
    await expect(page.getByTestId('warning-tip')).toHaveCount(0);
    await expect(page).toHaveURL(location);
    if (shots) await page.screenshot({ path: `${shots}/lane-warning-modal-${scheme}.png` });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await expect(laneDot).toBeFocused();
    expect(
      await lane
        .locator('xpath=ancestor::*[contains(@class,"xt-table-scroll")]')
        .evaluate((node) => node.scrollTop),
    ).toBe(laneScroll);
    await page.keyboard.press('Space');
    await expect(modal).toHaveAccessibleName('Correction Fix the flaky build');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(laneDot).toBeFocused();

    await page.getByRole('radio', { name: 'human + agent', exact: true }).click();
    const sep3 = effortDay(page, '2026-09-03');
    const dot = sep3.getByTestId('day-warning-dot');
    const body = page.getByTestId('human-timeline-body');
    const before = await body.evaluate((node) => node.scrollTop);
    const at = await centre(page, dot);
    await page.mouse.move(at.x, at.y);
    await page.mouse.click(at.x, at.y);
    await expect(modal).toHaveAccessibleName('Correction Rename the config module');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    await expect(modal).toContainText('you wrote "wrong" after a reply (09:30)');
    await expect(page.getByTestId('day-warning-tip')).toHaveCount(0);
    if (shots) await page.screenshot({ path: `${shots}/effort-warning-modal-${scheme}.png` });
    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(dot).toBeFocused();
    expect(await body.evaluate((node) => node.scrollTop)).toBe(before);
    await page.keyboard.press('Escape');
    await expect(sep3).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Enter');
    await expect(modal).toHaveAccessibleName('Correction Rename the config module');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(dot).toBeFocused();
    await page.keyboard.press('Space');
    await expect(modal).toHaveAccessibleName('Correction Rename the config module');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    await modal.getByRole('button', { name: 'Close' }).click();
    await expect(dot).toBeFocused();
    await expect(page).toHaveURL(location);
    // Overlapping dots remain individually reachable by arrows.
    const sep7 = effortDay(page, '2026-09-07');
    await sep7.focus();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Enter');
    await expect(modal).toHaveAccessibleName('Interruption Fix the flaky build');
    await expect(modal.getByRole('button', { name: 'Close' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(sep7.getByTestId('day-warning-dot').nth(1)).toBeFocused();
    expect(errors).toEqual([]);
  });
}

test.describe('report timezone and total labels', () => {
  test.use({ timezoneId: 'Pacific/Honolulu' });
  for (const scheme of ['light', 'dark'] as const) {
    test(`keeps native UTC warning positions and existing total label tooltips in ${scheme}`, async ({
      page,
    }) => {
      await open(page, scheme);
      await page.getByRole('radio', { name: 'human + agent', exact: true }).click();
      const day = effortDay(page, '2026-09-07');
      const dot = day.locator('[data-testid="day-warning-dot"][data-kind="repeated_failure"]');
      await expect(dot).toBeAttached();
      const percentage = await dot.evaluate((node) => parseFloat((node as HTMLElement).style.left));
      expect(percentage).toBeCloseTo(((14 + 4 / 60) / 24) * 100, 3);
      await expect(dot).toHaveAttribute('aria-label', /14:02, 14:04/);
      for (const [id, label] of [
        ['human-total-head', 'Human hrs'],
        ['agent-total-head', 'Agent hrs'],
        ['leverage-total-head', 'Leverage'],
      ]) {
        const header = page.getByTestId(id!);
        await expect(header).toHaveAttribute('aria-label', label!);
        await header.hover();
        await expect(page.getByRole('tooltip')).toHaveText(label!);
        await page.keyboard.press('Escape');
        await expect(page.getByRole('tooltip')).toHaveCount(0);
        await page.mouse.move(0, 0);
      }
    });
  }
});
