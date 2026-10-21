import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { GradingControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { GradingOverview } from '../../data/generated/GradingOverview';
import type { GradingStart } from '../../data/generated/GradingStart';
import { events } from '../../data/ipc-names';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import { linkKeep, linkLookup } from './link.synthetic';
import { pickerScenario, type PickerScenario } from './picker.synthetic';
import {
  LINK_HELP,
  LINK_KEEP_NOTE,
  MAKE_RUBRIC_NOTE,
  linkKeepStateText,
  rubricProblemText,
} from './scores';
import { scenario, type Scenario } from './scores.synthetic';

const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  localStorage.clear();
});

/** When the synthetic running run started (`scores.synthetic.json`). */
const STARTED = 1791193200000;
const MINUTE = 60_000;
/**
 * This Mac's clock set to `ms`, and with `ticking` its once-a-second timer
 * too. Testing Library's `waitFor` polls with `setInterval`, so a test that
 * waits for something other than the page changing keeps the real one.
 */
const clockAt = (ms: number, ticking = false) =>
  vi.useFakeTimers({
    toFake: ticking ? ['Date', 'setInterval', 'clearInterval'] : ['Date'],
    now: ms,
  });

/**
 * The F1 fixture source with grading answered by `overview` (and the
 * installed skills by `picker`) and spies.
 */
function sourceWith(
  overview: Scenario | (() => Promise<GradingOverview>),
  start: GradingStart = { state: 'started', run_id: '00000000-0000-4000-8000-0000000000ff' },
  picker: PickerScenario = 'mixed',
) {
  const source = new FixtureDataSource(exported);
  const controls = {
    skills: vi.fn<GradingControls['skills']>(async () => ({ state: 'not_configured' })),
    overview: vi.fn<GradingControls['overview']>(
      typeof overview === 'function' ? overview : async () => scenario(overview),
    ),
    runs: vi.fn<GradingControls['runs']>(async () => ({ state: 'not_configured' })),
    start: vi.fn<GradingControls['start']>(async () => start),
    cancel: vi.fn<GradingControls['cancel']>(async () => true),
    installed: vi.fn<GradingControls['installed']>(async () => pickerScenario(picker)),
    addSkill: vi.fn<GradingControls['addSkill']>(async () => ({
      state: 'added',
      skill_id: 'readme-check',
    })),
    removeSkill: vi.fn<GradingControls['removeSkill']>(async () => ({ state: 'removed' })),
    makeRubric: vi.fn<GradingControls['makeRubric']>(async () => ({
      state: 'started',
      job_id: '00000000-0000-4000-8000-0000000000aa',
    })),
    linkLookup: vi.fn<GradingControls['linkLookup']>(async () => linkLookup('several')),
    linkCancel: vi.fn<GradingControls['linkCancel']>(async () => false),
    linkKeep: vi.fn<GradingControls['linkKeep']>(async () => linkKeep('kept_several')),
    linkDiscard: vi.fn<GradingControls['linkDiscard']>(async () => true),
  };
  Object.defineProperty(source, 'grading', { value: controls });
  return { source, controls };
}

const opened = () =>
  waitFor(() => expect(screen.queryByText('Connecting live updates…')).toBeNull());
async function mount(path: string, source: FixtureDataSource = new FixtureDataSource(exported)) {
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await opened();
}
const page = () => document.querySelector<HTMLElement>('.xt-scores')!;
const card = (name: string) =>
  within(page()).getByRole('link', { name }).closest<HTMLElement>('.xt-score-card')!;
const rows = () =>
  within(within(page()).getByRole('table'))
    .getAllByRole('row')
    .filter((row) => row.classList.contains('xt-data-row'));

it('names a skill page by what it shows while no skill is known', async () => {
  await mount('/scores/claude-md-doctor');
  expect(await screen.findByText('Scores are not available here')).toBeTruthy();
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Skill scores');
  expect(
    within(screen.getByRole('navigation', { name: 'Breadcrumb' })).getByText('claude-md-doctor'),
  ).toBeTruthy();
});

it('says fixture mode has no scores, under its own menu item between Sessions and Pull requests', async () => {
  await mount('/scores');
  expect(await screen.findByText('Scores are not available here')).toBeTruthy();
  expect(screen.getByRole('heading', { level: 1, name: 'Scores' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: /Your projects/ })).toBeNull();
  const nav = screen.getByRole('navigation', { name: 'Main navigation' });
  const labels = within(nav)
    .getAllByRole('button')
    .map((button) => button.textContent);
  expect(labels.slice(0, 4)).toEqual(['Dashboard', 'Sessions', 'Scores', 'Pull requests']);
  expect(screen.getByRole('button', { name: 'Scores' }).getAttribute('aria-current')).toBe('page');
  expect(
    within(screen.getByRole('navigation', { name: 'Breadcrumb' })).getByText('scores'),
  ).toBeTruthy();
});

