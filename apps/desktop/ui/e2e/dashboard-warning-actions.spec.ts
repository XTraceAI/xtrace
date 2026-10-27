import { expect, test, type Page } from '@playwright/test';
import synthetic from '../src/app/session-warnings.synthetic.json' with { type: 'json' };
import diagnosis from '../src/app/session-diagnosis.synthetic.json' with { type: 'json' };
import { SESSION_WARNINGS_TEXT } from '../src/data/generated/SessionWarningsText';

const words = SESSION_WARNINGS_TEXT.diagnose;
const id = synthetic.after_dismissal.warning_id;
test.use({ timezoneId: 'UTC' });
type Controls = {
  calls: { runs: string[][]; stops: string[]; saves: string[][] };
  answer(key: string): void;
};
const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { __diagnosis: Controls }).__diagnosis.calls);
const answer = (page: Page, key: string) =>
  page.evaluate(
    (key) => (window as unknown as { __diagnosis: Controls }).__diagnosis.answer(key),
    key,
  );
const modal = (page: Page) => page.getByTestId('dashboard-warning-modal');
const close = (page: Page) => modal(page).getByRole('button', { name: 'Close', exact: true });
async function open(page: Page, theme: 'light' | 'dark', deferred = false) {
  await page.setViewportSize({ width: 1100, height: 800 });
  await page.emulateMedia({ colorScheme: theme });
  await page.goto(
    `/e2e/dashboard-warning.html?diagnosis&context${deferred ? '&cancel=deferred' : ''}&path=/dashboard?range=7d`,
  );
  await page.evaluate(() => document.fonts.ready);
}
async function dot(page: Page, surface: 'lane' | 'effort') {
  if (surface === 'effort')
    await page.getByRole('radio', { name: 'human + agent', exact: true }).click();
  const target =
    surface === 'lane'
      ? page.locator(`[data-testid="warning-dot"][data-warning-id="${id}"]`)
      : page.getByRole('button', {
          name: 'Fix the flaky build, Warning. Sep 7, Correction: you wrote "still not" after a reply (15:00)',
          exact: true,
        });
  await expect(target).toBeAttached();
  if (surface === 'effort') {
    const name = await target.getAttribute('aria-label');
    // The name gains the diagnosis state at its end while one runs or is kept.
    return page
      .getByTestId('day-warning-dot')
      .and(page.locator(`[aria-label^=${JSON.stringify(name)}]`));
  }
  return target;
}

