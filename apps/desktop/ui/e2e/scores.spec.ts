import { expect, test, type Page } from '@playwright/test';

test.use({ viewport: { width: 1120, height: 720 } });

/** Every call the harness's grading controls received. */
type Calls = {
  overviews: number;
  installed: number;
  starts: string[][];
  cancels: string[];
  adds: string[];
  removes: string[];
  makes: string[];
  lookups: string[];
  linkCancels: number;
  keeps: [string, string[]][];
  discards: string[];
  details: string[];
  reports: string[];
};
const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { __grading: Calls }).__grading);
const harness = (scenario: string, path = '/scores', start?: string, picker?: string, extra = '') =>
  `/e2e/scores.html?scenario=${scenario}&path=${encodeURIComponent(path)}${start ? `&start=${start}` : ''}${picker ? `&picker=${picker}` : ''}${extra}`;

/** Nothing runs past the page's right edge and the page never scrolls sideways. */
const assertFits = async (page: Page) => {
  const geometry = await page.evaluate(() => {
    const root = document.querySelector<HTMLElement>('.xt-scores')!;
    const right = root.getBoundingClientRect().right + 0.5;
    const outside = [...root.querySelectorAll(':not(.xt-table-scroll *)')].filter(
      (element) => element.getBoundingClientRect().right > right,
    ).length;
    const table = root.querySelector<HTMLElement>('.xt-table-scroll');
    return {
      width: document.documentElement.scrollWidth,
      outside,
      sideways: table ? table.scrollWidth - table.clientWidth : 0,
    };
  });
  expect(geometry).toEqual({ width: page.viewportSize()!.width, outside: 0, sideways: 0 });
};

const watchErrors = (page: Page) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  return errors;
};

const states = [
  'empty',
  'graded',
  'partial',
  'running',
  'running_unlisted',
  'waiting',
  'long_findings',
  'failed',
  'no_projects',
] as const;

/** When the synthetic running run started (`scores.synthetic.json`). */
const STARTED = 1791193200000;

test('every page state fits, drawn from the Rust rule’s answers', async ({ page }, info) => {
  const errors = watchErrors(page);
  // A going run reads four minutes in.
  await page.clock.setFixedTime(STARTED + 4 * 60_000 + 10_000);
  for (const scheme of ['light'] as const) {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    for (const state of states) {
      await page.goto(harness(state));
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Scores');
      await expect(page.locator('html')).toHaveAttribute('data-theme', scheme);
      await expect(page.locator('.xt-score-card')).toHaveCount(1);
      // Add a grading skill is the grid's last card, the same height as the skill card beside it.
      await expect(page.locator('.xt-scores-grid > li').last()).toHaveClass('xt-score-add');
      await expect(page.getByRole('button', { name: 'Add a grading skill' })).toBeVisible();
      const [skillBox, addBox] = await Promise.all([
        page.locator('.xt-score-card').boundingBox(),
        page.locator('.xt-score-add-button').boundingBox(),
      ]);
      expect(Math.abs(skillBox!.height - addBox!.height)).toBeLessThanOrEqual(0.5);
      expect(Math.abs(skillBox!.y - addBox!.y)).toBeLessThanOrEqual(0.5);
      await expect(page.getByRole('button', { name: 'Scores', exact: true })).toHaveAttribute(
        'aria-current',
        'page',
      );
      await assertFits(page);
      await page.screenshot({ path: info.outputPath(`scores-${state}-${scheme}.png`) });
      await page.goto(harness(state, '/scores/claude-md-doctor'));
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('CLAUDE.md doctor');
      await expect(page.getByRole('button', { name: 'Add a grading skill' })).toHaveCount(0);
      await assertFits(page);
      await page.screenshot({ path: info.outputPath(`skill-${state}-${scheme}.png`) });
    }
  }
  expect(errors).toEqual([]);
});