it('shows each skill’s average, graded count and top findings exactly as answered', async () => {
  const { source } = sourceWith('graded');
  await mount('/scores', source);
  const doctor = await waitFor(() => card('CLAUDE.md doctor'));
  expect(doctor.querySelector('.xt-score-big')?.textContent).toBe('75');
  expect(within(doctor).getByText('3 of 4 projects graded')).toBeTruthy();
  const findings = within(within(doctor).getByRole('list', { name: /Top findings/ })).getAllByRole(
    'listitem',
  );
  expect(findings.map((item) => item.textContent)).toEqual([
    'AGENTS.md is not loaded1 project',
    'CLAUDE.md is longer than 200 lines3 projects',
    'Two rules disagree1 project',
  ]);
  expect(within(findings[0]!).getByRole('img', { name: 'Critical' })).toBeTruthy();
  expect(within(findings[1]!).getByRole('img', { name: 'Warning' })).toBeTruthy();
  // The card opens the skill's page.
  fireEvent.click(within(doctor).getByRole('link', { name: 'CLAUDE.md doctor' }));
  expect(await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Scores' }).getAttribute('aria-current')).toBe('page');
});

it('shows a dash and “Not graded yet” before anything is graded, and invites the first grade', async () => {
  const { source } = sourceWith('empty');
  await mount('/scores', source);
  const doctor = await waitFor(() => card('CLAUDE.md doctor'));
  expect(doctor.querySelector('.xt-score-big')?.textContent).toBe('—');
  expect(within(doctor).getByText('Not graded yet')).toBeTruthy();
  expect(within(doctor).queryByRole('list')).toBeNull();
  fireEvent.click(within(doctor).getByRole('link', { name: 'CLAUDE.md doctor' }));
  expect(await screen.findByRole('heading', { name: 'Grade your first project' })).toBeTruthy();
  expect(
    screen.getByText(
      'Each grading runs Claude Code on your Claude account and takes a few minutes.',
    ),
  ).toBeTruthy();
  expect(rows().map((row) => within(row).getByRole('button').textContent)).toEqual([
    'Grade',
    'Grade',
    'Grade',
    'Grade',
  ]);
  expect(rows()[0]!.textContent).toContain('not graded');
});

it('lists every project and the apps it was found in, with Close focused first', async () => {
  const { source, controls } = sourceWith('graded');
  await mount('/scores', source);
  const trigger = await screen.findByRole('button', { name: 'Your projects · 4' });
  const before = controls.overview.mock.calls.length;
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Your projects' });
  await waitFor(() =>
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Close' })),
  );
  const items = within(within(dialog).getByRole('list', { name: 'Projects' })).getAllByRole(
    'listitem',
  );
  expect(items.map((item) => item.textContent)).toEqual([
    'atlas/work/atlasClaude Code · Claude app',
    'beacon/work/beaconCodex app · Codex',
    'cedar/work/cedarCursor',
    'delta/work/deltaClaude Code · Codex · Cursor',
  ]);
  // Opening it reads the project list again; it grades nothing.
  await waitFor(() => expect(controls.overview.mock.calls.length).toBeGreaterThan(before));
  expect(controls.start).not.toHaveBeenCalled();
  expect(within(dialog).queryByRole('button', { name: /Grade/ })).toBeNull();
});

/**
 * The Add a skill dialog's status line for its installed skills ("Add from a
 * link" has its own).
 */
const listStatus = (dialog: HTMLElement) =>
  within(dialog)
    .getAllByRole('status')
    .find((status) => !status.closest('.xt-score-link'))!;
/** The Add a skill dialog's group titled `title`, by its heading. */
const group = (dialog: HTMLElement, title: string) =>
  within(dialog)
    .getByRole('heading', { level: 3, name: new RegExp(`^${title}`) })
    .closest<HTMLElement>('section')!;
const rowNames = (section: HTMLElement) =>
  [...section.querySelectorAll('.xt-score-add-name')].map((name) => name.textContent);

/** The button that folds or unfolds the group titled `title`. */
const fold = (dialog: HTMLElement, title: string) =>
  within(group(dialog, title)).getByRole('button', { name: new RegExp(`^${title}`) });

it('ends the skill cards with Add a skill: three steps, Ready to add, Your skills, then two folded groups', async () => {
  const { source, controls } = sourceWith('graded');
  await mount('/scores', source);
  await waitFor(() => card('CLAUDE.md doctor'));
  const cards = within(within(page()).getByRole('list', { name: 'Grading skills' }))
    .getAllByRole('listitem')
    .filter((item) => item.parentElement?.classList.contains('xt-scores-grid'));
  const trigger = within(page()).getByRole('button', { name: 'Add a skill' });
  // It is the last card, after every skill's card.
  expect(cards.length).toBe(2);
  expect(cards.at(-1)!.contains(trigger)).toBe(true);
  expect(trigger.tabIndex).toBe(0);
  fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  const close = within(dialog).getByRole('button', { name: 'Close' });
  await waitFor(() => expect(document.activeElement).toBe(close));
  await waitFor(() => expect(within(dialog).getByRole('region', { name: 'Installed skills' })));
  // The three steps, in order.
  const steps = within(dialog).getByRole('list', { name: 'How a skill gets on Scores' });
  expect(
    within(steps)
      .getAllByRole('listitem')
      .map((step) => step.textContent),
  ).toEqual([
    '1Pick a skillOne you already use',
    '2Make a rubricXTrace writes it, about 1 minute',
    '3Add itIt gets its own score card',
  ]);
  // The groups in order, with the native answer's counts.
  expect(
    within(dialog)
      .getAllByRole('heading', { level: 3 })
      .map((heading) => heading.textContent),
  ).toEqual(['Add from a link', 'Ready to add3', 'Your skills5', '›Already on Scores2']);
  const ready = group(dialog, 'Ready to add');
  const yours = group(dialog, 'Your skills');
  expect(rowNames(ready)).toEqual(['Lint check', 'Lint check (my copy)', 'README check']);
  const row = (section: HTMLElement, name: string) =>
    [...section.querySelectorAll<HTMLElement>('.xt-score-add-row')].find(
      (item) => item.querySelector('.xt-score-add-name')?.textContent === name,
    )!;
  const add = within(row(ready, 'README check')).getByRole('button', { name: 'Add README check' });
  expect(add.getAttribute('data-variant')).toBe('primary');
  expect(row(ready, 'README check').querySelector('.xt-score-add-source')!.textContent).toBe(
    'Claude Code',
  );
  expect(row(ready, 'Lint check').textContent).toContain('Plugin code-tools');
  expect(row(ready, 'Lint check (my copy)').textContent).toContain(
    'Same id as Lint check; only one can be added',
  );
  expect(within(row(ready, 'Lint check (my copy)')).queryByRole('button')).toBeNull();
  // Your skills: one list of every skill a rubric can be made for, rubric
  // problems first, with the problem's reason; no repeated "no rubric" line.
  expect(rowNames(yours)).toEqual([
    'release-notes',
    'security-scan',
    'deploy',
    'frontend-design',
    'pdf',
  ]);
  expect(row(yours, 'release-notes').textContent).toContain('Its points do not add up to 100.');
  expect(within(yours).getByText(MAKE_RUBRIC_NOTE)).toBeTruthy();
  expect(MAKE_RUBRIC_NOTE).toContain('about a minute of your Claude usage');
  expect(dialog.textContent).not.toContain('No xtrace-rubric.md yet');
  expect(
    within(yours)
      .getAllByRole('button')
      .map((button) => button.getAttribute('aria-label')),
  ).toEqual([
    'Remake the rubric of release-notes',
    'Remake the rubric of security-scan',
    'Make a rubric for deploy',
    'Make a rubric for frontend-design',
    'Make a rubric for pdf',
  ]);
  expect(within(ready).queryByRole('button', { name: /rubric/ })).toBeNull();
  // Already on Scores is folded: its rows are not shown until it is opened.
  const onScores = fold(dialog, 'Already on Scores');
  expect(onScores.getAttribute('aria-expanded')).toBe('false');
  expect(rowNames(group(dialog, 'Already on Scores'))).toEqual([]);
  fireEvent.click(onScores);
  expect(onScores.getAttribute('aria-expanded')).toBe('true');
  const already = group(dialog, 'Already on Scores');
  expect(rowNames(already)).toEqual(['CLAUDE.md Doctor (copy)', 'claude-md-doctor']);
  expect(row(already, 'claude-md-doctor').textContent).toContain('On Scores (built in)');
  // Neither offers a rubric: Scores already has that skill built in.
  expect(
    [...already.querySelectorAll('.xt-score-add-row')].flatMap((item) => [
      ...item.querySelectorAll('button'),
    ]),
  ).toEqual([]);
  fireEvent.click(onScores);
  expect(rowNames(group(dialog, 'Already on Scores'))).toEqual([]);
  expect(dialog.textContent).toContain('1 skill folder could not be read');
  // Escape closes it and focus returns to the card.
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(controls.addSkill).not.toHaveBeenCalled();
  expect(controls.start).not.toHaveBeenCalled();
});

it('says how to get a skill ready while none is, and lists a home like a real one', async () => {
  const { source } = sourceWith('graded', undefined, 'yours');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  await within(dialog).findByRole('region', { name: 'Installed skills' });
  const ready = group(dialog, 'Ready to add');
  expect(within(ready).getByRole('heading').textContent).toBe('Ready to add0');
  expect(ready.textContent).toContain('Make a rubric for one of your skills below.');
  expect(ready.textContent).not.toContain('None');
  expect(rowNames(group(dialog, 'Your skills'))).toHaveLength(47);
  // Two refused skills: folded, with their count, closed at each opening.
  const cant = fold(dialog, 'Can’t be scored');
  expect(cant.textContent).toBe('›Can’t be scored2');
  expect(cant.getAttribute('aria-expanded')).toBe('false');
  expect(fold(dialog, 'Already on Scores').textContent).toBe('›Already on Scores1');
  fireEvent.click(cant);
  const refused = group(dialog, 'Can’t be scored');
  expect(rowNames(refused)).toEqual(['code-review-patterns', 'framer']);
  // Each says Claude's reason once, and can be tried again there.
  expect(refused.textContent).toContain('it needs project changes and the network');
  expect(refused.textContent).not.toContain('Can’t be scored:');
  expect(
    within(refused)
      .getAllByRole('button', { name: /^Try making a rubric again for / })
      .map((button) => button.textContent),
  ).toEqual(['Try again', 'Try again']);
  // Closing and opening the dialog folds it again.
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(screen.getByRole('button', { name: 'Add a skill' }));
  const again = await screen.findByRole('dialog', { name: 'Add a skill' });
  await within(again).findByRole('region', { name: 'Installed skills' });
  expect(fold(again, 'Can’t be scored').getAttribute('aria-expanded')).toBe('false');
});

it('searches folded groups too: a folded group shows how many of its skills match', async () => {
  const { source } = sourceWith('graded', undefined, 'yours');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  const search = await within(dialog).findByRole('searchbox', { name: 'Search installed skills' });
  fireEvent.change(search, { target: { value: 'framer' } });
  expect(fold(dialog, 'Can’t be scored').textContent).toBe('›Can’t be scored1 of 2');
  expect(fold(dialog, 'Already on Scores').textContent).toBe('›Already on Scores0 of 1');
  expect(within(group(dialog, 'Your skills')).getByRole('heading').textContent).toBe(
    'Your skills0 of 47',
  );
  expect(group(dialog, 'Your skills').textContent).toContain('None match');
  fireEvent.click(fold(dialog, 'Can’t be scored'));
  expect(rowNames(group(dialog, 'Can’t be scored'))).toEqual(['framer']);
  fireEvent.click(fold(dialog, 'Already on Scores'));
  expect(group(dialog, 'Already on Scores').textContent).toContain('None match');
  // A search matches the source label and the folder too.
  fireEvent.change(search, { target: { value: 'plugin work' } });
  expect(within(group(dialog, 'Your skills')).getByRole('heading').textContent).toBe(
    'Your skills12 of 47',
  );
  // Escape from the search box closes the dialog.
  fireEvent.keyDown(search, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
});

it('filters the installed skills by the search text and says how many match', async () => {
  const { source } = sourceWith('graded');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  const search = await within(dialog).findByRole('searchbox', { name: 'Search installed skills' });
  fireEvent.change(search, { target: { value: 'LINT' } });
  expect(within(group(dialog, 'Ready to add')).getByRole('heading').textContent).toBe(
    'Ready to add2 of 3',
  );
  expect(rowNames(group(dialog, 'Ready to add'))).toEqual(['Lint check', 'Lint check (my copy)']);
  expect(group(dialog, 'Your skills').textContent).toContain('None match');
  // A plugin's name matches too.
  fireEvent.change(search, { target: { value: 'plugin documents' } });
  expect(rowNames(group(dialog, 'Your skills'))).toEqual(['pdf']);
});

it('adds a ready skill by its location only, stays open and shows it on Scores', async () => {
  const { source, controls } = sourceWith('graded');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  controls.installed.mockResolvedValue(pickerScenario('added'));
  controls.overview.mockResolvedValue(scenario('added'));
  fireEvent.click(await within(dialog).findByRole('button', { name: 'Add README check' }));
  await waitFor(() =>
    expect(controls.addSkill).toHaveBeenCalledWith('/home/you/.claude/skills/readme-check'),
  );
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe('README check is on Scores now.'),
  );
  // The dialog stays open; the row moves to Already on Scores, which says so,
  // and the card is behind it.
  expect(screen.getByRole('dialog', { name: 'Add a skill' })).toBe(dialog);
  await waitFor(() =>
    expect(within(dialog).queryByRole('button', { name: 'Add README check' })).toBeNull(),
  );
  expect(rowNames(group(dialog, 'Ready to add'))).toEqual(['Lint check', 'Lint check (my copy)']);
  fireEvent.click(fold(dialog, 'Already on Scores'));
  expect(within(group(dialog, 'Already on Scores')).getByText('On Scores')).toBeTruthy();
  await waitFor(() =>
    expect(document.querySelector('.xt-scores-grid')!.textContent).toContain('README check'),
  );
  // An add that did not happen says why. A plugin's skill is sent by its
  // plugin and name, never its versioned folder.
  controls.addSkill.mockResolvedValueOnce({ state: 'id_clash' });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Add Lint check' }));
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe(
      'Another installed skill has the same id as Lint check; only one can be added.',
    ),
  );
  expect(controls.addSkill).toHaveBeenLastCalledWith('plugin:code-tools@official/lint');
  expect(controls.start).not.toHaveBeenCalled();
});

