import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import {
  linkKeep,
  linkLookup,
  type LinkKeepScenario,
  type LinkLookupScenario,
} from '../src/app/scores/link.synthetic';
import { pickerScenario, type PickerScenario } from '../src/app/scores/picker.synthetic';
import { scenario, type Scenario } from '../src/app/scores/scores.synthetic';
import { DataProvider } from '../src/data/DataProvider';
import type { GradingControls } from '../src/data/DataSource';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The app's own Shell and routes over the F1 fixture, with grading answered
 * from the Rust rules' synthetic answers (`scores.synthetic.json`,
 * `picker.synthetic.json`): `?scenario=` picks the overview, `?picker=` the
 * installed skills and `?path=` the address. Nothing runs: every start,
 * cancel, add, remove and rubric making is recorded on `window.__grading`
 * for the spec to count. Starts are answered as `?start=` says (started by
 * default; `waiting` puts it in line with its own run ID). An add is answered as `?add=` says (added by default); an
 * answered add moves the installed skills to `added` and the overview to
 * `added`, as the app would after storing it. A Make a rubric press is
 * answered as `?make=` says (started by default); a started one moves the
 * installed skills to `making` and the overview to `making_rubric`, and a
 * cancel of it moves them to `make_failed` and `graded`. A link Look up is
 * answered as `?link=` says (`several` by default, `one`, an error state, or
 * `wait` to stay unanswered until cancelled) and a Keep as `?keep=` says
 * (`kept_several` by default, `kept_one`, `changed`, or another state); each
 * is recorded too.
 */
const params = new URLSearchParams(location.search);
let name = (params.get('scenario') ?? 'graded') as Scenario;
let picker = (params.get('picker') ?? 'mixed') as PickerScenario;
const path = params.get('path') ?? '/scores';
const calls = {
  overviews: 0,
  installed: 0,
  starts: [] as string[][],
  cancels: [] as string[],
  adds: [] as string[],
  removes: [] as string[],
  makes: [] as string[],
  lookups: [] as string[],
  linkCancels: 0,
  keeps: [] as [string, string[]][],
  discards: [] as string[],
};
let waiting: (() => void) | null = null;
Object.assign(window, { __grading: calls });

const controls: GradingControls = {
  skills: async () => ({ state: 'not_configured' }),
  overview: async () => {
    calls.overviews += 1;
    return scenario(name);
  },
  runs: async () => ({ state: 'not_configured' }),
  start: async (projectPath, skillId) => {
    calls.starts.push([projectPath, skillId]);
    const state = params.get('start') ?? 'started';
    if (state === 'started') return { state, run_id: '00000000-0000-4000-8000-0000000000ff' };
    if (state === 'waiting') return { state, run_id: '00000000-0000-4000-8000-0000000000fd' };
    return { state } as Awaited<ReturnType<GradingControls['start']>>;
  },
  cancel: async (runId) => {
    calls.cancels.push(runId);
    if (picker === 'making') {
      picker = 'make_failed';
      name = 'graded';
    }
    return true;
  },
  installed: async () => {
    calls.installed += 1;
    return pickerScenario(picker);
  },
  addSkill: async (location) => {
    calls.adds.push(location);
    const state = params.get('add') ?? 'added';
    if (state !== 'added') return { state } as Awaited<ReturnType<GradingControls['addSkill']>>;
    picker = 'added';
    name = 'added';
    return { state, skill_id: 'readme-check' };
  },
  removeSkill: async (skillId) => {
    calls.removes.push(skillId);
    name = 'graded';
    return { state: 'removed' };
  },
  makeRubric: async (location) => {
    calls.makes.push(location);
    const state = params.get('make') ?? 'started';
    if (state !== 'started') return { state } as Awaited<ReturnType<GradingControls['makeRubric']>>;
    picker = 'making';
    name = 'making_rubric';
    return { state, job_id: '00000000-0000-4000-8000-0000000000aa' };
  },
  linkLookup: async (link) => {
    calls.lookups.push(link);
    const state = params.get('link') ?? 'several';
    if (state === 'wait')
      return new Promise((resolve) => {
        waiting = () => resolve({ state: 'cancelled' });
      });
    if (state === 'one' || state === 'several') return linkLookup(state as LinkLookupScenario);
    return { state } as Awaited<ReturnType<GradingControls['linkLookup']>>;
  },
  linkCancel: async () => {
    calls.linkCancels += 1;
    const cancel = waiting;
    waiting = null;
    cancel?.();
    return cancel !== null;
  },
  linkKeep: async (lookupId, paths) => {
    calls.keeps.push([lookupId, paths]);
    const state = params.get('keep') ?? 'kept_several';
    if (state === 'kept_one' || state === 'kept_several' || state === 'changed')
      return linkKeep(state as LinkKeepScenario);
    return { state } as Awaited<ReturnType<GradingControls['linkKeep']>>;
  },
  linkDiscard: async (lookupId) => {
    calls.discards.push(lookupId);
    return true;
  },
};
const source = new FixtureDataSource(fixture as FixtureExport);
Object.defineProperty(source, 'grading', { value: controls });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