test('a card shows its average, count and findings, and opens the skill by pointer or keyboard', async ({
  page,
}) => {
  await page.goto(harness('graded'));
  const card = page.locator('.xt-score-card');
  await expect(card.locator('.xt-score-big')).toHaveText('75');
  await expect(card).toContainText('3 of 4 projects graded');
  await expect(card.locator('.xt-score-insight')).toHaveCount(3);
  await expect(card.locator('.xt-score-insight').first()).toContainText('AGENTS.md is not loaded');
  // The whole card is the link's target.
  const box = (await card.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 8);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('CLAUDE.md doctor');
  await page.getByRole('link', { name: 'Back to Scores' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Scores');
  await page.getByRole('link', { name: 'CLAUDE.md doctor' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('CLAUDE.md doctor');
});

test('Add a grading skill is reached by Tab after the skill card and lists installed skills by group', async ({
  page,
}, info) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto(harness('graded'));
  await page.getByRole('link', { name: 'CLAUDE.md doctor' }).focus();
  await page.keyboard.press('Tab');
  const trigger = page.getByRole('button', { name: 'Add a grading skill' });
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Add a grading skill' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
  const heading = (title: string) =>
    dialog.getByRole('heading', { level: 3, name: new RegExp(`^${title}`) });
  await expect(dialog.getByRole('list', { name: 'How a skill gets on Scores' })).toBeVisible();
  await expect(heading('Ready to add')).toHaveText('Ready to add3');
  await expect(heading('Your skills')).toHaveText('Your skills5');
  await expect(heading('Already on Scores')).toContainText('Already on Scores2');
  await expect(heading('Can’t be scored')).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: /^Add / })).toHaveCount(2);
  await expect(dialog).toContainText('Same id as Lint check; only one can be added');
  await expect(dialog).toContainText('Its points do not add up to 100.');
  await expect(dialog).toContainText(
    'Make a rubric runs Claude Code in the background to write one for that skill; it uses about a minute of your Claude usage.',
  );
  // Not for the copy of the built-in skill: Scores already has it.
  await expect(dialog.getByRole('button', { name: /^Make a rubric for / })).toHaveCount(3);
  await expect(
    dialog.getByRole('button', { name: 'Make a rubric for claude-md-doctor' }),
  ).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: /^Remake the rubric of / })).toHaveCount(2);
  // A rubric's `---` is drawn as three dashes, not joined into a long one.
  const reason = dialog.locator('.xt-score-add-reason', { hasText: 'closing --- line' });
  await expect(reason).toHaveText('Its header has no closing --- line.');
  const dashes = await reason.evaluate((element: HTMLElement) => {
    // The same text with every ligature off, beside it, is the reference.
    const plain = element.cloneNode(true) as HTMLElement;
    plain.style.fontVariantLigatures = 'none';
    element.after(plain);
    const width = (node: HTMLElement) => {
      const text = node.firstChild!;
      const at = text.textContent!.indexOf('---');
      const range = document.createRange();
      range.setStart(text, at);
      range.setEnd(text, at + 3);
      return range.getBoundingClientRect().width;
    };
    const widths = { shown: width(element), plain: width(plain) };
    plain.remove();
    return widths;
  });
  expect(Math.abs(dashes.shown - dashes.plain)).toBeLessThan(0.5);
  // The dialog fits the window and nothing in it runs sideways.
  const fits = await dialog.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const list = element.querySelector<HTMLElement>('.xt-score-add-list')!;
    return {
      inside: box.bottom <= window.innerHeight && box.right <= window.innerWidth,
      sideways: list.scrollWidth - list.clientWidth,
    };
  });
  expect(fits).toEqual({ inside: true, sideways: 0 });
  await page.screenshot({ path: info.outputPath('add-skill-dialog-dark.png') });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.screenshot({ path: info.outputPath('add-skill-dialog-light.png') });
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
  const recorded = await calls(page);
  expect(recorded.adds).toEqual([]);
  expect(recorded.starts).toEqual([]);
});