it('keeps Add busy until the list read after it shows the skill on Scores', async () => {
  const { source, controls } = sourceWith('graded');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  const button = await within(dialog).findByRole('button', { name: 'Add README check' });
  // The read after the answer is held until the test lets it finish.
  let finish: () => void = () => {};
  controls.installed.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = () => resolve(pickerScenario('added'));
      }),
  );
  // Every time the row renders, it never shows a pressable Add again.
  const seen: string[] = [];
  const observer = new MutationObserver(() => {
    const now = within(dialog).queryByRole('button', { name: 'Add README check' });
    if (now) seen.push(`${now.textContent}:${now.getAttribute('aria-disabled')}`);
  });
  observer.observe(dialog, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
  });
  fireEvent.click(button);
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe('README check is on Scores now.'),
  );
  await waitFor(() => expect(controls.installed.mock.calls.length).toBeGreaterThan(1));
  const busy = within(dialog).getByRole('button', { name: 'Add README check' });
  expect(busy.textContent).toBe('Adding…');
  expect(busy.getAttribute('aria-disabled')).toBe('true');
  // A second press meanwhile adds nothing more.
  fireEvent.click(busy);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Add Lint check' }));
  expect(controls.addSkill).toHaveBeenCalledTimes(1);
  act(() => finish());
  await waitFor(() =>
    expect(within(dialog).queryByRole('button', { name: 'Add README check' })).toBeNull(),
  );
  observer.disconnect();
  expect(fold(dialog, 'Already on Scores').textContent).toBe('›Already on Scores3');
  expect(seen.filter((state) => state !== 'Adding…:true')).toEqual([]);
});

it('clears Add once a read after a refused add shows the row unchanged', async () => {
  const { source, controls } = sourceWith('graded');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  controls.addSkill.mockResolvedValueOnce({ state: 'failed' });
  fireEvent.click(await within(dialog).findByRole('button', { name: 'Add README check' }));
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe('README check could not be added. Try again.'),
  );
  await waitFor(() =>
    expect(within(dialog).getByRole('button', { name: 'Add README check' }).textContent).toBe(
      'Add',
    ),
  );
});

it('starts each opening of Add a skill with an empty search', async () => {
  const { source } = sourceWith('graded');
  await mount('/scores', source);
  const trigger = await screen.findByRole('button', { name: 'Add a skill' });
  fireEvent.click(trigger);
  let dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  fireEvent.change(
    await within(dialog).findByRole('searchbox', { name: 'Search installed skills' }),
    { target: { value: 'lint' } },
  );
  expect(rowNames(group(dialog, 'Ready to add'))).toHaveLength(2);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(trigger);
  dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  const search = await within(dialog).findByRole<HTMLInputElement>('searchbox', {
    name: 'Search installed skills',
  });
  expect(search.value).toBe('');
  expect(rowNames(group(dialog, 'Ready to add'))).toHaveLength(3);
});

it('says why plugin skills are not all listed when the plugin list cannot be used', async () => {
  const { source } = sourceWith('graded', undefined, 'plugins_unread');
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  expect(
    await within(dialog).findByText(
      /Plugin skills are not all listed: Claude Code’s installed_plugins\.json is a link\./,
    ),
  ).toBeTruthy();
  expect(dialog.textContent).not.toContain('Some folders hold more than XTrace reads');
  expect(dialog.textContent).not.toContain('Plugin code-tools');
});

it('names no limit number in a rubric problem', () => {
  expect(rubricProblemText.too_large).toBe('it is larger than XTrace reads');
  expect(rubricProblemText.too_many_criteria).toBe('it has more criteria than XTrace allows');
  for (const text of Object.values(rubricProblemText)) {
    expect(text).not.toMatch(/KiB|\b20\b|64/);
  }
});

it('says when installed skills are not read here or could not be read', async () => {
  for (const [picker, text] of [
    ['not_configured', 'This copy of XTrace runs on sample data, so it reads no installed skills.'],
    ['failed', 'Installed skills could not be read.'],
    ['empty', 'No installed skills were found for Claude Code, Codex or shared agents.'],
  ] as const) {
    const { source } = sourceWith('graded', undefined, picker);
    await mount('/scores', source);
    fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
    const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
    expect(await within(dialog).findByText(text)).toBeTruthy();
    cleanup();
  }
});

