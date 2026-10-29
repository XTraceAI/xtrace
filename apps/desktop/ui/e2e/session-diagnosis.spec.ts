import { expect, test, type Page } from '@playwright/test';
import synthetic from '../src/app/session-diagnosis.synthetic.json' with { type: 'json' };
import { SESSION_WARNINGS_TEXT } from '../src/data/generated/SessionWarningsText';

const words = SESSION_WARNINGS_TEXT.diagnose;
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

async function open(page: Page, scheme: 'light' | 'dark') {
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto('/e2e/session-diagnosis.html?diagnosis&path=/sessions/s-warned');
  const row = page.getByTestId('warning-item').first();
  await expect(row).toBeVisible();
  return row;
}

for (const scheme of ['light', 'dark'] as const) {
  test(`Diagnose asks before sending, Stop cancels, and keyboard can try again, ${scheme}`, async ({
    page,
  }) => {
    const row = await open(page, scheme);
    const button = row.getByRole('button', { name: /^Diagnose this warning:/ });
    await button.focus();
    await page.keyboard.press('Enter');
    const confirm = row.getByTestId('diagnosis-confirm');
    await expect(confirm).toContainText(SESSION_WARNINGS_TEXT.diagnose_before_sending);
    await expect(confirm.getByRole('button', { name: words.confirm })).toBeFocused();
    if (process.env.WARNING_SHOTS)
      await row.screenshot({
        path: `${process.env.WARNING_SHOTS}/diagnosis-confirm-${scheme}.png`,
      });
    expect((await calls(page)).runs).toEqual([]);
    await page.keyboard.press('Escape');
    await expect(confirm).toHaveCount(0);
    await expect(button).toBeFocused();
    await page.keyboard.press('Enter');
    await page.keyboard.press('Enter');
    const stop = row.getByRole('button', { name: words.stop });
    await expect(stop).toBeFocused();
    if (process.env.WARNING_SHOTS)
      await row.screenshot({
        path: `${process.env.WARNING_SHOTS}/diagnosis-running-${scheme}.png`,
      });
    const sent = (await calls(page)).runs;
    expect(sent).toHaveLength(1);
    expect(sent[0]!.slice(0, 3)).toEqual([
      's-warned',
      await row.getAttribute('data-warning-id'),
      'UTC',
    ]);
    await page.keyboard.press('Enter');
    await expect(row.getByTestId('diagnosis-result')).toHaveAttribute('data-state', 'cancelled');
    expect((await calls(page)).stops).toEqual([sent[0]![3]]);
    await row.getByRole('button', { name: words.close, exact: true }).click();
    await expect(row.getByTestId('diagnosis-result')).toHaveCount(0);
    await expect(button).toBeFocused();
  });

  test(`Diagnose shows the full skill and saves only on the explicit click, ${scheme}`, async ({
    page,
  }) => {
    const row = await open(page, scheme);
    await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
    await row.getByRole('button', { name: words.confirm }).click();
    await expect(row.getByTestId('diagnosis-running')).toBeVisible();
    await answer(page, 'make_skill');
    const made = synthetic.diagnoses.make_skill;
    const result = row.getByTestId('diagnosis-result');
    await expect(result.getByRole('heading', { name: words.heading })).toBeFocused();
    await expect(result.getByTestId('diagnosis-draft')).toHaveText(made.draft);
    await expect(result.getByTestId('diagnosis-path')).toContainText(made.skill_path);
    await expect(result.getByTestId('diagnosis-ran')).toHaveText(made.ran);
    await expect(result.getByTestId('diagnosis-sent')).toContainText(made.sent.text);
    expect((await calls(page)).saves).toEqual([]);
    await result.getByRole('button', { name: words.save_skill }).click();
    await expect(result.getByTestId('diagnosis-saved')).toContainText(synthetic.saved.path);
    expect((await calls(page)).saves).toEqual([['s-warned', made.skill_name, made.draft]]);
    const widths = await result.evaluate((element) => ({
      content: element.scrollWidth,
      box: element.clientWidth,
    }));
    expect(widths.content).toBeLessThanOrEqual(widths.box);
    if (process.env.WARNING_SHOTS)
      await row.screenshot({ path: `${process.env.WARNING_SHOTS}/diagnosis-skill-${scheme}.png` });
  });
}