test('a long installed list scrolls inside the dialog, by wheel and by keyboard', async ({
  page,
}, info) => {
  await page.goto(harness('graded', '/scores', undefined, 'many'));
  await page.getByRole('button', { name: 'Add a grading skill' }).click();
  const dialog = page.getByRole('dialog', { name: 'Add a grading skill' });
  const list = dialog.getByRole('region', { name: 'Installed skills' });
  await expect(list).toBeVisible();
  await expect(dialog.getByRole('heading', { level: 3, name: /^Your skills/ })).toHaveText(
    'Your skills41',
  );
  const geometry = await dialog.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const scroller = element.querySelector<HTMLElement>('.xt-score-add-list')!;
    return {
      inside: box.top >= 0 && box.bottom <= window.innerHeight,
      scrolls: scroller.scrollHeight > scroller.clientHeight,
      page: element.scrollHeight <= element.clientHeight + 1,
    };
  });
  expect(geometry).toEqual({ inside: true, scrolls: true, page: true });
  await list.hover();
  await page.mouse.wheel(0, 300);
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await list.focus();
  await page.keyboard.press('End');
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(100);
  await expect(dialog.getByText('tool-35')).toBeInViewport();
  await page.screenshot({ path: info.outputPath('add-skill-dialog-scrolled.png') });
  // The search narrows the list.
  await dialog.getByRole('searchbox', { name: 'Search installed skills' }).fill('tool-0');
  await expect(dialog.getByRole('heading', { level: 3, name: /^Your skills/ })).toHaveText(
    'Your skills10 of 41',
  );
});

test('an added skill that cannot grade keeps its card and says why', async ({ page }, info) => {
  await page.goto(harness('added_problem', '/scores', 'skill_unavailable'));
  const readme = page.locator('.xt-score-card').filter({ hasText: 'README check' });
  await expect(readme).toContainText('Can’t grade now.');
  // Reasons on the page draw `---` as three dashes too (no joined dashes).
  await expect(readme.locator('.xt-score-card-problem')).toHaveCSS(
    'font-variant-ligatures',
    'no-common-ligatures',
  );
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('scores-added-problem.png') });
  await readme.getByRole('link', { name: 'README check' }).click();
  await expect(page.getByText('This skill can’t grade right now')).toBeVisible();
  await expect(page.locator('.xt-scores-notice').last()).toHaveCSS(
    'font-variant-ligatures',
    'no-common-ligatures',
  );
  // Grade still asks native, which answers that it cannot grade now; the
  // reason is shown once, in the notice above.
  await page.getByRole('button', { name: 'Grade again atlas' }).click();
  await expect(page.locator('.xt-scores-message')).toHaveText(
    'This skill can’t grade right now, so nothing was graded.',
  );
  expect((await calls(page)).starts).toEqual([['/work/atlas', 'readme-check']]);
  await expect(page.getByText(/its points do not add up to 100/)).toHaveCount(1);
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('added-skill-problem-page.png') });
});

test('a Grade pressed while one runs is sent for native to put in line', async ({ page }) => {
  await page.goto(harness('running', '/scores/claude-md-doctor', 'waiting'));
  await expect(page.getByRole('row').filter({ hasText: 'delta' })).toContainText('Grading now');
  const again = page.getByRole('button', { name: 'Grade again atlas' });
  await expect(again).toBeEnabled();
  await again.click();
  await expect
    .poll(async () => (await calls(page)).starts)
    .toEqual([['/work/atlas', 'claude-md-doctor']]);
  await expect(page.locator('.xt-scores-message')).toHaveText('');
});

test('a finding’s title wraps to two lines at most, then ends in an ellipsis', async ({ page }) => {
  await page.goto(harness('long_findings'));
  const titles = page.locator('.xt-score-insight-title');
  await expect(titles).toHaveCount(2);
  const drawn = await titles.evaluateAll((elements) =>
    elements.map((element) => {
      const style = getComputedStyle(element);
      return {
        lines: Math.round(element.clientHeight / parseFloat(style.lineHeight)),
        clipped: element.scrollHeight > element.clientHeight + 1,
        clamp: style.getPropertyValue('-webkit-line-clamp'),
        tooltip: element.getAttribute('title') === element.textContent,
      };
    }),
  );
  for (const title of drawn) {
    expect(title.lines).toBeLessThanOrEqual(2);
    expect(title.clamp).toBe('2');
    expect(title.tooltip).toBe(true);
  }
  // The longest title is cut after two lines; the whole of it is the tooltip.
  expect(drawn[0]).toEqual({ lines: 2, clipped: true, clamp: '2', tooltip: true });
  await assertFits(page);
});