it('shows an added skill as its own card; one that cannot grade says why once and Grade asks native', async () => {
  const { source, controls } = sourceWith('added_problem', { state: 'skill_unavailable' });
  await mount('/scores', source);
  const readme = await waitFor(() => card('README check'));
  expect(readme.querySelector('.xt-score-big')?.textContent).toBe('70');
  expect(readme.textContent).toContain(
    'Can’t grade now. Its xtrace-rubric.md is no longer valid: its points do not add up to 100.',
  );
  expect(card('CLAUDE.md doctor').querySelector('.xt-score-card-problem')).toBeNull();
  fireEvent.click(within(readme).getByRole('link', { name: 'README check' }));
  await screen.findByRole('heading', { level: 1, name: 'README check' });
  expect(screen.getByText('This skill can’t grade right now')).toBeTruthy();
  expect(page().textContent).toContain('Added from /home/you/.claude/skills/readme-check');
  const reason = 'its points do not add up to 100';
  const times = () => page().textContent!.split(reason).length - 1;
  expect(times()).toBe(1);
  // Grade asks native, which says whether the skill can grade now.
  fireEvent.click(screen.getByRole('button', { name: 'Grade again atlas' }));
  await waitFor(() => expect(controls.start).toHaveBeenCalledWith('/work/atlas', 'readme-check'));
  await waitFor(() =>
    expect(page().querySelector('.xt-scores-message')!.textContent).toBe(
      'This skill can’t grade right now, so nothing was graded.',
    ),
  );
  // The reason is still shown once, above.
  expect(times()).toBe(1);
  // Fixed meanwhile: the same press starts a grading.
  controls.start.mockResolvedValueOnce({
    state: 'started',
    run_id: '00000000-0000-4000-8000-0000000000fe',
  });
  fireEvent.click(screen.getByRole('button', { name: 'Grade again atlas' }));
  await waitFor(() => expect(controls.start).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(page().querySelector('.xt-scores-message')!.textContent).toBe(''));
});

it('shows where a plugin skill is opened from now, or only its plugin when it is gone', async () => {
  const withOrigin = (folder: string | null) => async () => {
    const answer = scenario('added_problem');
    if (answer.state === 'loaded') {
      answer.skills[1].skill.origin = { kind: 'added', folder, plugin: 'code-tools' };
      answer.skills[1].skill.problem = folder === null ? { reason: 'plugin_unavailable' } : null;
    }
    return answer;
  };
  const current = '/home/you/.claude/plugins/cache/official/code-tools/1.3.0/skills/lint';
  let { source } = sourceWith(withOrigin(current));
  await mount('/scores/readme-check', source);
  await screen.findByRole('heading', { level: 1, name: 'README check' });
  expect(page().textContent).toContain(`Added from plugin code-tools · ${current}`);
  expect(screen.queryByText('This skill can’t grade right now')).toBeNull();
  cleanup();
  ({ source } = sourceWith(withOrigin(null)));
  await mount('/scores/readme-check', source);
  await screen.findByRole('heading', { level: 1, name: 'README check' });
  expect(page().textContent).toContain('Added from plugin code-tools');
  expect(page().textContent).not.toContain('Added from plugin code-tools ·');
  expect(page().textContent).toContain(
    'Its Claude Code plugin is no longer installed and turned on. Its scores stay.',
  );
});

it('removes an added skill from Scores by its id and goes back; a built-in one has no Remove', async () => {
  const { source, controls } = sourceWith('added');
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  expect(screen.queryByRole('button', { name: 'Remove from Scores' })).toBeNull();
  cleanup();
  await mount('/scores/readme-check', source);
  await screen.findByRole('heading', { level: 1, name: 'README check' });
  controls.removeSkill.mockResolvedValueOnce({ state: 'failed' });
  fireEvent.click(screen.getByRole('button', { name: 'Remove from Scores' }));
  await waitFor(() =>
    expect(page().querySelector('.xt-scores-message')!.textContent).toBe(
      'The skill could not be removed. Try again.',
    ),
  );
  // While it grades, native refuses and the page says so.
  controls.removeSkill.mockResolvedValueOnce({ state: 'running' });
  fireEvent.click(screen.getByRole('button', { name: 'Remove from Scores' }));
  await waitFor(() =>
    expect(page().querySelector('.xt-scores-message')!.textContent).toBe(
      'This skill is grading a project now. Wait for it to finish or cancel it, then remove it.',
    ),
  );
  expect(screen.getByRole('heading', { level: 1, name: 'README check' })).toBeTruthy();
  controls.overview.mockResolvedValue(scenario('graded'));
  fireEvent.click(screen.getByRole('button', { name: 'Remove from Scores' }));
  await screen.findByRole('heading', { level: 1, name: 'Scores' });
  expect(controls.removeSkill).toHaveBeenLastCalledWith('readme-check');
  await waitFor(() => expect(document.querySelectorAll('.xt-score-card').length).toBe(1));
});

