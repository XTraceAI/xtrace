import synthetic from './details.synthetic.json';
import type { GradingRunDetails } from '../../data/generated/GradingRunDetails';

/**
 * Test-only: a run's details as the app's own Rust rule gave them over
 * seeded disposable stores and grade folders (`grading_details_tests.rs`,
 * byte-checked there; regenerate with `XTRACE_WRITE_SCORES_SYNTHETIC=1`).
 * Run IDs, projects and scores are those of the Scores page's `graded`
 * answer; the texts are made up. Nothing here computes a value.
 */
export type DetailsScenario =
  | 'full'
  | 'no_criteria'
  | 'folder_not_kept'
  | 'content_deleted'
  | 'unreadable'
  | 'not_found'
  | 'failed'
  | 'not_configured';

// JSON imports widen literal unions; the export is the generated shape.
const scenarios = synthetic as unknown as Record<DetailsScenario, GradingRunDetails>;

/** A scenario's answer, copied so a view that edits it changes no other. */
export const detailsScenario = (name: DetailsScenario): GradingRunDetails =>
  structuredClone(scenarios[name]);

/**
 * The answer for the run `runId` of the `graded` overview: atlas's full
 * details, beacon's from before rubrics, cedar's with its folder pruned;
 * not found for any other run.
 */
export function detailsFor(runId: string): GradingRunDetails {
  for (const name of ['full', 'no_criteria', 'folder_not_kept'] as const) {
    const answer = scenarios[name];
    if (answer.state === 'loaded' && answer.run_id === runId) return detailsScenario(name);
  }
  return detailsScenario('not_found');
}