test('a waiting row shows its place with Cancel, and a waiting grading no row shows is listed', async ({
  page,
}, info) => {
  const errors = watchErrors(page);
  await page.clock.setFixedTime(STARTED + 4 * 60_000 + 10_000);
  await page.goto(harness('waiting'));
  const card = page.locator('.xt-score-card');
  await expect(card).toContainText('Grading · 0 h 4 m');
  await expect(card.locator('.xt-score-card-count')).toHaveText(
    '3 of 4 projects graded · 2 waiting',
  );
  const elsewhere = page.getByTestId('waiting-elsewhere');
  await expect(elsewhere).toContainText(
    '/work/old-beacon is waiting to be graded with CLAUDE.md doctor · 1 ahead',
  );
  await expect(elsewhere).toContainText('This project is no longer in your project list');
  await assertFits(page);
  await page.goto(harness('waiting', '/scores/claude-md-doctor'));
  const atlas = page.getByRole('row').filter({ hasText: 'atlas' });
  await expect(atlas).toContainText('Waiting · next in line');
  await expect(atlas.getByRole('button', { name: /Grade/ })).toHaveCount(0);
  // Cancel is reached by keyboard like any row action.
  const cancel = atlas.getByRole('button', { name: 'Cancel waiting grading atlas' });
  await cancel.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.xt-scores-message')).toHaveText('Cancelling the grading…');
  await page
    .getByTestId('waiting-elsewhere')
    .getByRole('button', { name: 'Cancel waiting grading /work/old-beacon' })
    .click();
  await expect
    .poll(async () => (await calls(page)).cancels)
    .toEqual(['00000000-0000-4000-8000-0000000000c1', '00000000-0000-4000-8000-0000000000c2']);
  expect((await calls(page)).starts).toEqual([]);
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('skill-waiting-cancelled.png') });
  expect(errors).toEqual([]);
});

/** When the synthetic rubric job started (`picker.synthetic.json`). */
const MAKING_STARTED = 1784000000000;