for (const theme of ['light', 'dark'] as const) {
  for (const surface of ['lane', 'effort'] as const) {
    test(`Dashboard ${surface} modal dismisses and restores its dot without leaving Dashboard, ${theme}`, async ({
      page,
    }) => {
      await open(page, theme);
      const target = await dot(page, surface);
      await target.focus();
      await page.keyboard.press('Enter');
      await expect(modal(page)).toHaveAccessibleName('Correction Fix the flaky build');
      await expect(modal(page).getByTestId('warning-item')).toHaveCount(1);
      await expect(modal(page).getByTestId('warning-context')).toContainText('Still not fixed.');
      await expect(modal(page).getByTestId('warning-context')).toContainText(
        'The sample build is ready.',
      );
      await expect(modal(page).getByTestId('warning-context')).toContainText(
        'I will inspect the retry limit',
      );
      await expect(modal(page).getByTestId('warning-context')).not.toContainText(
        'Synthetic subagent',
      );
      await expect(
        modal(page).getByRole('button', { name: /^Diagnose this warning:/ }),
      ).toBeVisible();
      const location = page.url();
      const body =
        surface === 'effort'
          ? page.getByTestId('human-timeline-body')
          : page.locator('[role="table"][aria-label="Session lanes"]').locator('..');
      const scroll = await body.evaluate((node) => node.scrollTop);
      await modal(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
      const undo = modal(page).getByRole('button', { name: 'Undo', exact: true });
      await expect(undo).toBeFocused();
      await expect(target).toHaveCount(0);
      await expect(
        modal(page).getByRole('button', { name: /^Diagnose this warning:/ }),
      ).toHaveCount(0);
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-modal-dismissed-${theme}.png`,
        });
      await undo.click();
      await expect(modal(page).getByRole('button', { name: 'Dismiss', exact: true })).toBeFocused();
      await expect(target).toHaveCount(1);
      await close(page).click();
      await expect(modal(page)).toHaveCount(0);
      await expect(page).toHaveURL(location);
      expect(await body.evaluate((node) => node.scrollTop)).toBe(scroll);
      const presses = await page.evaluate(
        () => (window as unknown as { __dismissals: unknown }).__dismissals,
      );
      expect(presses).toEqual({ dismiss: [['s-warned', id]], restore: [['s-warned', id]] });
    });

    // Closing the window leaves the diagnosis running: the dot circles with
    // an arc and its hint says how long it has run, then that it finished.
    test(`Dashboard ${surface} modal asks first, keeps running after Close and shows it on the dot, ${theme}`, async ({
      page,
    }) => {
      await open(page, theme, true);
      const target = await dot(page, surface);
      await target.focus();
      await page.keyboard.press('Enter');
      const button = modal(page).getByRole('button', { name: /^Diagnose this warning:/ });
      await button.click();
      await expect(modal(page).getByRole('button', { name: words.confirm })).toBeFocused();
      expect((await calls(page)).runs).toEqual([]);
      await modal(page).getByRole('button', { name: words.not_now }).click();
      await expect(button).toBeFocused();
      await button.click();
      await page.keyboard.press('Escape');
      await expect(modal(page).getByTestId('diagnosis-confirm')).toHaveCount(0);
      await expect(button).toBeFocused();
      await button.click();
      await close(page).click();
      await expect(modal(page)).toHaveCount(0);
      expect((await calls(page)).runs).toEqual([]);
      await target.focus();
      await page.keyboard.press('Enter');
      await expect(modal(page).getByTestId('diagnosis-confirm')).toHaveCount(0);
      if (process.env.WARNING_SHOTS) {
        await expect(modal(page).getByTestId('warning-context')).toContainText('Still not fixed.');
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-modal-before-${theme}.png`,
        });
      }
      await button.click();
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-modal-confirm-${theme}.png`,
        });
      await modal(page).getByRole('button', { name: words.confirm }).click();
      const stop = modal(page).getByRole('button', { name: words.stop });
      await expect(stop).toBeFocused();
      const sent = (await calls(page)).runs;
      expect(sent).toHaveLength(1);
      expect(sent[0]!.slice(0, 3)).toEqual(['s-warned', id, 'UTC']);
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-modal-running-${theme}.png`,
        });
      if (surface === 'lane') await close(page).click();
      else await page.keyboard.press('Escape');
      await expect(modal(page)).toHaveCount(0);
      await expect(target).toBeFocused();
      expect((await calls(page)).stops).toEqual([]);
      await expect(target).toHaveAttribute('data-diagnosis', 'running');
      await expect(target).toHaveAccessibleName(new RegExp(`; ${words.dot_running_name}$`));
      const hint =
        surface === 'lane' ? page.getByTestId('warning-tip') : page.getByTestId('day-warning-tip');
      // A lane dot's hint opens on hover; an Effort dot's on focus.
      // Its right side sits under the neighbouring dot, so point at its left edge.
      if (surface === 'lane') await target.hover({ position: { x: 1, y: 5 } });
      await expect(hint.getByTestId('diagnosis-status')).toHaveText(/^Diagnosing… 0:0\d$/);
      if (process.env.WARNING_SHOTS) {
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-dot-running-${theme}.png`,
        });
        // Close up: the arc around the dot, and its hint.
        const box = (await target.boundingBox())!;
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-dot-running-closeup-${theme}.png`,
          clip: {
            x: Math.max(0, Math.min(box.x - 300, 500)),
            y: Math.max(0, box.y - 150),
            width: 600,
            height: 220,
          },
        });
      }
      // Reopened, it shows the same run with Stop, and focus stays at Close.
      await page.keyboard.press('Enter');
      await expect(stop).toBeVisible();
      await expect(stop).not.toBeFocused();
      await close(page).click();
      await expect(target).toBeFocused();
      // The answer arrives while the window is closed: the dot says it finished.
      await answer(page, 'make_skill');
      await expect(target).toHaveAttribute('data-diagnosis', 'finished');
      await expect(target).toHaveAccessibleName(new RegExp(`; ${words.dot_finished}$`));
      if (surface === 'lane') {
        await page.mouse.move(0, 0);
        await target.hover({ position: { x: 1, y: 5 } });
      }
      await expect(hint.getByTestId('diagnosis-status')).toHaveText(words.dot_finished);
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/${surface}-dot-finished-${theme}.png`,
        });
      await page.keyboard.press('Enter');
      await expect(modal(page).getByTestId('diagnosis-result')).toBeVisible();
      expect((await calls(page)).stops).toEqual([]);
    });
  }

  test(`Dashboard diagnosis uses Stop and shows a complete fake skill result before explicit Save, ${theme}`, async ({
    page,
  }) => {
    await open(page, theme);
    const target = await dot(page, 'lane');
    await target.focus();
    await page.keyboard.press('Enter');
    const button = modal(page).getByRole('button', { name: /^Diagnose this warning:/ });
    await button.click();
    await modal(page).getByRole('button', { name: words.confirm }).click();
    await modal(page).getByRole('button', { name: words.stop }).click();
    await expect(modal(page).getByTestId('diagnosis-result')).toHaveAttribute(
      'data-state',
      'cancelled',
    );
    await modal(page)
      .getByTestId('diagnosis-result')
      .getByRole('button', { name: words.close, exact: true })
      .click();
    await expect(button).toBeFocused();
    await button.click();
    await modal(page).getByRole('button', { name: words.confirm }).click();
    await answer(page, 'make_skill');
    const result = modal(page).getByTestId('diagnosis-result');
    await expect(result.getByRole('heading', { name: words.heading })).toBeFocused();
    await expect(result.getByTestId('diagnosis-draft')).toHaveText(
      diagnosis.diagnoses.make_skill.draft,
    );
    expect((await calls(page)).saves).toEqual([]);
    await result.getByRole('button', { name: words.save_skill }).click();
    await expect(result.getByTestId('diagnosis-saved')).toContainText(diagnosis.saved.path);
    expect((await calls(page)).saves).toEqual([
      ['s-warned', diagnosis.diagnoses.make_skill.skill_name, diagnosis.diagnoses.make_skill.draft],
    ]);
    if (process.env.WARNING_SHOTS)
      await page.screenshot({ path: `${process.env.WARNING_SHOTS}/modal-result-${theme}.png` });
    await close(page).click();
    await expect(modal(page)).toHaveCount(0);
    await expect(target).toBeFocused();
  });
}

for (const theme of ['light', 'dark'] as const) {
  test(`Dashboard context stays readable in a small window with long text, ${theme}`, async ({
    page,
  }) => {
    await open(page, theme);
    await page.goto('/e2e/dashboard-warning.html?diagnosis&context&long&path=/dashboard?range=7d');
    const target = await dot(page, 'lane');
    await target.focus();
    await page.keyboard.press('Enter');
    await expect(
      modal(page).getByRole('button', { name: 'Show the rest of this block' }),
    ).toBeVisible();
    await page.setViewportSize({ width: 540, height: 560 });
    const expand = modal(page).getByRole('button', { name: 'Show the rest of this block' });
    await expand.click();
    await expect(modal(page).getByRole('button', { name: 'Show less' })).toBeVisible();
    await expect(modal(page)).toBeVisible();
    const fits = await modal(page).evaluate((node) => node.scrollWidth <= node.clientWidth + 1);
    expect(fits).toBe(true);
    await modal(page).getByRole('button', { name: 'Show less' }).click();
    if (process.env.WARNING_SHOTS) {
      await page.screenshot({
        path: `${process.env.WARNING_SHOTS}/context-small-actions-${theme}.png`,
      });
      await modal(page).evaluate((node) => {
        node.scrollTop = 0;
      });
      await page.screenshot({ path: `${process.env.WARNING_SHOTS}/context-small-${theme}.png` });
    }
    await close(page).click();
    await expect(target).toBeFocused();
  });
}

for (const theme of ['light', 'dark'] as const) {
  test(`Dashboard shows surrounding messages from a subagent source, ${theme}`, async ({
    page,
  }) => {
    await open(page, theme);
    await page.goto(
      '/e2e/dashboard-warning.html?diagnosis&context=subagent&path=/dashboard?range=7d',
    );
    const target = await dot(page, 'lane');
    await target.focus();
    await page.keyboard.press('Enter');
    const context = modal(page).getByTestId('warning-context');
    await expect(context).toContainText('Synthetic subagent question.');
    await expect(context).toContainText('Synthetic subagent correction.');
    await expect(context).toContainText('Synthetic subagent reply.');
    await expect(context).not.toContainText('The sample build is ready.');
    // A sub-agent's messages are saved under the user role but are not a
    // person's (M-02): the neutral caption, never "You" on the right.
    await expect(context.locator('[data-own]')).toHaveCount(0);
    await expect(context).not.toContainText(SESSION_WARNINGS_TEXT.you);
    await expect(context.locator('.xt-chat-who').first()).toContainText('User');
    if (process.env.WARNING_SHOTS)
      await page.screenshot({
        path: `${process.env.WARNING_SHOTS}/subagent-neighbours-${theme}.png`,
      });
    await close(page).click();
    await expect(target).toBeFocused();
    expect((await calls(page)).runs).toEqual([]);
  });
}

for (const theme of ['light', 'dark'] as const)
  test(`saved native excerpt from a synthetic session stays small, ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 800 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/e2e/dashboard-warning.html?diagnosis&context=large&path=/dashboard?range=7d');
    const target = await dot(page, 'lane');
    await target.focus();
    await page.keyboard.press('Enter');
    const context = modal(page).getByTestId('warning-context');
    await expect(context).toContainText('Still not fixed. Please check the retry limit.');
    await expect(context).toContainText('The sample build is ready.');
    await expect(context).toContainText('I will inspect the retry limit');
    await expect(context.locator('.xt-warning-context-message')).toHaveCount(3);
    await expect(context.locator('[data-trigger]')).toHaveCount(1);
    await expect(modal(page).getByRole('button', { name: 'Dismiss', exact: true })).toBeVisible();
    await expect(
      modal(page).getByRole('button', { name: /^Diagnose this warning:/ }),
    ).toBeVisible();
    if (process.env.WARNING_SHOTS)
      await page.screenshot({ path: `${process.env.WARNING_SHOTS}/large-context-${theme}.png` });
  });