for (const scheme of ['light', 'dark'] as const)
  test(`an instruction suggestion copies only, with tool and sent-message evidence, ${scheme}`, async ({
    page,
  }) => {
    await page.addInitScript(() => {
      Object.assign(window, { __copied: [] });
      Object.defineProperty(navigator, 'clipboard', {
        value: {
          writeText: async (text: string) => {
            (window as unknown as { __copied: string[] }).__copied.push(text);
          },
        },
      });
    });
    const row = await open(page, scheme);
    await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
    await row.getByRole('button', { name: words.confirm }).click();
    await expect(row.getByTestId('diagnosis-running')).toBeVisible();
    await answer(page, 'add_to_claude_md');
    const made = synthetic.diagnoses.add_to_claude_md;
    await expect(row.getByTestId('diagnosis-line-text')).toHaveText(made.draft);
    await expect(row.getByTestId('diagnosis-line')).toContainText(made.instruction_file);
    await expect(row.getByTestId('diagnosis-ran')).toHaveText(made.ran);
    await expect(row.getByTestId('diagnosis-tried')).toContainText(made.tried[0]!.text);
    await expect(row.getByTestId('diagnosis-limits')).toContainText(made.limits[0]!.text);
    await row.getByRole('button', { name: words.copy }).click();
    await expect(row.getByTestId('diagnosis-copied')).toHaveText(words.copied);
    expect(
      await page.evaluate(() => (window as unknown as { __copied: string[] }).__copied),
    ).toEqual([made.draft]);
    expect((await calls(page)).saves).toEqual([]);
    await expect(row.getByRole('button', { name: words.save_skill })).toHaveCount(0);
    if (process.env.WARNING_SHOTS)
      await row.screenshot({
        path: `${process.env.WARNING_SHOTS}/diagnosis-instruction-${scheme}.png`,
      });
  });

for (const scheme of ['light', 'dark'] as const)
  for (const key of ['no_agent_tool', 'run_failed', 'answer_refused'] as const)
    test(`Diagnose states why it could not answer: ${key}, ${scheme}`, async ({ page }) => {
      const row = await open(page, scheme);
      await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
      await row.getByRole('button', { name: words.confirm }).click();
      await expect(row.getByTestId('diagnosis-running')).toBeVisible();
      await answer(page, key);
      await expect(row.getByTestId('diagnosis-text')).toHaveText(synthetic.diagnoses[key].text);
      if (key === 'no_agent_tool' && process.env.WARNING_SHOTS)
        await row.screenshot({
          path: `${process.env.WARNING_SHOTS}/diagnosis-no-tool-${scheme}.png`,
        });
    });

for (const scheme of ['light', 'dark'] as const) {
  for (const width of [1120, 1440]) {
    test(`clear diagnosis shows exact sent evidence and keeps details collapsed, ${scheme}, ${width}px`, async ({
      page,
    }, testInfo) => {
      const row = await open(page, scheme);
      await page.setViewportSize({ width, height: 850 });
      await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
      await row.getByRole('button', { name: words.confirm }).click();
      await expect(row.getByTestId('diagnosis-running')).toBeVisible();
      await answer(page, 'problem_no_suggestion');
      const result = row.getByTestId('diagnosis-result');
      await expect(result.getByTestId('diagnosis-real')).toHaveText('The model found a problem');
      await expect(result.getByTestId('diagnosis-suggestion')).toHaveText(
        'No skill or instruction change suggested.',
      );
      await expect(result.getByTestId('diagnosis-ran')).not.toBeVisible();
      const evidence = result.getByTestId('diagnosis-evidence');
      await expect(evidence).not.toHaveAttribute('open');
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/clear-diagnosis-${testInfo.project.name}-${scheme}-${width}.png`,
        });
      await result.getByRole('button', { name: '[13]', exact: true }).focus();
      await page.keyboard.press('Enter');
      const message = evidence.locator('[data-number="13"]');
      await expect(message).toBeFocused();
      await expect(message).toHaveText(
        'Message 13 · person · flaggedCheck the database setup and use pnpm for this project.',
      );
      await expect(evidence.getByTestId('diagnosis-message')).toHaveCount(20);
      await expect(evidence.locator('[data-number="1"]')).toContainText('[cut]');
      for (const target of [
        result.getByTestId('diagnosis-real'),
        result.getByTestId('diagnosis-suggestion'),
        result.getByTestId('diagnosis-evidence'),
        result.getByRole('button', { name: words.close, exact: true }),
      ]) {
        const box = await target.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
      }
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/clear-evidence-${testInfo.project.name}-${scheme}-${width}.png`,
        });
      await result.locator('summary', { hasText: words.details }).click();
      await expect(result.getByTestId('diagnosis-ran')).toBeVisible();
      const detailBounds = await result.getByTestId('diagnosis-details').boundingBox();
      expect(detailBounds!.x).toBeGreaterThanOrEqual(0);
      expect(detailBounds!.x + detailBounds!.width).toBeLessThanOrEqual(width);
      if (process.env.WARNING_SHOTS)
        await page.screenshot({
          path: `${process.env.WARNING_SHOTS}/clear-details-${testInfo.project.name}-${scheme}-${width}.png`,
        });
      expect((await calls(page)).runs).toHaveLength(1);
      await result.getByRole('button', { name: words.close, exact: true }).click();
      await expect(result).toHaveCount(0);
      await expect(row.getByRole('button', { name: /^Diagnose this warning:/ })).toBeFocused();
      await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
      await expect(row.getByRole('button', { name: words.confirm })).toBeVisible();
      expect((await calls(page)).runs).toHaveLength(1);
    });
  }
}