test('Make a rubric shows the job going with Cancel, then why it failed or the made criteria', async ({
  page,
}, info) => {
  const errors = watchErrors(page);
  await page.clock.setFixedTime(MAKING_STARTED + 70_000);
  const row = (name: string) =>
    page
      .getByRole('dialog', { name: 'Add a grading skill' })
      .locator('.xt-score-add-row')
      .filter({ has: page.locator('.xt-score-add-name', { hasText: new RegExp(`^${name}$`) }) });
  const shoot = async (name: string, target: string) => {
    await row(target).scrollIntoViewIfNeeded();
    const fits = await page
      .getByRole('dialog', { name: 'Add a grading skill' })
      .evaluate((element) => {
        const box = element.getBoundingClientRect();
        const list = element.querySelector<HTMLElement>('.xt-score-add-list')!;
        return {
          inside: box.bottom <= window.innerHeight && box.right <= window.innerWidth,
          sideways: list.scrollWidth - list.clientWidth,
        };
      });
    expect(fits).toEqual({ inside: true, sideways: 0 });
    await page.screenshot({ path: info.outputPath(`${name}.png`) });
  };
  for (const scheme of ['light'] as const) {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.goto(harness('graded'));
    await page.getByRole('button', { name: 'Add a grading skill' }).click();
    const dialog = page.getByRole('dialog', { name: 'Add a grading skill' });
    await expect(
      dialog.locator('[data-group="yours"] .xt-score-add-row', { hasText: 'deploy' }),
    ).toHaveCount(1);
    await expect(
      row('deploy').getByRole('button', { name: 'Make a rubric for deploy' }),
    ).toHaveText('Make a rubric');
    await shoot(`maker-needs-rubric-${scheme}`, 'deploy');
    await expect(
      row('release-notes').getByRole('button', { name: 'Remake the rubric of release-notes' }),
    ).toHaveText('Remake rubric');
    await shoot(`maker-rubric-problem-${scheme}`, 'release-notes');
    // Make sends only the location; the row then shows the job going.
    await row('deploy').getByRole('button', { name: 'Make a rubric for deploy' }).click();
    await expect(row('deploy')).toContainText('Making a rubric · 0 h 1 m');
    const cancel = row('deploy').getByRole('button', { name: 'Cancel making a rubric for deploy' });
    await expect(cancel).toBeVisible();
    await expect(page.getByTestId('making-rubric')).toContainText(
      'A rubric for deploy is being made · 0 h 1 m',
    );
    await shoot(`maker-making-${scheme}`, 'deploy');
    // Cancel sends the job's ID; the rows then say how each job ended.
    await expect(cancel).not.toHaveAttribute('aria-disabled', 'true');
    await cancel.click();
    await expect(row('deploy')).toContainText(
      'No rubric was made: the rubric Claude wrote was not valid on a second try either',
    );
    await expect(
      row('deploy').getByRole('button', { name: 'Try making a rubric again for deploy' }),
    ).toHaveText('Try again');
    await expect(row('frontend-design')).toContainText('Making a rubric was cancelled.');
    await shoot(`maker-failed-${scheme}`, 'deploy');
    // A skill Claude judged unfit for scoring: folded under Can't be scored,
    // opened it says why, with Try again.
    await dialog.getByRole('button', { name: /^Can’t be scored/ }).click();
    await expect(row('pdf')).toContainText(
      'This skill reads, fills and merges PDF files; it does not review a project, so its output cannot be scored.',
    );
    await expect(
      row('pdf').getByRole('button', { name: 'Try making a rubric again for pdf' }),
    ).toHaveText('Try again');
    await shoot(`maker-refused-${scheme}`, 'pdf');
    await expect(dialog).toBeVisible();
    const recorded = await calls(page);
    expect(recorded.makes).toEqual(['/home/you/.claude/skills/deploy']);
    expect(recorded.cancels).toEqual(['00000000-0000-4000-8000-0000000000aa']);
    expect(recorded.starts).toEqual([]);
    // A made rubric: the row is ready to add, with its criteria.
    await page.goto(harness('graded', '/scores', undefined, 'made'));
    await page.getByRole('button', { name: 'Add a grading skill' }).click();
    await expect(row('Deploy')).toContainText(
      'Rubric made: Build passes 40 · Staging deploy succeeds 40 · Rollback documented 20',
    );
    await expect(row('Deploy').getByRole('button', { name: 'Add Deploy' })).toBeVisible();
    await expect(row('Frontend design')).toContainText('Rubric made and kept by XTrace');
    await shoot(`maker-made-${scheme}`, 'Deploy');
  }
  // A job going holds the one slot: the Scores page names it with Cancel,
  // and a Grade press is sent for native to put in line.
  await page.goto(harness('making_rubric', '/scores/claude-md-doctor', 'waiting'));
  await expect(page.getByTestId('making-rubric')).toContainText(
    'A rubric for deploy is being made',
  );
  await expect(page.getByTestId('making-rubric')).toContainText(
    'Gradings you start wait until it is done.',
  );
  await page.getByRole('button', { name: 'Grade again atlas' }).click();
  await expect
    .poll(async () => (await calls(page)).starts)
    .toEqual([['/work/atlas', 'claude-md-doctor']]);
  await expect(page.locator('.xt-scores-message')).toHaveText('');
  await assertFits(page);
  await page.screenshot({ path: info.outputPath('maker-skill-page-busy.png') });
  expect(errors).toEqual([]);
});