for (const theme of ['light', 'dark'] as const) {
  test(`Dashboard uses the same exact diagnosis evidence without navigating, ${theme}`, async ({
    page,
  }, testInfo) => {
    await open(page, theme);
    const target = await dot(page, 'lane');
    await target.focus();
    await page.keyboard.press('Enter');
    const dialog = modal(page);
    const location = page.url();
    await dialog.getByRole('button', { name: /^Diagnose this warning:/ }).click();
    await dialog.getByRole('button', { name: words.confirm }).click();
    await expect(dialog.getByTestId('diagnosis-running')).toBeVisible();
    await answer(page, 'not_sure');
    await expect(dialog.getByTestId('diagnosis-real')).toHaveText(
      'The messages were not enough to tell',
    );
    await expect(dialog.getByTestId('diagnosis-ran')).not.toBeVisible();
    await expect(dialog.getByRole('button', { name: '[999]', exact: true })).toHaveCount(0);
    await expect(dialog.getByText(words.reference_unavailable, { exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: '[11]', exact: true }).click();
    await expect(dialog.locator('[data-number="11"]')).toBeFocused();
    await expect(dialog.locator('[data-number="11"]')).toContainText(
      'The tests failed to connect to the database.',
    );
    expect((await calls(page)).runs).toHaveLength(1);
    await expect(page).toHaveURL(location);
    if (process.env.WARNING_SHOTS)
      await page.screenshot({
        path: `${process.env.WARNING_SHOTS}/clear-modal-evidence-${testInfo.project.name}-${theme}.png`,
      });
    await dialog.getByRole('button', { name: words.close, exact: true }).click();
    await expect(dialog.getByTestId('diagnosis-result')).toHaveCount(0);
    await expect(dialog).toBeVisible();
    await expect(close(page)).toBeVisible();
  });
}

for (const theme of ['light', 'dark'] as const) {
  for (const surface of ['lane', 'effort'] as const) {
    test(`closing a dismissed ${surface} warning returns to the remaining row in ${theme}`, async ({
      page,
    }) => {
      await open(page, theme);
      const target = await dot(page, surface);
      await target.focus();
      await page.keyboard.press('Enter');
      await modal(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
      await expect(target).toHaveCount(0);
      await expect(modal(page).getByRole('button', { name: 'Undo', exact: true })).toBeFocused();
      await close(page).click();
      await expect(modal(page)).toHaveCount(0);
      const fallback =
        surface === 'effort'
          ? page.locator('[data-testid="human-day"][data-date="2026-09-07"]')
          : page
              .getByRole('table', { name: 'Session lanes' })
              .getByRole('link', { name: /Open session Fix the flaky build/ });
      await expect(fallback).toBeFocused();
    });
  }
}
