import synthetic from './link.synthetic.json';
import type { GradingLinkKeep } from '../../data/generated/GradingLinkKeep';
import type { GradingLinkLookup } from '../../data/generated/GradingLinkLookup';

/**
 * Test-only: "Add from a link" answers as the app's own Rust rules gave them
 * over made-up repositories and a made-up home (`skill_link_tests.rs`,
 * byte-checked there; regenerate with `XTRACE_WRITE_SCORES_SYNTHETIC=1`).
 * Nothing here computes a value.
 */
export type LinkLookupScenario = 'one' | 'several';
export type LinkKeepScenario = 'kept_one' | 'kept_several' | 'changed';

// JSON imports widen literal unions; the export is the generated shape.
const answers = synthetic as unknown as Record<LinkLookupScenario, GradingLinkLookup> &
  Record<LinkKeepScenario, GradingLinkKeep>;

/** A look-up answer, copied so a view that edits it changes no other. */
export const linkLookup = (name: LinkLookupScenario): GradingLinkLookup =>
  structuredClone(answers[name]);

/** A keep answer, copied the same way. */
export const linkKeep = (name: LinkKeepScenario): GradingLinkKeep => structuredClone(answers[name]);
