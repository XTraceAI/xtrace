import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { NativeHostStatus } from '../src/data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../src/data/generated/NativeIndexStatus';

/**
 * The welcome page over the generated F1 export, with test-only index statuses
 * served in its place (not real history). F1 itself reports indexed history
 * at startup, so it opens Dashboard; `serve` clears that fact unless asked.
 */
const host = (name: string, patch: Partial<NativeHostStatus> = {}): NativeHostStatus => ({
  host: name,
  state: 'pending',
  needs_attention: false,
  detail: null,
  sessions_imported: 0,
  sessions_partial: 0,
  sessions_skipped: 0,
  skipped_conversations: [],
  skipped_conversations_omitted: 0,
  records_new: 0,
  records_enriched: 0,
  diagnostics: 0,
  ...patch,
});
const base = {
  freshness: { freshness: 'live' },
  python: { state: 'available', path: '/usr/bin/python3' },
  readers: { state: 'verified', commit: 'b'.repeat(40), plugin_version: '0.1.0' },
  reconciles: 1,
  files_scanned: 212,
  codex_history_gaps: null,
} as const;
const states: Record<string, NativeIndexStatus> = {
  scanning: {
    ...base,
    phase: { phase: 'scanning' },
    freshness: { freshness: 'unknown' },
    python: { state: 'resolving' },
    hosts: ['claude', 'codex', 'cursor'].map((name) => host(name)),
    needs_attention: false,
    reconciles: 0,
    files_scanned: 128,
  },
  partial: {
    ...base,
    phase: { phase: 'ready' },
    freshness: {
      freshness: 'degraded',
      reason: 'the watcher could not be registered for one of the history folders',
    },
    python: { state: 'missing', reason: 'python3 was not found on the login shell path' },
    hosts: [
      host('claude', {
        state: 'incomplete',
        sessions_imported: 12_345,
        sessions_partial: 12,
        sessions_skipped: 3,
        skipped_conversations: [],
        skipped_conversations_omitted: 0,
        records_new: 1_234_567,
        diagnostics: 4,
        detail: 'some transcript lines could not be parsed and were skipped',
        // Sessions were read, so the app does not flag the gaps.
        needs_attention: false,
      }),
      host('codex', {
        state: 'missing_runtime',
        detail: 'python3 was not found',
        needs_attention: true,
      }),
      host('cursor', {
        state: 'reader_failed',
        needs_attention: true,
        detail:
          'the bundled reader exited with status 1 before it reported any session from this history',
      }),
    ],
    needs_attention: true,
  },
  missing: {
    ...base,
    phase: { phase: 'ready' },
    hosts: [
      host('claude', { state: 'complete', sessions_imported: 41, records_new: 900 }),
      host('codex', { state: 'missing_source' }),
      host('cursor', { state: 'missing_source' }),
    ],
    needs_attention: false,
  },
  disabled: fixture.native_index as NativeIndexStatus,
};
/**
 * Database counts that agree with each state: its sessions are the sessions
 * its hosts report read, whole or partly (both are stored), so a screenshot
 * never shows one session beside a host that read thousands.
 */
const stateCounts = (status: NativeIndexStatus): FixtureExport['db_counts'] => {
  const sessions = status.hosts.reduce(
    (sum, host) => sum + host.sessions_imported + host.sessions_partial,
    0,
  );
  const records = status.hosts.reduce((sum, host) => sum + host.records_new, 0);
  return { sessions, records, usage: Math.floor(records / 10) };
};

