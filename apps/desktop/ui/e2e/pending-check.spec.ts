import { expect, test, type Page } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };
import type { DashboardLaneSession } from '../src/data/generated/DashboardLaneSession';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionRow } from '../src/data/generated/SessionRow';

/**
 * Sessions hidden while XTrace has not finished checking whether another
 * session started them, in a real engine, over synthetic sessions (not real
 * history) served in place of the development F1 export. The Sessions page
 * says how many beside the table (the Rust count for these filters, which
 * includes sessions not loaded) and why an empty table is empty; the Dashboard
 * Sessions card says why it is empty and notes the hidden sessions in its
 * subtitle. Neither page starts scrolling to make room.
 */
const SHOWN = '01a0aaaa-0000-7000-8000-000000000001';
const CHECKING_A = '01a0bbbb-0000-7000-8000-000000000002';
const CHECKING_B = '01a0cccc-0000-7000-8000-000000000003';
const SHOWN_TITLE = 'A checked main session';
const CHECKING_EMPTY =
  'No session to show yet. Sessions are hidden while XTrace has not finished checking whether another session started them.';

/** Every window's export, with these sessions and the Rust count of those being checked. */
function exportWith(ids: readonly string[], checkingSessions: number): FixtureExport {
  const out = structuredClone(fixture as FixtureExport);
  const checking = (id: string) => id !== SHOWN;
  for (const report of out.dashboards) {
    const minute = 60_000;
    report.lanes = ids.map((session_id, index) => ({
      session_id,
      host: 'codex',
      start_ms: report.lane_end_ms - (index + 1) * 60 * minute,
      end_ms: report.lane_end_ms - (index + 1) * 60 * minute + 20 * minute,
    }));
    report.lanes_total = ids.length;
    report.lanes_truncated = false;
    const template = report.lane_sessions[0];
    report.lane_sessions = ids.map((session_id): DashboardLaneSession => ({
      ...structuredClone(template),
      session_id,
      host: 'codex',
      title: checking(session_id) ? null : SHOWN_TITLE,
      parent: null,
      known_child: false,
      child_check: checking(session_id) ? 'checking' : 'checked',
    }));
  }
  for (const page of out.sessions) {
    const template = page.rows[0];
    page.rows = ids.map((id): SessionRow => ({
      ...structuredClone(template),
      id,
      host: 'codex',
      title: checking(id) ? null : SHOWN_TITLE,
      parent: null,
      known_child: false,
      child_check: checking(id) ? 'checking' : 'checked',
    }));
  }
  const summary = out.sessions[0].summary!;
  out.sessions_summaries = out.sessions.map((page) => ({
    days: page.window.days,
    session_ids: [...ids].sort(),
    summary: {
      ...summary,
      main_sessions: ids.filter((id) => !checking(id)).length,
      checking_sessions: checkingSessions,
    },
  }));
  return out;
}

async function open(page: Page, exported: FixtureExport, path: string) {
  await page.route('**/fixtures/F1.json?import', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export default ${JSON.stringify(exported)};`,
    }),
  );
  await page.setViewportSize({ width: 1120, height: 720 });
  await page.goto(path);
  await page.evaluate(() => document.fonts.ready);
}

/** How far the page and the app's content area scroll beyond their boxes. */
const overflow = (page: Page) =>
  page.evaluate(() => {
    const outlet = document.querySelector<HTMLElement>('.xt-shell-outlet')!;
    return {
      outlet: outlet.scrollHeight - outlet.clientHeight,
      page: document.documentElement.scrollHeight - window.innerHeight,
    };
  });

test('the Sessions page says why every session is hidden, and how many', async ({ page }) => {
  await open(page, exportWith([CHECKING_A, CHECKING_B], 2), '/sessions?range=7d');
  const table = page.getByRole('table', { name: 'Indexed sessions' });
  await expect(table.getByText(CHECKING_EMPTY)).toBeVisible();
  const line = page.locator('.xt-sessions-checking');
  await expect(line).toHaveText(
    '2 sessions are hidden because XTrace has not finished checking whether another session started them.',
  );
  await expect(line).toBeVisible();
  // The line sits above the table, inside the card, and is not cut off.
  const placed = await line.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const grid = document
      .querySelector('[role="table"][aria-label="Indexed sessions"]')!
      .getBoundingClientRect();
    return {
      above: box.bottom <= grid.top + 0.5,
      clipped: element.scrollWidth > element.clientWidth,
    };
  });
  expect(placed).toEqual({ above: true, clipped: false });
  await expect(table).not.toContainText(CHECKING_A.slice(0, 8));
  // The page itself still does not scroll.
  const { outlet, page: doc } = await overflow(page);
  expect(outlet).toBeLessThanOrEqual(0);
  expect(doc).toBeLessThanOrEqual(0);
});

test('the Sessions page counts hidden sessions it has not loaded beside a listed one', async ({
  page,
}) => {
  await open(page, exportWith([SHOWN, CHECKING_A], 1234), '/sessions?range=7d');
  const table = page.getByRole('table', { name: 'Indexed sessions' });
  await expect(table.getByText(SHOWN_TITLE)).toBeVisible();
  await expect(page.locator('.xt-sessions-checking')).toHaveText(
    '1,234 sessions are hidden because XTrace has not finished checking whether another session started them.',
  );
  await expect(table).not.toContainText(CHECKING_A.slice(0, 8));
});

test('the Dashboard Sessions card says why it is empty and notes the hidden sessions', async ({
  page,
}) => {
  await open(page, exportWith([CHECKING_A, CHECKING_B], 2), '/dashboard');
  const card = page.getByRole('region', { name: 'Sessions' });
  const lanes = page.getByRole('table', { name: 'Session lanes' });
  await expect(
    lanes.getByText(
      /^No session to show yet in the last 48 hours \(.+\)\. Sessions are hidden while XTrace has not finished checking whether another session started them\.$/,
    ),
  ).toBeVisible();
  const meta = card.locator('.xt-section-meta');
  const full =
    'Active in the last 48 hours · 2 hidden: check not finished. 2 sessions are hidden because XTrace has not finished checking whether another session started them.';
  // The short line is drawn; the full sentence is its tooltip and what a
  // screen reader reads in its place.
  await expect(meta.locator('[aria-hidden="true"]')).toHaveText(
    'Active in the last 48 hours · 2 hidden: check not finished',
  );
  await expect(meta.locator('[aria-hidden="true"]')).toBeVisible();
  await expect(meta.locator('.sr-only')).toHaveText(full);
  await expect(meta).toHaveAttribute('title', full);
  await expect(lanes).not.toContainText(CHECKING_A.slice(0, 8));
  // The fixed Dashboard still fits the supported minimum window.
  const { outlet, page: doc } = await overflow(page);
  expect(outlet).toBeLessThanOrEqual(0);
  expect(doc).toBeLessThanOrEqual(0);
});

test('the Dashboard Sessions card notes hidden sessions while it lists others', async ({
  page,
}) => {
  await open(page, exportWith([SHOWN, CHECKING_A], 1), '/dashboard');
  const card = page.getByRole('region', { name: 'Sessions' });
  const lanes = page.getByRole('table', { name: 'Session lanes' });
  await expect(
    lanes.getByRole('link', { name: new RegExp(`^Open session ${SHOWN_TITLE}`) }),
  ).toBeVisible();
  await expect(card.locator('.xt-section-meta [aria-hidden="true"]')).toHaveText(
    'Active in the last 48 hours · 1 hidden: check not finished',
  );
  await expect(lanes).not.toContainText(CHECKING_A.slice(0, 8));
});