test('the panel: three steps, an empty Ready to add, one list of your skills, folded groups', async ({
  page,
}, info) => {
  const errors = watchErrors(page);
  const dialog = page.getByRole('dialog', { name: 'Add a grading skill' });
  const list = dialog.getByRole('region', { name: 'Installed skills' });
  const fits = async () => {
    const geometry = await dialog.evaluate((element) => {
      const box = element.getBoundingClientRect();
      const scroller = element.querySelector<HTMLElement>('.xt-score-add-list')!;
      return {
        inside: box.top >= 0 && box.bottom <= window.innerHeight && box.right <= window.innerWidth,
        sideways: scroller.scrollWidth - scroller.clientWidth,
        page: element.scrollHeight <= element.clientHeight + 1,
      };
    });
    expect(geometry).toEqual({ inside: true, sideways: 0, page: true });
  };
  for (const scheme of ['light'] as const) {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    // A home like a real one: nothing ready, 50 skills, 2 refused, 1 built in.
    await page.goto(harness('graded', '/scores', undefined, 'yours'));
    await page.getByRole('button', { name: 'Add a grading skill' }).click();
    await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
    await expect(list).toBeVisible();
    const steps = dialog.getByRole('list', { name: 'How a skill gets on Scores' });
    await expect(steps.getByRole('listitem')).toHaveText([
      '1Pick a skillOne you already use',
      '2Make a rubricXTrace writes it, about 1 minute',
      '3Add itIt gets its own score card',
    ]);
    const heading = (title: string) =>
      dialog.getByRole('heading', { level: 3, name: new RegExp(`^${title}`) });
    await expect(heading('Ready to add')).toHaveText('Ready to add0');
    await expect(dialog.locator('[data-group="ready"]')).toContainText(
      'Make a rubric for one of your skills below.',
    );
    await expect(heading('Your skills')).toHaveText('Your skills47');
    const cant = dialog.getByRole('button', { name: /^Can’t be scored/ });
    const already = dialog.getByRole('button', { name: /^Already on Scores/ });
    await expect(cant).toContainText('Can’t be scored2');
    await expect(already).toContainText('Already on Scores1');
    await expect(cant).toHaveAttribute('aria-expanded', 'false');
    await expect(already).toHaveAttribute('aria-expanded', 'false');
    await fits();
    await page.screenshot({ path: info.outputPath(`panel-yours-${scheme}.png`) });
    // Opened by keyboard, the folded groups show their rows at the bottom.
    await cant.focus();
    await page.keyboard.press('Enter');
    await expect(cant).toHaveAttribute('aria-expanded', 'true');
    await already.click();
    await expect(already).toHaveAttribute('aria-expanded', 'true');
    const refused = dialog.locator('[data-group="cant_score"] .xt-score-add-row');
    await expect(refused).toHaveCount(2);
    await expect(refused.first()).toContainText('it needs project changes and the network');
    await expect(
      dialog.getByRole('button', { name: /^Try making a rubric again for / }),
    ).toHaveCount(2);
    await expect(dialog.locator('[data-group="on_scores"] .xt-score-add-row')).toContainText(
      'On Scores (built in)',
    );
    await list.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await expect(
      dialog.getByRole('button', { name: 'Try making a rubric again for framer' }),
    ).toBeInViewport();
    await fits();
    await page.screenshot({ path: info.outputPath(`panel-folded-open-${scheme}.png`) });
    // The search reaches folded groups: a closed one shows its match count.
    await cant.click();
    await dialog.getByRole('searchbox', { name: 'Search installed skills' }).fill('framer');
    await expect(cant).toContainText('Can’t be scored1 of 2');
    await expect(heading('Your skills')).toHaveText('Your skills0 of 47');
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    // Skills ready to add, each with its own Add.
    await page.goto(harness('graded', '/scores', undefined, 'made'));
    await page.getByRole('button', { name: 'Add a grading skill' }).click();
    await expect(heading('Ready to add')).toHaveText('Ready to add5');
    await expect(
      dialog.locator('[data-group="ready"]').getByRole('button', { name: /^Add / }),
    ).toHaveCount(4);
    await fits();
    await page.screenshot({ path: info.outputPath(`panel-ready-${scheme}.png`) });
  }
  expect((await calls(page)).makes).toEqual([]);
  expect(errors).toEqual([]);
});

