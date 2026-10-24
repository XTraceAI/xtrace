import synthetic from './picker.synthetic.json';
import type { GradingInstalledSkills } from '../../data/generated/GradingInstalledSkills';

/**
 * Test-only: the Add a grading skill dialog's answers as the app's own Rust rule gave
 * them over made-up installed skills (`grading_picker_tests.rs`,
 * byte-checked there; regenerate with `XTRACE_WRITE_SCORES_SYNTHETIC=1`).
 * Nothing here computes a value.
 */
export type PickerScenario =
  | 'mixed'
  | 'plugins_unread'
  | 'added'
  | 'many'
  | 'making'
  | 'made'
  | 'make_failed'
  | 'yours'
  | 'all_refused'
  | 'empty'
  | 'not_configured'
  | 'failed';

// JSON imports widen literal unions; the export is the generated shape.
export const pickerScenarios = synthetic as unknown as Record<
  PickerScenario,
  GradingInstalledSkills
>;

/** A scenario's answer, copied so a view that edits it changes no other. */
export const pickerScenario = (name: PickerScenario): GradingInstalledSkills =>
  structuredClone(pickerScenarios[name]);
