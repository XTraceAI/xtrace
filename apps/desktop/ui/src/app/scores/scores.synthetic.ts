import synthetic from './scores.synthetic.json';
import type { GradingOverview } from '../../data/generated/GradingOverview';

/**
 * Test-only: the Scores page's answers as the app's own Rust rule gave them
 * over seeded disposable stores (`grading_overview_tests.rs`, byte-checked
 * there; regenerate with `XTRACE_WRITE_SCORES_SYNTHETIC=1`). Made-up projects,
 * not real history and not a design sample. Nothing here computes a value.
 */
export type Scenario =
  | 'no_projects'
  | 'empty'
  | 'graded'
  | 'partial'
  | 'running'
  | 'running_unlisted'
  | 'long_findings'
  | 'failed'
  | 'added'
  | 'added_problem'
  | 'making_rubric'
  | 'not_configured'
  | 'failed_read';

// JSON imports widen literal unions; the export is the generated shape.
export const scenarios = synthetic as unknown as Record<Scenario, GradingOverview>;

/** A scenario's answer, copied so a view that edits it changes no other. */
export const scenario = (name: Scenario): GradingOverview => structuredClone(scenarios[name]);