/** Where the link screenshots go when asked (`XTRACE_LINK_SHOTS`), else the test's output. */
const linkShot = (
  info: { outputPath: (name: string) => string; project: { name: string } },
  name: string,
) =>
  process.env.XTRACE_LINK_SHOTS
    ? `${process.env.XTRACE_LINK_SHOTS}/${info.project.name}-${name}.png`
    : info.outputPath(`${name}.png`);

test('Add from a link: look up, preview several with clashes, keep the chosen ones', async ({
  page,
}, info) => {
  const errors = watchErrors(page);
  const dialog = page.getByRole('dialog', { name: 'Add a grading skill' });
  const box = dialog.getByRole('region', { name: 'Add from a link' });
  const field = box.getByRole('textbox', { name: 'GitHub link' });
  const fits = async () => {
    const geometry = await dialog.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const preview = element.querySelector<HTMLElement>('.xt-score-link-list');
      return {
        inside:
          rect.top >= 0 && rect.bottom <= window.innerHeight && rect.right <= window.innerWidth,
        sideways: preview ? preview.scrollWidth - preview.clientWidth : 0,
        page: element.scrollHeight <= element.clientHeight + 1,
      };
    });
    expect(geometry).toEqual({ inside: true, sideways: 0, page: true });
  };
  for (const scheme of ['light'] as const) {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.goto(`${harness('graded', '/scores', undefined, 'yours')}&link=several`);
    await page.getByRole('button', { name: 'Add a grading skill' }).click();
    await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
    // The field is the next stop after Close, above the three steps.
    await page.keyboard.press('Tab');
    await expect(field).toBeFocused();
    await expect(box).toContainText(
      'Paste a public GitHub link to a skill or a repository of skills.',
    );
    await page.keyboard.type('https://github.com/octo/team-skills');
    await page.keyboard.press('Enter');
    const preview = box.getByRole('region', { name: 'Skills at this link' });
    await expect(preview).toBeVisible();
    await expect(box.locator('.xt-score-link-source')).toHaveText(
      'github.com/octo/team-skills · 0123456Top of the repository · default branch',
    );
    await expect(box.locator('.xt-score-link-row')).toHaveCount(6);
    await expect(box.getByRole('checkbox')).toHaveCount(4);
    await expect(box.locator('[data-keep="name_in_agents"]')).toContainText(
      '~/.agents/skills/taken already exists, so it can’t be kept',
    );
    await expect(box.locator('[data-keep="name_in_claude"]')).toContainText(
      '~/.claude/skills/claude-taken already exists, so it can’t be kept',
    );
    const keep = box.getByRole('button', { name: 'Keep chosen skills' });
    await expect(keep).toHaveAttribute('aria-disabled', 'true');
    // The installed list is still there below, smaller.
    await expect(dialog.getByRole('region', { name: 'Installed skills' })).toBeVisible();
    await fits();
    await page.screenshot({ path: linkShot(info, `preview-several-${scheme}`) });
    // Chosen by keyboard: Space ticks the focused box.
    await box.getByRole('checkbox', { name: 'changelog' }).focus();
    await page.keyboard.press('Space');
    await box.getByRole('checkbox', { name: 'api-review' }).click();
    await expect(box.getByRole('button', { name: 'Keep 2 skills' })).toBeVisible();
    await page.screenshot({ path: linkShot(info, `preview-several-chosen-${scheme}`) });
    await box.getByRole('button', { name: 'Keep 2 skills' }).click();
    const status = box.getByRole('status');
    await expect(status).toHaveText(
      'api-review is in ~/.agents/skills/api-review and linked in ~/.claude/skills/api-review. ' +
        'changelog is in ~/.agents/skills/changelog; ~/.claude/skills already has that name, so no link was made there. ' +
        'They are listed below with your other skills.',
    );
    await expect(status).toBeFocused();
    await expect(preview).toBeHidden();
    await fits();
    await page.screenshot({ path: linkShot(info, `kept-${scheme}`) });
  }
  const recorded = await calls(page);
  expect(recorded.lookups.at(-1)).toBe('https://github.com/octo/team-skills');
  expect(recorded.keeps.at(-1)).toEqual([
    '00000000-0000-4000-8000-0000000000c1',
    ['skills/api-review', 'skills/changelog'],
  ]);
  expect(recorded.adds).toEqual([]);
  expect(recorded.makes).toEqual([]);
  expect(errors).toEqual([]);
});