async function serve(
  page: Page,
  native: NativeIndexStatus,
  history = false,
  counts?: FixtureExport['db_counts'],
) {
  const exported = structuredClone(fixture as FixtureExport);
  exported.app_info.had_indexed_history_at_startup = history;
  exported.native_index = native;
  if (counts) exported.db_counts = counts;
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(exported)};`,
    }),
  );
}
const heading = (page: Page) => page.getByRole('heading', { level: 1 });
const details = (page: Page) => page.getByRole('button', { name: 'Details' });
async function openDetails(page: Page) {
  await details(page).click();
  await expect(details(page)).toHaveAttribute('aria-expanded', 'true');
  return page.getByRole('region', { name: 'Details' });
}
/** The layout checks every state shares: nothing wider than the page or its column. */
const measureLayout = (page: Page) =>
  page.evaluate(() => {
    const root = document.querySelector('.xt-welcome')!;
    const box = root.getBoundingClientRect();
    const escaped = [...root.querySelectorAll<HTMLElement>('*')]
      .filter((element) => {
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && (rect.left < box.left - 0.5 || rect.right > box.right + 0.5);
      })
      .map((element) => element.className);
    const cards = [...root.querySelectorAll('.xt-section-card, .xt-welcome-step')].map((card) => {
      const rect = card.getBoundingClientRect();
      const inner = [...card.querySelectorAll<HTMLElement>('*')].filter((element) => {
        const r = element.getBoundingClientRect();
        return r.width > 0 && (r.left < rect.left - 0.5 || r.right > rect.right + 0.5);
      });
      return inner.length;
    });
    const clipped = [
      ...root.querySelectorAll<HTMLElement>(
        '.xt-welcome-host, .xt-welcome-count, .xt-welcome-tour-item, .xt-welcome-host-detail-item',
      ),
    ].filter((element) => element.scrollWidth > element.clientWidth + 1).length;
    return {
      page: document.documentElement.scrollWidth,
      outlet: (() => {
        const outlet = document.querySelector('.xt-shell-outlet')!;
        return outlet.scrollWidth - outlet.clientWidth;
      })(),
      escaped,
      cards,
      clipped,
    };
  });

test('a fresh startup opens welcome with the real counts; a deep link is kept', async ({
  page,
}) => {
  await serve(page, states.scanning);
  await page.goto('/');
  await expect(page).toHaveURL(/\/first-launch$/);
  await expect(heading(page)).toHaveText('Welcome to XTrace');
  const total = page.locator('.xt-welcome-total');
  await expect(total).toContainText(String(fixture.db_counts.sessions));
  await expect(total).toContainText('In the local database now');
  const region = await openDetails(page);
  const counts = region.getByRole('group', { name: 'Database counts' });
  await expect(counts).toContainText(String(fixture.db_counts.sessions));
  await expect(counts).toContainText(String(fixture.db_counts.records));
  await expect(region.getByTestId('welcome-files')).toHaveText('128');
  await page.goto('/sessions');
  await expect(heading(page)).toHaveText('Sessions');
  await expect(page).toHaveURL(/\/sessions$/);
});

test('existing indexed history opens Dashboard', async ({ page }) => {
  await serve(page, states.missing, true);
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(heading(page)).toHaveText('What your agents did');
});

test('continue by keyboard is remembered across a restart', async ({ page }) => {
  await serve(page, states.missing);
  await page.goto('/');
  await expect(heading(page)).toHaveText('Welcome to XTrace');
  const cta = page.getByRole('button', { name: 'Open Dashboard' });
  for (let step = 0; step < 30; step++) {
    if (await cta.evaluate((element) => element === document.activeElement)) break;
    await page.keyboard.press('Tab');
  }
  await expect(cta).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/dashboard$/);
  await page.goto('/');
  await expect(page).toHaveURL(/\/dashboard$/);
});

test('a ready welcome shows the release check texts with Details closed', async ({ page }) => {
  await serve(page, states.missing);
  await page.goto('/first-launch');
  await expect(page.getByText('The initial scan finished.', { exact: true })).toBeVisible();
  await expect(page.getByText('Bundled readers verified', { exact: true })).toBeVisible();
  await expect(page.getByText('In the local database now', { exact: true })).toBeVisible();
  await expect(page.getByText('Reading database counts')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Open Dashboard', exact: true })).toHaveCount(1);
  await expect(details(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByTestId('welcome-freshness')).toHaveCount(0);
});

test('Details opens and closes by keyboard', async ({ page }) => {
  await serve(page, states.partial);
  await page.goto('/first-launch');
  await expect(heading(page)).toHaveText('Welcome to XTrace');
  const toggle = details(page);
  for (let step = 0; step < 40; step++) {
    if (await toggle.evaluate((element) => element === document.activeElement)) break;
    await page.keyboard.press('Tab');
  }
  await expect(toggle).toBeFocused();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const region = page.getByRole('region', { name: 'Details' });
  await expect(region.getByTestId('welcome-freshness')).toHaveText(
    'Degraded: the watcher could not be registered for one of the history folders',
  );
  await expect(region).toContainText('the bundled reader exited with status 1');
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByTestId('welcome-freshness')).toHaveCount(0);
});

test('a storage write failure still leaves for Dashboard, and welcome returns next time', async ({
  page,
}) => {
  await page.addInitScript(() => {
    Storage.prototype.setItem = () => {
      throw new DOMException('quota', 'QuotaExceededError');
    };
  });
  await serve(page, states.missing);
  await page.goto('/');
  await page.getByRole('button', { name: 'Open Dashboard' }).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  await page.goto('/');
  await expect(page).toHaveURL(/\/first-launch$/);
});

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const state of Object.keys(states))
      test(`welcome fits at ${width}x${height} ${scheme} while ${state}`, async ({
        page,
      }, info) => {
        const errors: string[] = [];
        page.on('pageerror', (error) => errors.push(error.message));
        await serve(page, states[state], false, stateCounts(states[state]));
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/first-launch');
        await expect(heading(page)).toHaveText('Welcome to XTrace');
        const cta = page.getByRole('button', { name: 'Open Dashboard' });
        await expect(cta).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
        // Closed (as first shown), then with Details open: both must fit.
        for (const open of [false, true]) {
          // Opening scrolls Details into view above the pinned bar.
          if (open) await openDetails(page);
          // The continue is always wholly in view, whatever is open or scrolled.
          await expect(cta).toBeInViewport({ ratio: 1 });
          if (open) await expectDetailsReachable(page);
          const layout = await measureLayout(page);
          expect(layout.page).toBe(width);
          expect(layout.outlet).toBe(0);
          expect(layout.escaped).toEqual([]);
          expect(layout.cards.every((count) => count === 0)).toBe(true);
          expect(layout.clipped).toBe(0);
          await page.screenshot({
            path: info.outputPath(
              `welcome-${state}-${width}-${scheme}${open ? '-details' : ''}.png`,
            ),
          });
        }
        expect(errors).toEqual([]);
      });

/**
 * Scrolled to the end of the page, the whole Details panel sits above the
 * pinned bar: nothing in it is hidden behind the bar. Scrolled back to the
 * top for the next check.
 */
async function expectDetailsReachable(page: Page) {
  const cta = page.getByRole('button', { name: 'Open Dashboard' });
  const gap = await page.evaluate(() => {
    const outlet = document.querySelector('.xt-shell-outlet')!;
    outlet.scrollTop = outlet.scrollHeight;
    const panel = document.querySelector('.xt-welcome-details')!.getBoundingClientRect();
    const bar = document.querySelector('.xt-welcome-actions')!.getBoundingClientRect();
    return bar.top - panel.bottom;
  });
  expect(gap).toBeGreaterThanOrEqual(-0.5);
  await expect(cta).toBeInViewport({ ratio: 1 });
}

/** The welcome page with the telemetry notice due, as on a real first launch. */
for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    test(`welcome with the telemetry notice keeps Open Dashboard in view at ${width}x${height} ${scheme}`, async ({
      page,
    }, info) => {
      await serve(page, states.missing, false, stateCounts(states.missing));
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/e2e/telemetry-notice.html?notice=1&route=/first-launch');
      await expect(heading(page)).toHaveText('Welcome to XTrace');
      const notice = page.getByTestId('telemetry-disclosure');
      await expect(notice).toBeVisible();
      // Part of step 3, not floating over the page.
      await expect(page.locator('.xt-welcome-step').last()).toContainText('Anonymous usage data');
      await page.evaluate(() => document.fonts.ready);
      const cta = page.getByRole('button', { name: 'Open Dashboard' });
      await expect(cta).toBeInViewport({ ratio: 1 });
      const layout = await measureLayout(page);
      expect(layout.page).toBe(width);
      expect(layout.outlet).toBe(0);
      expect(layout.escaped).toEqual([]);
      await page.screenshot({ path: info.outputPath(`welcome-telemetry-${width}-${scheme}.png`) });
      // Scrolled to the end, the notice is all above the pinned bar.
      const gap = await page.evaluate(() => {
        const outlet = document.querySelector('.xt-shell-outlet')!;
        outlet.scrollTop = outlet.scrollHeight;
        const card = document
          .querySelector('[data-testid="telemetry-disclosure"]')!
          .getBoundingClientRect();
        const bar = document.querySelector('.xt-welcome-actions')!.getBoundingClientRect();
        return bar.top - card.bottom;
      });
      expect(gap).toBeGreaterThanOrEqual(-0.5);
      await expect(cta).toBeInViewport({ ratio: 1 });
      await page.screenshot({
        path: info.outputPath(`welcome-telemetry-${width}-${scheme}-end.png`),
      });
    });

/**
 * Share of the telemetry disclosure a reader can see: the part inside the
 * window and above the pinned actions bar.
 */
const readableShare = (page: Page) =>
  page.evaluate(() => {
    const card = document
      .querySelector('[data-testid="telemetry-disclosure"]')!
      .getBoundingClientRect();
    const bar = document.querySelector('.xt-welcome-actions')!.getBoundingClientRect();
    const window_ = Math.max(0, Math.min(card.bottom, innerHeight) - Math.max(card.top, 0));
    const above = Math.max(0, Math.min(card.bottom, bar.top) - Math.max(card.top, 0));
    return { inWindow: window_ / card.height, aboveBar: above / card.height };
  });
const shownCount = (page: Page) =>
  page.evaluate(() => (window as { telemetryShown?: number }).telemetryShown ?? 0);
/** Two frames and a pause: time for any visibility callback to have run. */
const settleFrames = (page: Page) =>
  page.evaluate(
    () =>
      new Promise((done) =>
        requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 300))),
      ),
  );

for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  test(`the notice counts as shown only once half of it is above the pinned bar at ${width}x${height}`, async ({
    page,
  }) => {
    await serve(page, states.missing, false, stateCounts(states.missing));
    await page.setViewportSize({ width, height });
    await page.goto('/e2e/telemetry-notice.html?notice=1&route=/first-launch');
    await expect(page.getByTestId('telemetry-disclosure')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await settleFrames(page);
    // On load the bar covers the notice's top half or more: not yet shown.
    const first = await readableShare(page);
    expect(first.aboveBar).toBeLessThan(0.5);
    expect(await shownCount(page)).toBe(0);
    // Scrolled so at least half is inside the window but less than half is
    // above the bar: what the bar covers does not count, so still not shown.
    await page.evaluate(() => {
      const outlet = document.querySelector('.xt-shell-outlet')!;
      const card = document
        .querySelector('[data-testid="telemetry-disclosure"]')!
        .getBoundingClientRect();
      const bar = document.querySelector('.xt-welcome-actions')!.getBoundingClientRect();
      const target = (bar.top + innerHeight) / 2 - card.height / 2;
      outlet.scrollTop += card.top - target;
    });
    await settleFrames(page);
    const straddle = await readableShare(page);
    expect(straddle.inWindow).toBeGreaterThanOrEqual(0.5);
    expect(straddle.aboveBar).toBeLessThan(0.5);
    expect(await shownCount(page)).toBe(0);
    // Scrolled so at least half is above the bar: shown, once.
    await page.evaluate(() => {
      const outlet = document.querySelector('.xt-shell-outlet')!;
      outlet.scrollTop = outlet.scrollHeight;
    });
    await settleFrames(page);
    expect((await readableShare(page)).aboveBar).toBeGreaterThanOrEqual(0.5);
    await expect.poll(() => shownCount(page)).toBe(1);
  });

/** Test-only counts: plausible indexed-history volumes, and the exact JSON integer limit. */
const largeCounts: Record<string, FixtureExport['db_counts']> = {
  large: { sessions: 12_345, records: 1_234_567, usage: 123_456 },
  limit: {
    sessions: Number.MAX_SAFE_INTEGER,
    records: Number.MAX_SAFE_INTEGER,
    usage: Number.MAX_SAFE_INTEGER,
  },
};
for (const [width, height] of [
  [1440, 900],
  [1120, 720],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const [name, counts] of Object.entries(largeCounts))
      test(`${name} database counts stay whole at ${width}x${height} ${scheme}`, async ({
        page,
      }, info) => {
        await serve(page, states.partial, false, counts);
        await page.setViewportSize({ width, height });
        await page.emulateMedia({ colorScheme: scheme });
        await page.goto('/first-launch');
        await expect(page.locator('.xt-welcome-total')).toContainText(
          counts.sessions.toLocaleString('en-US'),
        );
        const group = (await openDetails(page)).getByRole('group', { name: 'Database counts' });
        await expect(group).toContainText(counts.records.toLocaleString('en-US'));
        await page.evaluate(() => document.fonts.ready);
        const cells = await group.evaluate((root) =>
          [...root.querySelectorAll<HTMLElement>('.xt-welcome-count')].map((tile) => {
            const cell = tile.querySelector<HTMLElement>('.xt-metric-cell')!;
            const label = tile.querySelector<HTMLElement>('.xt-welcome-count-label')!;
            const range = document.createRange();
            range.selectNodeContents(cell);
            const text = range.getBoundingClientRect();
            const box = tile.getBoundingClientRect();
            const own = cell.getBoundingClientRect();
            const style = getComputedStyle(cell);
            return {
              text: cell.textContent,
              // The glyphs themselves, not the outer card: inside the span, the span inside its tile.
              overflow: cell.scrollWidth - cell.clientWidth,
              glyphsInside:
                text.left >= own.left - 0.5 &&
                text.right <= own.right + 0.5 &&
                own.left >= box.left - 0.5 &&
                own.right <= box.right + 0.5,
              ellipsis: style.textOverflow === 'ellipsis' && style.overflow !== 'visible',
              overlapsLabel:
                label.getBoundingClientRect().right > own.left + 0.5 &&
                label.getBoundingClientRect().bottom > own.top + 0.5,
            };
          }),
        );
        expect(cells.map((cell) => cell.text)).toEqual(
          [counts.sessions, counts.records, counts.usage].map((value) =>
            value.toLocaleString('en-US'),
          ),
        );
        for (const cell of cells) {
          expect(cell.overflow).toBeLessThanOrEqual(0);
          expect(cell.glyphsInside).toBe(true);
          expect(cell.ellipsis).toBe(false);
          expect(cell.overlapsLabel).toBe(false);
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        await expect(page.getByRole('button', { name: 'Open Dashboard' })).toBeVisible();
        await group.screenshot({ path: info.outputPath(`counts-${name}-${width}-${scheme}.png`) });
      });