for (const scheme of ['light', 'dark'] as const) {
  test(`shared diagnosis wraps in a 390px component container, ${scheme}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 430, height: 850 });
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/e2e/session-diagnosis.html?diagnosis&diagnosislocal');
    const container = page.getByTestId('local-diagnosis');
    const row = container.getByTestId('warning-item').first();
    await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
    await row.getByRole('button', { name: words.confirm }).click();
    await expect(row.getByTestId('diagnosis-running')).toBeVisible();
    await answer(page, 'add_to_claude_md');
    const result = row.getByTestId('diagnosis-result');
    await expect(result.getByTestId('diagnosis-ran')).not.toBeVisible();
    await result.getByRole('button', { name: '[13]', exact: true }).click();
    await expect(result.locator('[data-number="13"]')).toBeFocused();
    await result.locator('summary', { hasText: words.details }).click();
    await expect(result.getByTestId('diagnosis-ran')).toBeVisible();
    const bounds = await container.boundingBox();
    expect(bounds!.width).toBe(390);
    const targets = [
      result,
      result.getByTestId('diagnosis-real'),
      result.getByTestId('diagnosis-suggestion'),
      result.getByTestId('diagnosis-evidence'),
      result.getByTestId('diagnosis-line-text'),
      result.getByTestId('diagnosis-details'),
      result.getByRole('button', { name: words.close, exact: true }),
    ];
    for (const target of targets) {
      const box = await target.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(bounds!.x);
      expect(box!.x + box!.width).toBeLessThanOrEqual(bounds!.x + bounds!.width);
      expect(await target.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
    }
    if (process.env.WARNING_SHOTS)
      await page.screenshot({
        path: `${process.env.WARNING_SHOTS}/clear-component-390-${testInfo.project.name}-${scheme}.png`,
      });
    expect((await calls(page)).runs).toHaveLength(1);
  });
}

// Each verdict reads at a glance: its own sign and its own colours, from the
// native verdict kind, with the native words beside the sign.
const verdicts = [
  { key: 'problem_no_suggestion', kind: 'yes', ink: '--danger', tint: '--danger-tint' },
  { key: 'nothing', kind: 'no', ink: '--success', tint: '--success-tint' },
  { key: 'not_sure', kind: 'not_sure', ink: '--secondary', tint: '--cell' },
] as const;
for (const scheme of ['light', 'dark'] as const)
  for (const { key, kind, ink, tint } of verdicts)
    test(`the ${kind} verdict is a callout with its own sign and colours, ${scheme}`, async ({
      page,
    }, testInfo) => {
      const row = await open(page, scheme);
      await row.getByRole('button', { name: /^Diagnose this warning:/ }).click();
      await row.getByRole('button', { name: words.confirm }).click();
      await expect(row.getByTestId('diagnosis-running')).toBeVisible();
      await answer(page, key);
      const made = synthetic.diagnoses[key];
      const verdict = row.getByTestId('diagnosis-real');
      await expect(verdict).toHaveText(made.real_problem_text);
      await expect(verdict).toHaveAttribute('data-value', kind);
      const sign = verdict.locator('.xt-diagnosis-sign');
      await expect(sign).toHaveAttribute('aria-hidden', 'true');
      await expect(sign.locator('svg')).toBeVisible();
      const drawn = await verdict.evaluate(
        (node, tokens) => {
          const probe = document.createElement('span');
          node.append(probe);
          probe.style.color = `var(${tokens.ink})`;
          probe.style.backgroundColor = `var(${tokens.tint})`;
          const want = getComputedStyle(probe);
          const result = {
            color: getComputedStyle(node).color,
            sign: getComputedStyle(node.querySelector('svg')!).stroke,
            background: getComputedStyle(node).backgroundColor,
            wantColor: want.color,
            wantBackground: want.backgroundColor,
          };
          probe.remove();
          return result;
        },
        { ink, tint },
      );
      expect(drawn.color).toBe(drawn.wantColor);
      expect(drawn.sign).toBe(drawn.wantColor);
      expect(drawn.background).toBe(drawn.wantBackground);
      if (process.env.WARNING_SHOTS)
        await row.getByTestId('diagnosis-result').screenshot({
          path: `${process.env.WARNING_SHOTS}/verdict-page-${kind}-${testInfo.project.name}-${scheme}.png`,
        });
    });