test('a shown score opens its grade’s details by keyboard, with every finding and Open full report', async ({
  page,
}, info) => {
  const errors = watchErrors(page);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto(harness('graded', '/scores/claude-md-doctor'));
  const trigger = page.getByRole('button', { name: '75, details for atlas' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'atlas · CLAUDE.md doctor' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Close' })).toBeFocused();
  await expect(dialog.getByText('Complete')).toBeVisible();
  const findings = dialog.locator('.xt-score-finding');
  await expect(findings).toHaveCount(5);
  await expect(findings.first()).toContainText('AGENTS.md is not loaded');
  await expect(findings.first()).toContainText('Fix Add @AGENTS.md to CLAUDE.md');
  await expect(findings.last()).toContainText('Commands run');
  await expect(dialog.locator('.xt-score-criterion')).toHaveCount(3);
  await expect(dialog.locator('.xt-score-criterion').first()).toContainText('15 / 30');
  // The dialog fits the window; only its middle scrolls, never sideways.
  const fits = await dialog.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const body = element.querySelector<HTMLElement>('.xt-score-details-body')!;
    return {
      inside: box.top >= 0 && box.bottom <= window.innerHeight && box.right <= window.innerWidth,
      sideways: body.scrollWidth - body.clientWidth,
    };
  });
  expect(fits).toEqual({ inside: true, sideways: 0 });
  await page.screenshot({ path: info.outputPath('grade-details-dark.png') });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.screenshot({ path: info.outputPath('grade-details-light.png') });
  await dialog.getByRole('button', { name: 'Open full report' }).click();
  await expect
    .poll(async () => (await calls(page)).reports)
    .toEqual(['00000000-0000-4000-8000-000000000001']);
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
  const recorded = await calls(page);
  expect(recorded.details).toEqual(['00000000-0000-4000-8000-000000000001']);
  expect(recorded.starts).toEqual([]);
  expect(errors).toEqual([]);
});

test('a grade without criteria, kept files or stored content says why, and opens no report', async ({
  page,
}, info) => {
  await page.goto(harness('graded', '/scores/claude-md-doctor'));
  await page.getByRole('button', { name: '61, details for cedar' }).click();
  const cedar = page.getByRole('dialog', { name: 'cedar · CLAUDE.md doctor' });
  await expect(cedar).toContainText(
    'Partial: the skill skipped a step, so this score may miss something',
  );
  await expect(cedar).toContainText(
    'Only the newest grades keep their full report, and this one’s was removed.',
  );
  await expect(cedar.getByRole('button', { name: 'Open full report' })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('grade-details-pruned.png') });
  await page.keyboard.press('Escape');
  await page.goto(
    harness('graded', '/scores/claude-md-doctor', undefined, undefined, '&details=content_deleted'),
  );
  await page.getByRole('button', { name: '75, details for atlas' }).click();
  const deleted = page.getByRole('dialog', { name: 'atlas · CLAUDE.md doctor' });
  await expect(deleted).toContainText(
    'Stored content was deleted. The score stays; its summary, findings, points and full report are gone.',
  );
  await expect(deleted.locator('.xt-score-finding')).toHaveCount(0);
  await expect(deleted.getByRole('button', { name: 'Open full report' })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('grade-details-deleted.png') });
  expect((await calls(page)).reports).toEqual([]);
});