it('shows Add a skill before anything is graded, but not on a skill page or in fixture mode', async () => {
  const { source } = sourceWith('empty');
  await mount('/scores', source);
  await waitFor(() => card('CLAUDE.md doctor'));
  expect(within(page()).getByRole('button', { name: 'Add a skill' })).toBeTruthy();
  fireEvent.click(within(page()).getByRole('link', { name: 'CLAUDE.md doctor' }));
  expect(await screen.findByRole('heading', { name: 'Grade your first project' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Add a skill' })).toBeNull();
  cleanup();
  await mount('/scores');
  expect(await screen.findByText('Scores are not available here')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Add a skill' })).toBeNull();
});

it('lists each project’s score, when it was graded and its state', async () => {
  const { source } = sourceWith('failed');
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  const [atlas, beacon, cedar, delta] = rows();
  expect(atlas!.textContent).toContain('75');
  expect(atlas!.textContent).toContain('Last try failed: it ran past its time limit');
  expect(within(atlas!).getByRole('button').textContent).toBe('Grade again');
  expect(beacon!.textContent).toContain('90');
  expect(cedar!.textContent).toContain('61');
  expect(cedar!.textContent).toContain('Partial result');
  expect(delta!.textContent).toContain('not graded');
  expect(delta!.textContent).toContain('Last try stopped: XTrace closed during the grading');
  expect(within(delta!).getByRole('button').textContent).toBe('Grade');
  expect(page().querySelector('.xt-scores-average .xt-score-big')?.textContent).toBe('75');
});

it('starts a grading with only the project path and skill ID, and says why one did not start', async () => {
  const { source, controls } = sourceWith('empty');
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { name: 'Grade your first project' });
  fireEvent.click(screen.getByRole('button', { name: 'Grade atlas' }));
  await waitFor(() =>
    expect(controls.start).toHaveBeenCalledWith('/work/atlas', 'claude-md-doctor'),
  );
  expect(within(page()).getByRole('status').textContent).toBe('');
  for (const [answer, text] of [
    [
      { state: 'claude_unavailable' },
      'Claude Code was not found on this Mac, so nothing was graded.',
    ],
    [{ state: 'not_a_project' }, 'beacon is no longer in your project list, so it was not graded.'],
    [
      { state: 'busy' },
      'Only one grading runs at a time. Another grading is going now; wait for it to finish or cancel it.',
    ],
  ] as const) {
    controls.start.mockResolvedValueOnce(answer);
    fireEvent.click(screen.getByRole('button', { name: 'Grade beacon' }));
    await waitFor(() => expect(within(page()).getByRole('status').textContent).toBe(text));
  }
  controls.start.mockRejectedValueOnce(new Error('ipc'));
  fireEvent.click(screen.getByRole('button', { name: 'Grade beacon' }));
  await waitFor(() =>
    expect(within(page()).getByRole('status').textContent).toBe(
      'The grading could not be started. Try again.',
    ),
  );
});

it('while one grading runs, another Grade explains instead of starting, and Cancel stops it', async () => {
  clockAt(STARTED + 4 * MINUTE + 10_000);
  const { source, controls } = sourceWith('running');
  await mount('/scores', source);
  const doctor = await waitFor(() => card('CLAUDE.md doctor'));
  expect(within(doctor).getByText('Grading · 0 h 4 m')).toBeTruthy();
  // The row shows the run, so no notice repeats it.
  expect(screen.queryByTestId('running-elsewhere')).toBeNull();
  fireEvent.click(within(doctor).getByRole('link', { name: 'CLAUDE.md doctor' }));
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  expect(screen.queryByTestId('running-elsewhere')).toBeNull();
  const delta = rows()[3]!;
  expect(delta.textContent).toContain('Grading now · 0 h 4 m');
  // Grade stays pressable and says why it cannot start; nothing is sent.
  const again = screen.getByRole('button', { name: 'Grade again atlas' });
  expect(again.hasAttribute('disabled')).toBe(false);
  fireEvent.click(again);
  expect(within(page()).getByRole('status').textContent).toBe(
    'Only one grading runs at a time. delta is being graded with CLAUDE.md doctor now; wait for it to finish or cancel it.',
  );
  expect(controls.start).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel grading delta' }));
  await waitFor(() =>
    expect(controls.cancel).toHaveBeenCalledWith('00000000-0000-4000-8000-000000000004'),
  );
  await waitFor(() =>
    expect(within(page()).getByRole('status').textContent).toBe('Stopping the grading…'),
  );
  controls.cancel.mockResolvedValueOnce(false);
  fireEvent.click(screen.getByRole('button', { name: 'Cancel grading delta' }));
  await waitFor(() =>
    expect(within(page()).getByRole('status').textContent).toBe(
      'That grading had already finished.',
    ),
  );
});

it('shows the elapsed time progress reports, and reads again for a run it does not show', async () => {
  // A report ahead of this Mac's clock is never shown as less.
  clockAt(STARTED + MINUTE);
  const { source, controls } = sourceWith('running');
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  const runId = '00000000-0000-4000-8000-000000000004';
  act(() =>
    source.emit(events.gradingProgress, { run_id: runId, state: 'running', elapsed_seconds: 185 }),
  );
  await waitFor(() => expect(rows()[3]!.textContent).toContain('Grading now · 0 h 3 m'));
  const before = controls.overview.mock.calls.length;
  // A payload that is not a progress report changes nothing.
  act(() => source.emit(events.gradingProgress, { run_id: runId, state: 'odd' }));
  act(() =>
    source.emit(events.gradingProgress, {
      run_id: 'another',
      state: 'running',
      elapsed_seconds: 1,
    }),
  );
  await waitFor(() => expect(controls.overview.mock.calls.length).toBeGreaterThan(before));
  // Progress never makes a row live: the stored status in the answer does.
  expect(rows()[0]!.textContent).not.toContain('Grading now');
});

it('says when scores could not be read and reads again on request', async () => {
  const { source, controls } = sourceWith('failed_read');
  await mount('/scores', source);
  expect(await screen.findByText('Scores could not be read')).toBeTruthy();
  controls.overview.mockResolvedValue(scenario('graded'));
  fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await waitFor(() => card('CLAUDE.md doctor'));
});

it('says an address that names no skill on Scores names none', async () => {
  const { source } = sourceWith('graded');
  await mount('/scores/not-a-skill', source);
  expect(await screen.findByText('This skill is not on Scores')).toBeTruthy();
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Skill scores');
  expect(screen.getByRole('link', { name: 'Back to Scores' })).toBeTruthy();
});

it('says when no projects were found', async () => {
  const { source } = sourceWith('no_projects');
  await mount('/scores/claude-md-doctor', source);
  expect(
    await screen.findByText('No projects found in Claude Code, the Claude app, Codex or Cursor.'),
  ).toBeTruthy();
  expect(screen.getByRole('heading', { name: 'Projects' })).toBeTruthy();
});

it('moves a going run’s elapsed time on from its start between progress reports', async () => {
  clockAt(STARTED + MINUTE, true);
  const { source } = sourceWith('running');
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  await waitFor(() => expect(rows()[3]!.textContent).toContain('Grading now · 0 h 1 m'));
  act(() => vi.advanceTimersByTime(3 * MINUTE));
  expect(rows()[3]!.textContent).toContain('Grading now · 0 h 4 m');
  // Only the going row moves: the stored status says which one that is.
  expect(rows()[0]!.textContent).not.toContain('Grading now');
});

it('offers Cancel for a going run whose project is no longer listed, on both pages', async () => {
  clockAt(STARTED + 4 * MINUTE);
  const { source, controls } = sourceWith('running_unlisted');
  await mount('/scores', source);
  const notice = await screen.findByTestId('running-elsewhere');
  expect(notice.textContent).toBe(
    '/work/old-atlas is being graded with CLAUDE.md doctor · 0 h 4 mThis project is no longer in your project list, so no row shows it.Cancel',
  );
  fireEvent.click(within(notice).getByRole('button', { name: 'Cancel grading /work/old-atlas' }));
  await waitFor(() =>
    expect(controls.cancel).toHaveBeenCalledWith('00000000-0000-4000-8000-000000000004'),
  );
  await waitFor(() =>
    expect(within(page()).getByRole('status').textContent).toBe('Stopping the grading…'),
  );
  fireEvent.click(within(card('CLAUDE.md doctor')).getByRole('link', { name: 'CLAUDE.md doctor' }));
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  // No row shows the run, so the notice above the table does, with Cancel.
  expect(rows().some((row) => row.textContent?.includes('Grading now'))).toBe(false);
  const above = screen.getByTestId('running-elsewhere');
  fireEvent.click(screen.getByRole('button', { name: 'Grade again atlas' }));
  expect(within(page()).getByRole('status').textContent).toBe(
    'Only one grading runs at a time. /work/old-atlas is being graded with CLAUDE.md doctor now; wait for it to finish or cancel it.',
  );
  expect(controls.start).not.toHaveBeenCalled();
  fireEvent.click(within(above).getByRole('button', { name: 'Cancel grading /work/old-atlas' }));
  await waitFor(() => expect(controls.cancel).toHaveBeenCalledTimes(2));
});

it('explains a Grade press while another press’s start is still being answered', async () => {
  let answer: (value: GradingStart) => void = () => {};
  const { source, controls } = sourceWith('empty');
  controls.start.mockImplementationOnce(
    () => new Promise<GradingStart>((resolve) => (answer = resolve)),
  );
  await mount('/scores/claude-md-doctor', source);
  await screen.findByRole('heading', { name: 'Grade your first project' });
  fireEvent.click(screen.getByRole('button', { name: 'Grade atlas' }));
  fireEvent.click(screen.getByRole('button', { name: 'Grade beacon' }));
  expect(within(page()).getByRole('status').textContent).toBe('Starting a grading…');
  expect(controls.start).toHaveBeenCalledTimes(1);
  expect(controls.start).toHaveBeenCalledWith('/work/atlas', 'claude-md-doctor');
  await act(async () =>
    answer({ state: 'started', run_id: '00000000-0000-4000-8000-0000000000ff' }),
  );
  await waitFor(() => expect(within(page()).getByRole('status').textContent).toBe(''));
  expect(controls.start).toHaveBeenCalledTimes(1);
});

it('keeps Your projects open on the last list, with focus inside, when its re-read fails', async () => {
  for (const failure of ['answer', 'request'] as const) {
    const { source, controls } = sourceWith('graded');
    await mount('/scores', source);
    const trigger = await screen.findByRole('button', { name: 'Your projects · 4' });
    if (failure === 'answer') controls.overview.mockResolvedValue(scenario('failed_read'));
    else controls.overview.mockRejectedValue(new Error('ipc'));
    const before = controls.overview.mock.calls.length;
    fireEvent.click(trigger);
    const dialog = await screen.findByRole('dialog', { name: 'Your projects' });
    await waitFor(() => expect(controls.overview.mock.calls.length).toBeGreaterThan(before));
    expect((await within(dialog).findByRole('alert')).textContent).toBe(
      'The project list could not be read again. This is the list read last.',
    );
    expect(screen.getByRole('dialog', { name: 'Your projects' })).toBe(dialog);
    expect(within(dialog).getAllByRole('listitem')).toHaveLength(4);
    await waitFor(() =>
      expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Close' })),
    );
    cleanup();
  }
});

/** The synthetic rubric job's start (`picker.synthetic.json`). */
const MAKING_STARTED = 1784000000000;

const installedRow = (dialog: HTMLElement, name: string) =>
  [...dialog.querySelectorAll<HTMLElement>('.xt-score-add-row')].find(
    (item) => item.querySelector('.xt-score-add-name')?.textContent === name,
  )!;

async function openDialog(source: FixtureDataSource) {
  await mount('/scores', source);
  fireEvent.click(await screen.findByRole('button', { name: 'Add a skill' }));
  const dialog = await screen.findByRole('dialog', { name: 'Add a skill' });
  await within(dialog).findByRole('region', { name: 'Installed skills' });
  return dialog;
}

it('makes a rubric by the location only and shows the job going, with Cancel by its job ID', async () => {
  clockAt(MAKING_STARTED + 70_000);
  const { source, controls } = sourceWith('graded');
  const dialog = await openDialog(source);
  controls.installed.mockResolvedValue(pickerScenario('making'));
  controls.overview.mockResolvedValue(scenario('making_rubric'));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Make a rubric for deploy' }));
  await waitFor(() =>
    expect(controls.makeRubric).toHaveBeenCalledWith('/home/you/.claude/skills/deploy'),
  );
  const deploy = () => installedRow(dialog, 'deploy');
  await waitFor(() => expect(deploy().textContent).toContain('Making a rubric · 0 h 1 m'));
  expect(deploy().getAttribute('aria-busy')).toBe('true');
  expect(deploy().closest('section')!.getAttribute('data-group')).toBe('yours');
  // A plugin's skill is sent by its plugin and name.
  fireEvent.click(within(dialog).getByRole('button', { name: 'Make a rubric for pdf' }));
  expect(listStatus(dialog).textContent).toBe(
    'Only one grading runs at a time, and making a rubric counts as one. A rubric for deploy is being made now; wait for it to finish or cancel it.',
  );
  expect(controls.makeRubric).toHaveBeenCalledTimes(1);
  // Cancel sends the job's ID only, once the read after the Make press is shown.
  controls.installed.mockResolvedValue(pickerScenario('make_failed'));
  controls.overview.mockResolvedValue(scenario('graded'));
  const cancel = within(deploy()).getByRole('button', {
    name: 'Cancel making a rubric for deploy',
  });
  await waitFor(() => expect(cancel.getAttribute('aria-disabled')).toBeNull());
  fireEvent.click(cancel);
  await waitFor(() =>
    expect(controls.cancel).toHaveBeenCalledWith('00000000-0000-4000-8000-0000000000aa'),
  );
  expect(controls.start).not.toHaveBeenCalled();
  expect(controls.addSkill).not.toHaveBeenCalled();
});

it('says why no rubric was made and offers Try again, and says why a press started nothing', async () => {
  const { source, controls } = sourceWith('graded', undefined, 'make_failed');
  const dialog = await openDialog(source);
  const deploy = installedRow(dialog, 'deploy');
  expect(deploy.textContent).toContain(
    `No rubric was made: the rubric Claude wrote was not valid on a second try either: ${rubricProblemText.total}.`,
  );
  expect(installedRow(dialog, 'release-notes').textContent).toContain(
    'No rubric was made: it ran past its time limit.',
  );
  expect(installedRow(dialog, 'frontend-design').textContent).toContain(
    'Making a rubric was cancelled.',
  );
  // A skill Claude judged unfit for scoring is under Can't be scored, folded:
  // opened, it says why and can be tried again.
  expect(installedRow(dialog, 'pdf')).toBeUndefined();
  fireEvent.click(fold(dialog, 'Can’t be scored'));
  const pdf = installedRow(group(dialog, 'Can’t be scored'), 'pdf');
  expect(pdf.textContent).toContain(
    'This skill reads, fills and merges PDF files; it does not review a project, so its output cannot be scored.',
  );
  expect(
    within(pdf).getByRole('button', { name: 'Try making a rubric again for pdf' }).textContent,
  ).toBe('Try again');
  controls.makeRubric.mockResolvedValueOnce({ state: 'claude_unavailable' });
  fireEvent.click(
    within(deploy).getByRole('button', { name: 'Try making a rubric again for deploy' }),
  );
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe(
      'Claude Code was not found on this Mac, so no rubric was made.',
    ),
  );
  expect(controls.makeRubric).toHaveBeenCalledWith('/home/you/.claude/skills/deploy');
  // A busy answer explains one at a time.
  controls.makeRubric.mockResolvedValueOnce({ state: 'busy' });
  // Make stays busy until the list read after its answer.
  await waitFor(() =>
    expect(
      within(installedRow(dialog, 'deploy'))
        .getByRole('button', { name: 'Try making a rubric again for deploy' })
        .getAttribute('aria-disabled'),
    ).toBeNull(),
  );
  fireEvent.click(
    within(installedRow(dialog, 'release-notes')).getByRole('button', {
      name: 'Try making a rubric again for release-notes',
    }),
  );
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toContain(
      'Only one grading runs at a time, and making a rubric counts as one.',
    ),
  );
});

it('moves a made skill to Ready to add with its criteria, then Add works as for any ready skill', async () => {
  const { source, controls } = sourceWith('graded', undefined, 'made');
  const dialog = await openDialog(source);
  const ready = group(dialog, 'Ready to add');
  const deploy = installedRow(ready, 'Deploy');
  expect(deploy.textContent).toContain(
    'Rubric made: Build passes 40 · Staging deploy succeeds 40 · Rollback documented 20',
  );
  expect(installedRow(ready, 'Frontend design').textContent).toContain(
    'Rubric made and kept by XTrace: Distinctive design 100',
  );
  fireEvent.click(within(deploy).getByRole('button', { name: 'Add Deploy' }));
  await waitFor(() =>
    expect(controls.addSkill).toHaveBeenCalledWith('/home/you/.claude/skills/deploy'),
  );
});

it('a grading going now makes Make a rubric explain instead of starting', async () => {
  const { source, controls } = sourceWith('running');
  const dialog = await openDialog(source);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Make a rubric for deploy' }));
  await waitFor(() =>
    expect(listStatus(dialog).textContent).toBe(
      'Only one grading runs at a time. delta is being graded with CLAUDE.md doctor now; wait for it to finish or cancel it.',
    ),
  );
  expect(controls.makeRubric).not.toHaveBeenCalled();
});

it('names a rubric being made on Scores with its own Cancel, and Grade explains it', async () => {
  clockAt(MAKING_STARTED + 2 * MINUTE + 5_000);
  const { source, controls } = sourceWith('making_rubric');
  await mount('/scores', source);
  const notice = await screen.findByTestId('making-rubric');
  expect(notice.textContent).toContain('A rubric for deploy is being made · 0 h 2 m');
  fireEvent.click(
    within(notice).getByRole('button', { name: 'Cancel making a rubric for deploy' }),
  );
  await waitFor(() =>
    expect(controls.cancel).toHaveBeenCalledWith('00000000-0000-4000-8000-0000000000aa'),
  );
  fireEvent.click(within(card('CLAUDE.md doctor')).getByRole('link', { name: 'CLAUDE.md doctor' }));
  await screen.findByRole('heading', { level: 1, name: 'CLAUDE.md doctor' });
  expect(screen.getByTestId('making-rubric')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Grade again atlas' }));
  expect(within(page()).getByRole('status').textContent).toBe(
    'Only one grading runs at a time, and making a rubric counts as one. A rubric for deploy is being made now; wait for it to finish or cancel it.',
  );
  expect(controls.start).not.toHaveBeenCalled();
});

it('shows the rubric job’s reported elapsed time and reads again when it is over', async () => {
  clockAt(MAKING_STARTED);
  const { source, controls } = sourceWith('graded', undefined, 'making');
  const dialog = await openDialog(source);
  const jobId = '00000000-0000-4000-8000-0000000000aa';
  const location = '/home/you/.claude/skills/deploy';
  act(() =>
    source.emit(events.gradingRubricProgress, {
      job_id: jobId,
      location,
      state: 'running',
      elapsed_seconds: 95,
    }),
  );
  await waitFor(() =>
    expect(installedRow(dialog, 'deploy').textContent).toContain('Making a rubric · 0 h 2 m'),
  );
  const before = controls.installed.mock.calls.length;
  // Not a report: nothing changes.
  act(() => source.emit(events.gradingRubricProgress, { job_id: jobId, state: 'odd' }));
  controls.installed.mockResolvedValue(pickerScenario('made'));
  act(() =>
    source.emit(events.gradingRubricProgress, {
      job_id: jobId,
      location,
      state: 'finished',
      elapsed_seconds: 100,
    }),
  );
  await waitFor(() => expect(controls.installed.mock.calls.length).toBeGreaterThan(before));
  await waitFor(() =>
    expect(group(dialog, 'Ready to add').textContent).toContain('Rubric made: Build passes 40'),
  );
});

it('says no other skill needs a rubric when every remaining one was refused', async () => {
  const { source } = sourceWith('graded', undefined, 'all_refused');
  const dialog = await openDialog(source);
  const yours = group(dialog, 'Your skills');
  expect(within(yours).getByRole('heading').textContent).toBe('Your skills0');
  expect(yours.textContent).toContain('No other skill needs a rubric.');
  expect(yours.textContent).not.toContain('Every skill here has a rubric');
  cleanup();
  // With nothing refused, the other text. The answer is the mixed one with
  // its "your skills" rows left out: a home whose skills are all ready or on
  // Scores.
  const { source: made, controls } = sourceWith('graded', undefined, 'mixed');
  const mixed = pickerScenario('mixed');
  if (mixed.state !== 'loaded') throw new Error('mixed is loaded');
  controls.installed.mockResolvedValue({
    ...mixed,
    skills: mixed.skills.filter((skill) => skill.group !== 'yours'),
    yours: 0,
  });
  const dialog2 = await openDialog(made);
  expect(group(dialog2, 'Your skills').textContent).toContain(
    'Every skill here has a rubric or is on Scores.',
  );
});

it('moves focus with a skill tried again from Can’t be scored to its new button', async () => {
  const { source, controls } = sourceWith('graded', undefined, 'make_failed');
  const dialog = await openDialog(source);
  fireEvent.click(fold(dialog, 'Can’t be scored'));
  const tryAgain = within(group(dialog, 'Can’t be scored')).getByRole('button', {
    name: 'Try making a rubric again for pdf',
  });
  tryAgain.focus();
  // After the press, the next read lists it among Your skills.
  controls.installed.mockResolvedValue(pickerScenario('making'));
  fireEvent.click(tryAgain);
  await waitFor(() => expect(controls.makeRubric).toHaveBeenCalledWith(PDF));
  const moved = await within(dialog).findByRole('button', { name: 'Make a rubric for pdf' });
  expect(moved.closest('section')!.getAttribute('data-group')).toBe('yours');
  await waitFor(() => expect(document.activeElement).toBe(moved));
});

it('focuses the status line when a skill tried again is not shown afterwards', async () => {
  const { source, controls } = sourceWith('graded', undefined, 'make_failed');
  const dialog = await openDialog(source);
  fireEvent.click(fold(dialog, 'Can’t be scored'));
  controls.installed.mockResolvedValue(pickerScenario('empty'));
  fireEvent.click(
    within(group(dialog, 'Can’t be scored')).getByRole('button', {
      name: 'Try making a rubric again for pdf',
    }),
  );
  const status = listStatus(dialog);
  await waitFor(() => expect(document.activeElement).toBe(status));
  expect(status.textContent).toBe('Making a rubric for pdf again.');
});

/** The documents plugin's pdf skill (`picker.synthetic.json`). */
const PDF = 'plugin:documents@official/pdf';

/** "Add from a link" in the open Add a skill dialog. */
const linkBox = (dialog: HTMLElement) =>
  within(dialog).getByRole('region', { name: 'Add from a link' });
/** The made-up look-up's ID (`link.synthetic.json`). */
const LOOKUP_ID = '00000000-0000-4000-8000-0000000000c1';
const linkRows = (box: HTMLElement) =>
  [...box.querySelectorAll<HTMLElement>('.xt-score-link-row')].map((row) => ({
    path: row.dataset.path,
    name: row.querySelector('.xt-score-add-name')?.textContent,
    box: row.querySelector('input[type="checkbox"]') !== null,
    text: row.textContent,
  }));
async function lookUp(box: HTMLElement, link: string) {
  fireEvent.change(within(box).getByRole('textbox', { name: 'GitHub link' }), {
    target: { value: link },
  });
  fireEvent.submit(within(box).getByRole('textbox', { name: 'GitHub link' }).closest('form')!);
}

it('puts Add from a link above the three steps and previews what the native look-up found', async () => {
  const { source, controls } = sourceWith('graded');
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  const steps = within(dialog).getByRole('list', { name: 'How a skill gets on Scores' });
  // Above the steps, with its plain help line.
  expect(box.compareDocumentPosition(steps) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const field = within(box).getByRole('textbox', { name: 'GitHub link' });
  expect(within(box).getByText(LINK_HELP).id).toBe(field.getAttribute('aria-describedby'));
  // Nothing to look up yet.
  const lookUpButton = within(box).getByRole('button', { name: 'Look up' });
  expect(lookUpButton.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(lookUpButton);
  expect(controls.linkLookup).not.toHaveBeenCalled();
  await lookUp(box, '  https://github.com/octo/team-skills  ');
  await within(box).findByRole('region', { name: 'Skills at this link' });
  expect(controls.linkLookup).toHaveBeenCalledWith('https://github.com/octo/team-skills');
  expect(box.querySelector('.xt-score-link-source')!.textContent).toBe(
    'github.com/octo/team-skills · 0123456Top of the repository · default branch',
  );
  const rows = linkRows(box);
  expect(rows.map((row) => [row.name, row.box])).toEqual([
    ['api-review', true],
    ['changelog', true],
    ['claude-taken', false],
    ['docs-check', true],
    ['taken', false],
    ['test-plan', true],
  ]);
  expect(rows[0]!.text).toContain('Has a rubric (api-review): it can go on Scores once kept');
  expect(rows[0]!.text).toContain('2 files · 1 KB');
  expect(rows[0]!.text).toContain('Folder name: api-review');
  expect(rows[1]!.text).toContain('No rubric yet: XTrace can make one once it is kept');
  expect(rows[2]!.text).toContain(
    '~/.claude/skills/claude-taken already exists, so it can’t be kept',
  );
  expect(rows[3]!.text).toContain('Its rubric has a problem: it does not start with a header');
  expect(rows[4]!.text).toContain('~/.agents/skills/taken already exists, so it can’t be kept');
  expect(rows[5]!.text).toContain('2 files · 2 KB');
  // None is chosen at first, so Keep keeps nothing.
  expect(
    within(box)
      .getAllByRole('checkbox')
      .every((check) => !(check as HTMLInputElement).checked),
  ).toBe(true);
  const keep = within(box).getByRole('button', { name: 'Keep chosen skills' });
  expect(keep.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(keep);
  expect(controls.linkKeep).not.toHaveBeenCalled();
  expect(within(box).getByText(LINK_KEEP_NOTE)).toBeTruthy();
});

it('keeps the chosen skills by the look-up and their paths, says where they went and reads the list again', async () => {
  const { source, controls } = sourceWith('graded');
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  await lookUp(box, 'https://github.com/octo/team-skills');
  await within(box).findByRole('region', { name: 'Skills at this link' });
  const reads = controls.installed.mock.calls.length;
  // Ticked in reverse; sent in the preview's order.
  fireEvent.click(within(box).getByRole('checkbox', { name: 'changelog' }));
  fireEvent.click(within(box).getByRole('checkbox', { name: 'api-review' }));
  fireEvent.click(within(box).getByRole('button', { name: 'Keep 2 skills' }));
  await waitFor(() =>
    expect(controls.linkKeep).toHaveBeenCalledWith(LOOKUP_ID, [
      'skills/api-review',
      'skills/changelog',
    ]),
  );
  const status = within(box).getByRole('status');
  await waitFor(() =>
    expect(status.textContent).toBe(
      'api-review is in ~/.agents/skills/api-review and linked in ~/.claude/skills/api-review. ' +
        'changelog is in ~/.agents/skills/changelog; ~/.claude/skills already has that name, so no link was made there. ' +
        'They are listed below with your other skills.',
    ),
  );
  // The preview is gone; focus is on what happened, and the list is read again.
  expect(within(box).queryByRole('region', { name: 'Skills at this link' })).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(status));
  await waitFor(() => expect(controls.installed.mock.calls.length).toBeGreaterThan(reads));
  expect(controls.linkDiscard).not.toHaveBeenCalled();
});

it('offers one skill with a single Keep and no checkbox', async () => {
  const { source, controls } = sourceWith('graded');
  controls.linkLookup.mockResolvedValue(linkLookup('one'));
  controls.linkKeep.mockResolvedValue(linkKeep('kept_one'));
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  await lookUp(box, 'https://github.com/octo/skills/tree/main/skills/pdf');
  const keep = await within(box).findByRole('button', { name: 'Keep pdf' });
  expect(within(box).queryByRole('checkbox')).toBeNull();
  expect(box.querySelector('.xt-score-link-source')!.textContent).toBe(
    'github.com/octo/skills · 0123456skills/pdf · main',
  );
  fireEvent.click(keep);
  await waitFor(() => expect(controls.linkKeep).toHaveBeenCalledWith(LOOKUP_ID, ['skills/pdf']));
  await waitFor(() =>
    expect(within(box).getByRole('status').textContent).toBe(
      'pdf is in ~/.agents/skills/pdf and linked in ~/.claude/skills/pdf. It is listed below with your other skills.',
    ),
  );
});

it('says why a link gave nothing to preview', async () => {
  const { source, controls } = sourceWith('graded');
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  for (const [answer, text] of [
    [{ state: 'not_github_link' }, 'Only public GitHub links work for now.'],
    [{ state: 'no_skill' }, 'No skill found at this link.'],
    [
      { state: 'not_found' },
      'GitHub has no public repository, branch, tag or commit by that name.',
    ],
    [
      { state: 'too_large', limit: 'download' },
      'That repository can’t be looked up: it is larger than XTrace downloads. Link the skill’s own folder instead.',
    ],
    [{ state: 'timed_out' }, 'GitHub did not answer in time. Try again.'],
  ] as const) {
    controls.linkLookup.mockResolvedValueOnce(answer);
    await lookUp(box, 'https://example.com/x');
    await waitFor(() => expect(within(box).getByRole('alert').textContent).toBe(text));
    expect(within(box).queryByRole('region', { name: 'Skills at this link' })).toBeNull();
  }
  controls.linkLookup.mockRejectedValueOnce(new Error('ipc'));
  await lookUp(box, 'https://github.com/octo/skills');
  await waitFor(() =>
    expect(within(box).getByRole('alert').textContent).toBe(
      'The link could not be looked up. Try again.',
    ),
  );
});

it('shows the fresh preview and keeps nothing when a chosen name was taken meanwhile', async () => {
  const { source, controls } = sourceWith('graded');
  controls.linkKeep.mockResolvedValue(linkKeep('changed'));
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  await lookUp(box, 'https://github.com/octo/team-skills');
  await within(box).findByRole('region', { name: 'Skills at this link' });
  fireEvent.click(within(box).getByRole('checkbox', { name: 'api-review' }));
  fireEvent.click(within(box).getByRole('checkbox', { name: 'docs-check' }));
  fireEvent.click(within(box).getByRole('button', { name: 'Keep 2 skills' }));
  await waitFor(() =>
    expect(within(box).getByRole('status').textContent).toBe(
      'Something changed since the preview (a name is taken now), so nothing was kept. Check the list again.',
    ),
  );
  // api-review is taken now: no checkbox, and only docs-check stays chosen.
  expect(linkRows(box).find((row) => row.name === 'api-review')!.box).toBe(false);
  expect(within(box).getByRole('button', { name: 'Keep 1 skill' })).toBeTruthy();
});

it('cancels a look-up going and drops the preview when the dialog closes', async () => {
  const { source, controls } = sourceWith('graded');
  let answer: (value: Awaited<ReturnType<GradingControls['linkLookup']>>) => void = () => {};
  controls.linkLookup.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        answer = resolve;
      }),
  );
  controls.linkCancel.mockImplementation(async () => {
    answer({ state: 'cancelled' });
    return true;
  });
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  await lookUp(box, 'https://github.com/octo/skills');
  await waitFor(() =>
    expect(within(box).getByRole('status').textContent).toBe(
      'Looking up https://github.com/octo/skills…',
    ),
  );
  expect(
    within(box).getByRole('button', { name: 'Looking up…' }).getAttribute('aria-disabled'),
  ).toBe('true');
  fireEvent.click(within(box).getByRole('button', { name: 'Cancel' }));
  await waitFor(() =>
    expect(within(box).getByRole('status').textContent).toBe('Look-up cancelled.'),
  );
  expect(controls.linkCancel).toHaveBeenCalledTimes(1);
  // A preview is dropped, with its downloaded files, when the dialog closes.
  await lookUp(box, 'https://github.com/octo/team-skills');
  await within(box).findByRole('region', { name: 'Skills at this link' });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(controls.linkDiscard).toHaveBeenCalledWith(LOOKUP_ID));
});

it('keeps the preview after a keep the native side refused, and drops it when the dialog closes', async () => {
  const { source, controls } = sourceWith('graded');
  controls.linkKeep.mockResolvedValue({ state: 'agents_folder_unusable' });
  const dialog = await openDialog(source);
  const box = linkBox(dialog);
  await lookUp(box, 'https://github.com/octo/team-skills');
  await within(box).findByRole('region', { name: 'Skills at this link' });
  fireEvent.click(within(box).getByRole('checkbox', { name: 'api-review' }));
  fireEvent.click(within(box).getByRole('button', { name: 'Keep 1 skill' }));
  await waitFor(() =>
    expect(within(box).getByRole('status').textContent).toBe(
      '~/.agents/skills is not a folder XTrace can use, so nothing was kept.',
    ),
  );
  // Still held natively: the preview stays, so Keep can be tried again.
  expect(within(box).getByRole('region', { name: 'Skills at this link' })).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(controls.linkDiscard).toHaveBeenCalledWith(LOOKUP_ID));
});

it('drops nothing on close once a keep or an expired preview ended the look-up', async () => {
  for (const answer of [linkKeep('kept_one'), { state: 'expired' } as const]) {
    const { source, controls } = sourceWith('graded');
    controls.linkLookup.mockResolvedValue(linkLookup('one'));
    controls.linkKeep.mockResolvedValue(answer);
    const dialog = await openDialog(source);
    const box = linkBox(dialog);
    await lookUp(box, 'https://github.com/octo/skills/tree/main/skills/pdf');
    fireEvent.click(await within(box).findByRole('button', { name: 'Keep pdf' }));
    await waitFor(() => expect(controls.linkKeep).toHaveBeenCalled());
    await waitFor(() =>
      expect(within(box).queryByRole('region', { name: 'Skills at this link' })).toBeNull(),
    );
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(controls.linkDiscard).not.toHaveBeenCalled();
    cleanup();
  }
});

it('says a later skill with the same folder name can’t be kept, naming the one above', () => {
  expect(linkKeepStateText({ state: 'same_name_above', path: 'skills/pdf' })).toBe(
    'skills/pdf, listed above, gets the same folder name; only that one can be kept',
  );
});
