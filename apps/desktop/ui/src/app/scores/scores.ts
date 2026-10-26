import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useData } from '../../data/DataProvider';
import type { Unsubscribe } from '../../data/DataSource';
import type { GradingAttempt } from '../../data/generated/GradingAttempt';
import type { GradingFailure } from '../../data/generated/GradingFailure';
import type { GradingInstalledRubric } from '../../data/generated/GradingInstalledRubric';
import type { GradingInstalledScores } from '../../data/generated/GradingInstalledScores';
import type { GradingLinkKeep } from '../../data/generated/GradingLinkKeep';
import type { GradingLinkKeepState } from '../../data/generated/GradingLinkKeepState';
import type { GradingLinkKept } from '../../data/generated/GradingLinkKept';
import type { GradingLinkLimit } from '../../data/generated/GradingLinkLimit';
import type { GradingLinkLookup } from '../../data/generated/GradingLinkLookup';
import type { GradingLinkSource } from '../../data/generated/GradingLinkSource';
import type { GradingInstalledSkill } from '../../data/generated/GradingInstalledSkill';
import type { GradingPluginProblem } from '../../data/generated/GradingPluginProblem';
import type { GradingRubricProblem } from '../../data/generated/GradingRubricProblem';
import type { GradingSkillAdd } from '../../data/generated/GradingSkillAdd';
import type { GradingSkillProblem } from '../../data/generated/GradingSkillProblem';
import type { GradingSkillRemove } from '../../data/generated/GradingSkillRemove';
import type { GradingSkillSource } from '../../data/generated/GradingSkillSource';
import type { GradingOverview } from '../../data/generated/GradingOverview';
import type { GradingProgress } from '../../data/generated/GradingProgress';
import type { GradingReportOpen } from '../../data/generated/GradingReportOpen';
import type { GradingRunDetails } from '../../data/generated/GradingRunDetails';
import type { GradingRunGap } from '../../data/generated/GradingRunGap';
import type { GradingRubricCriterion } from '../../data/generated/GradingRubricCriterion';
import type { GradingRubricFailure } from '../../data/generated/GradingRubricFailure';
import type { GradingRubricMake } from '../../data/generated/GradingRubricMake';
import type { GradingRubricNow } from '../../data/generated/GradingRubricNow';
import type { GradingRubricProgress } from '../../data/generated/GradingRubricProgress';
import type { GradingRunningNow } from '../../data/generated/GradingRunningNow';
import type { GradingSeverity } from '../../data/generated/GradingSeverity';
import type { GradingStart } from '../../data/generated/GradingStart';
import type { GradingWaitingElsewhere } from '../../data/generated/GradingWaitingElsewhere';
import type { ProjectSourceId } from '../../data/generated/ProjectSourceId';
import { events } from '../../data/ipc-names';
import { queryKeys } from '../../data/query-client';
import { refreshQueries } from '../../data/subscribe-invalidation';
import { clock, isInstant } from '../../kit/clock';
import { byteSize } from '../../kit/format';
import type { ControlTone } from '../../kit/control-tone';
import { HOST_NAMES } from '../../kit/hosts';
import { agentTime } from '../agent-duration';
import { plural } from '../dashboard/present';

/**
 * The Scores page's words. Every number, average, ranking and state here was
 * decided natively (`GradingOverview`); this file only says them: labels for
 * typed values, a time in this Mac's clock, the elapsed time a progress
 * event reported. Nothing is averaged, ranked or derived again.
 */
export type Loaded = Extract<GradingOverview, { state: 'loaded' }>;

/** The overview query: read when a Scores page opens, and on every change. */
export function useGradingOverview() {
  const { source } = useData();
  const grading = source.grading;
  return useQuery({
    queryKey: queryKeys.gradingOverview,
    queryFn: () => grading!.overview(),
    enabled: !!grading,
  });
}

/** Read the overview again now, never cancelling a read already running. */
export function useRefreshOverview() {
  const client = useQueryClient();
  return () => refreshQueries(client, [queryKeys.gradingOverview], () => {});
}

/**
 * The details of the run `runId` behind a shown score, read while its
 * dialog is open. What they hold, in which order, and why criteria or a
 * report are missing is native.
 */
export function useRunDetails(runId: string | null, open: boolean) {
  const { source } = useData();
  const grading = source.grading;
  return useQuery({
    queryKey: queryKeys.gradingRunDetails(runId ?? ''),
    queryFn: () => grading!.runDetails(runId!),
    enabled: !!grading && open && runId !== null,
  });
}

/**
 * The installed skills for the Add a grading skill dialog, read while it is open.
 * Which rows are on Scores, in which group and order, is native.
 */
export function useInstalledSkills(open: boolean) {
  const { source } = useData();
  const grading = source.grading;
  return useQuery({
    queryKey: queryKeys.gradingInstalled,
    queryFn: () => grading!.installed(),
    enabled: !!grading && open,
  });
}

/** Read the installed skills and the overview again now. */
export function useRefreshInstalled() {
  const client = useQueryClient();
  return () =>
    refreshQueries(client, [queryKeys.gradingInstalled, queryKeys.gradingOverview], () => {});
}

const isProgress = (payload: unknown): payload is GradingProgress => {
  const report = payload as Partial<GradingProgress> | null | undefined;
  return (
    typeof report?.run_id === 'string' &&
    (report.state === 'running' || report.state === 'finished') &&
    typeof report.elapsed_seconds === 'number' &&
    Number.isFinite(report.elapsed_seconds) &&
    report.elapsed_seconds >= 0
  );
};

/**
 * The elapsed seconds the latest `grading://progress` reported, by run ID.
 * Progress only drives this display: whether a run is going is the stored
 * status the overview carries. A report for a run the overview does not show
 * as going, or a run's last report, reads the overview again.
 */
export function useGradingProgress(runningRunId: string | undefined) {
  const { source } = useData();
  const refresh = useRefreshOverview();
  const [elapsed, setElapsed] = useState<Record<string, number>>({});
  const running = useRef(runningRunId);
  const reread = useRef(refresh);
  useEffect(() => {
    running.current = runningRunId;
    reread.current = refresh;
  });
  useEffect(() => {
    let live = true;
    let stop: Unsubscribe | undefined;
    const heard = (payload?: unknown) => {
      if (!live || !isProgress(payload)) return;
      setElapsed((current) => ({ ...current, [payload.run_id]: payload.elapsed_seconds }));
      if (payload.state === 'finished' || payload.run_id !== running.current) reread.current();
    };
    Promise.resolve(source.subscribe(events.gradingProgress, heard)).then(
      (unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      },
      () => {},
    );
    return () => {
      live = false;
      stop?.();
    };
  }, [source]);
  return elapsed;
}

const isRubricProgress = (payload: unknown): payload is GradingRubricProgress => {
  const report = payload as Partial<GradingRubricProgress> | null | undefined;
  return (
    typeof report?.job_id === 'string' &&
    typeof report.location === 'string' &&
    (report.state === 'running' || report.state === 'finished') &&
    typeof report.elapsed_seconds === 'number' &&
    Number.isFinite(report.elapsed_seconds) &&
    report.elapsed_seconds >= 0
  );
};

/**
 * The elapsed seconds the latest `grading://rubric-progress` reported, by
 * job ID, while the Add a grading skill dialog is open. Progress only drives this
 * display: whether a job is going is the installed list's `rubric_job`. A
 * job's last report reads the installed skills and the overview again.
 */
export function useRubricProgress(open: boolean) {
  const { source } = useData();
  const refresh = useRefreshInstalled();
  const [elapsed, setElapsed] = useState<Record<string, number>>({});
  const reread = useRef(refresh);
  useEffect(() => {
    reread.current = refresh;
  });
  useEffect(() => {
    if (!open) return;
    let live = true;
    let stop: Unsubscribe | undefined;
    const heard = (payload?: unknown) => {
      if (!live || !isRubricProgress(payload)) return;
      setElapsed((current) => ({ ...current, [payload.job_id]: payload.elapsed_seconds }));
      if (payload.state === 'finished') reread.current();
    };
    Promise.resolve(source.subscribe(events.gradingRubricProgress, heard)).then(
      (unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      },
      () => {},
    );
    return () => {
      live = false;
      stop?.();
    };
  }, [source, open]);
  return elapsed;
}

/** A score as drawn: the number, or a dash when there is none. */
export const scoreText = (score: number | null | undefined) =>
  score === null || score === undefined ? '—' : String(score);

/** When a run finished or started, with the year: grades have no age limit. */
export const timeText = (ms: number) =>
  isInstant(ms) ? clock(ms, { date: 'year' }) : 'an unreadable time';

/**
 * When a run that is going started: the time of day, since a run lasts
 * under an hour; the row's tooltip states the date as well.
 */
export const startedText = (ms: number) => (isInstant(ms) ? clock(ms) : 'at an unreadable time');

/** Where each app that names projects is called by its own name. */
export const sourceNames: Record<ProjectSourceId, string> = {
  claude_code: HOST_NAMES.claude,
  claude_app: 'Claude app',
  codex_app: 'Codex app',
  codex_cli: HOST_NAMES.codex,
  cursor: HOST_NAMES.cursor,
};

/** Why a run failed or stopped, as the end of a sentence. */
export const failureText: Record<GradingFailure, string> = {
  app_closed: 'XTrace closed during the grading',
  project_changed: 'the project changed before the grading started',
  project_unavailable: 'the project folder could not be opened',
  project_written: 'the grading changed files in the project, so its result was not kept',
  project_work_folder_uncheckable:
    'a .claude-md-doctor folder in the project is a link or too large to check',
  skill_unavailable: 'the skill could not be read',
  skill_too_large: 'the skill’s folder has more files or data than XTrace copies for a grading',
  skill_link_refused: 'the skill’s folder has a link to a folder, to nothing or out of the folder',
  rubric_invalid: 'the skill’s rubric is missing or not valid',
  skill_changed: 'the skill’s rubric changed while the grading was starting; grade again',
  sandbox_unavailable:
    'Claude Code’s sandbox is not available, so the project could not be kept read-only',
  path_has_whitespace: 'a folder it needs has a space in its path',
  staging_failed: 'its private folders could not be made',
  setup_failed: 'it could not be set up',
  spawn_failed: 'Claude Code could not be started',
  claude_too_old:
    'Claude Code is too old for grading, or its version could not be read; update Claude Code',
  claude_failed: 'Claude Code stopped with an error',
  timeout: 'it ran past its time limit',
  unobservable: 'XTrace lost track of it',
  result_missing: 'it finished without a result',
  result_invalid: 'its result could not be read',
  archive_failed: 'its result could not be saved',
  internal_error: 'XTrace hit an error while saving it',
  claude_unavailable: 'Claude Code was not found on this Mac when its turn came',
  other: 'of a reason this version of XTrace does not know',
};

/** A finding's weight in words, and the tone its icon is drawn in. */
export const severity: Record<GradingSeverity, { label: string; tone: ControlTone }> = {
  critical: { label: 'Critical', tone: 'danger' },
  warning: { label: 'Warning', tone: 'warning' },
  info: { label: 'Note', tone: 'info' },
  pass: { label: 'Pass', tone: 'success' },
};

/** Why a run's details could not be shown. */
export function detailsText(answer: Exclude<GradingRunDetails, { state: 'loaded' }>) {
  switch (answer.state) {
    case 'not_found':
      return 'This grading is no longer stored. Grade the project again to see its details.';
    case 'not_configured':
      return 'Grading details are not available in this copy of XTrace.';
    case 'failed':
      return 'The details could not be read. Try again.';
  }
}

/** Stored findings that could not be read. */
export const FINDINGS_UNREADABLE = 'This grade’s stored findings could not be read.';

/** Why a run shows no points per criterion, as a sentence. */
export const criteriaGapText: Record<GradingRunGap, string> = {
  folder_not_kept:
    'Only the newest grades keep their saved files, and this one’s were removed, so its points per criterion are gone.',
  not_reported: 'This grade has no points per criterion: it was made before skills had rubrics.',
  unreadable:
    'This grade’s saved files could not be read, so its points per criterion are not shown.',
};

/** Why a run's full report cannot be opened, as a sentence. */
export const reportGapText: Record<GradingRunGap, string> = {
  folder_not_kept: 'Only the newest grades keep their full report, and this one’s was removed.',
  not_reported: 'The skill wrote no full report for this grade.',
  unreadable: 'This grade’s full report could not be read.',
};

/** What an Open full report press that opened nothing says. */
export function openReportText(answer: Exclude<GradingReportOpen, { state: 'opened' }>) {
  switch (answer.state) {
    case 'not_found':
      return 'This grading is no longer stored, so there is no report to open.';
    case 'content_deleted':
      return 'Stored content was deleted, so the full report is gone.';
    case 'unavailable':
      return reportGapText[answer.reason];
    case 'not_configured':
      return 'Opening reports is not available in this copy of XTrace.';
    case 'failed':
      return OPEN_REPORT_FAILED;
  }
}

/** An Open full report press whose request itself failed. */
export const OPEN_REPORT_FAILED = 'The report could not be opened. Try again.';

/** How long a run has been going, in the app's one way of writing agent time. */
export const elapsedText = (seconds: number) => agentTime(seconds * 1000);

/** How often a going run's elapsed time is read again from this Mac's clock. */
export const ELAPSED_TICK_MS = 1000;

/**
 * Seconds a going run has been going: since its stored start on this Mac's
 * clock, never less than the latest progress report. Only the report when
 * the start is unreadable or later than now; none when neither is known.
 */
export function elapsedSeconds(
  startedAtMs: number | undefined,
  reported: number | undefined,
  nowMs: number,
): number | undefined {
  const since =
    startedAtMs !== undefined && isInstant(startedAtMs) && nowMs >= startedAtMs
      ? (nowMs - startedAtMs) / 1000
      : undefined;
  if (since === undefined) return reported;
  return reported === undefined ? since : Math.max(since, reported);
}

/**
 * [`elapsedSeconds`] for a run going now, re-read every [`ELAPSED_TICK_MS`]
 * so the time moves between progress reports. Formatting only: whether the
 * run is going is the stored status the overview carries (`startedAtMs` is
 * given only for such a run).
 */
export function useElapsed(startedAtMs: number | undefined, reported: number | undefined) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (startedAtMs === undefined) return;
    const timer = setInterval(() => setNow(Date.now()), ELAPSED_TICK_MS);
    return () => clearInterval(timer);
  }, [startedAtMs]);
  return startedAtMs === undefined ? undefined : elapsedSeconds(startedAtMs, reported, now);
}

/** A row's newer run, in words: what it is doing or how it ended. */
export function attemptText(attempt: GradingAttempt, elapsed: number | undefined) {
  switch (attempt.state) {
    case 'running':
      return elapsed === undefined
        ? `Grading now · started ${startedText(attempt.started_at_ms)}`
        : `Grading now · ${elapsedText(elapsed)}`;
    case 'failed':
      return `Last try failed: ${failureText[attempt.reason]}`;
    case 'cancelled':
      return 'Last try was cancelled';
    case 'interrupted':
      return `Last try stopped: ${failureText[attempt.reason]}`;
  }
}

/** A waiting grading's place in line, by how many wait before it. */
const placeText = (ahead: number) => (ahead === 0 ? 'next in line' : `${ahead} ahead`);

/** A waiting grading's row state, in words. */
export const waitingText = (ahead: number) => `Waiting · ${placeText(ahead)}`;

/**
 * A waiting grading no row shows: which project, with which skill, its place
 * in line, and why it has no row.
 */
export function waitingElsewhereText(waiting: GradingWaitingElsewhere) {
  const project = waiting.project_name ?? waiting.project_path;
  const skill = waiting.skill_name ? ` with ${waiting.skill_name}` : '';
  return {
    lead: `${project} is waiting to be graded${skill} · ${placeText(waiting.ahead)}`,
    detail:
      waiting.project_name === null
        ? 'This project is no longer in your project list, so no row shows it.'
        : 'This skill is not on Scores, so no row shows it.',
  };
}

/** Why a rubric cannot be made now, naming the grading that is going. */
export function oneAtATime(running: GradingRunningNow) {
  const project = running.project_name ?? running.project_path;
  const skill = running.skill_name ? ` with ${running.skill_name}` : '';
  return `Only one grading runs at a time. ${project} is being graded${skill} now; wait for it to finish or cancel it.`;
}

/**
 * The run going now when no row on the page shows it: which project, with
 * which skill, and why it has no row when its project is no longer listed.
 */
export function runningElsewhereText(running: GradingRunningNow) {
  const skill = running.skill_name ? ` with ${running.skill_name}` : '';
  return running.project_name === null
    ? {
        lead: `${running.project_path} is being graded${skill}`,
        detail: 'This project is no longer in your project list, so no row shows it.',
      }
    : { lead: `${running.project_name} is being graded${skill}`, detail: null };
}

/**
 * Why a grading cannot start while a rubric is being made: the two share the
 * one slot.
 */
export function rubricOneAtATime(making: GradingRubricNow) {
  return `Only one grading runs at a time, and making a rubric counts as one. A rubric for ${making.name} is being made now; wait for it to finish or cancel it.`;
}

/** What a Grade press that neither started a run nor joined the line says. */
export function startText(
  answer: Exclude<GradingStart, { state: 'started' } | { state: 'waiting' }>,
  project: string,
) {
  switch (answer.state) {
    case 'already_going':
      return `${project} is already being graded with this skill, or waiting to be.`;
    case 'not_a_project':
      return `${project} is no longer in your project list, so it was not graded.`;
    case 'unknown_skill':
      return 'This skill is not on Scores, so nothing was graded.';
    case 'skill_unavailable':
      return 'This skill can’t grade right now, so nothing was graded.';
    case 'claude_unavailable':
      return 'Claude Code was not found on this Mac, so nothing was graded.';
    case 'not_configured':
      return 'Grading is not available in this copy of XTrace.';
    case 'stopped':
      return 'XTrace is closing, so nothing was graded.';
    case 'failed':
      return 'The grading could not be started. Try again.';
  }
}

/** A Grade press while another press's start is still being answered. */
export const STARTING = 'Starting a grading…';
/** A Grade press whose request itself failed. */
export const START_FAILED = 'The grading could not be started. Try again.';
/**
 * What a Cancel press says, by whether the run was still going (or, for a
 * waiting one, still in line).
 */
export const cancelText = (stopping: boolean, waiting = false) =>
  waiting
    ? stopping
      ? 'Cancelling the grading…'
      : 'That grading was no longer waiting.'
    : stopping
      ? 'Stopping the grading…'
      : 'That grading had already finished.';

/** Why a rubric was refused, as the end of a sentence. */
export const rubricProblemText: Record<GradingRubricProblem, string> = {
  missing: 'there is no xtrace-rubric.md',
  not_regular_file: 'it is a link or not a regular file',
  too_large: 'it is larger than XTrace reads',
  unreadable: 'it could not be read',
  not_utf8: 'it is not UTF-8 text',
  no_header: 'it does not start with a header between --- lines',
  header_unterminated: 'its header has no closing --- line',
  header_line: 'a header line is not “key: value”',
  unknown_key: 'its header has a key the format does not define',
  duplicate_key: 'its header names a key twice',
  missing_key: 'its header is missing a required key',
  protocol: 'it does not say protocol: 1',
  id: 'its id is not lowercase letters, digits and dashes',
  field: 'its name, version or description is empty or too long',
  target: 'its target is not project',
  heading: 'a ## heading is neither a criterion with points nor Findings',
  points: 'a criterion’s points are not a whole number from 1 to 100',
  criterion_name: 'a criterion’s name is empty or too long',
  duplicate_criterion: 'it names a criterion twice',
  no_guidance: 'a section has no guidance under it',
  duplicate_findings: 'it has two Findings sections',
  findings_points: 'its Findings section has points',
  no_criteria: 'it has no criteria',
  too_many_criteria: 'it has more criteria than XTrace allows',
  total: 'its points do not add up to 100',
};

/** The first letter of a sentence end, capitalised to start a line. */
const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/** A rubric problem as its own line in the Add a grading skill dialog. */
export const rubricProblemLine = (problem: GradingRubricProblem) =>
  `${sentence(rubricProblemText[problem])}.`;

/** Why an added skill cannot grade now, as a sentence. */
export function skillProblemText(problem: GradingSkillProblem) {
  switch (problem.reason) {
    case 'folder_missing':
      return 'Its folder is gone or was replaced by a link.';
    case 'plugin_unavailable':
      return 'Its Claude Code plugin is no longer installed and turned on.';
    case 'skill_unreadable':
      return 'Its SKILL.md can no longer be read.';
    case 'rubric_missing':
      return 'Its folder no longer has an xtrace-rubric.md.';
    case 'rubric_invalid':
      return `Its xtrace-rubric.md is no longer valid: ${rubricProblemText[problem.rubric]}.`;
    case 'rubric_id_changed':
      return 'Its xtrace-rubric.md now has another id.';
  }
}

/** Where an installed skill was found. */
export function skillSourceText(source: GradingSkillSource) {
  switch (source.kind) {
    case 'claude':
      return HOST_NAMES.claude;
    case 'agents':
      return 'Agents';
    case 'codex':
      return HOST_NAMES.codex;
    case 'plugin':
      return `Plugin ${source.plugin}`;
  }
}

/** Why an installed skill shows no Add button, or nothing when it does. */
export function installedStateText(scores: GradingInstalledScores) {
  switch (scores.state) {
    case 'addable':
    case 'not_ready':
      return null;
    case 'on_scores':
      return 'On Scores';
    case 'built_in':
      return 'On Scores (built in)';
    case 'id_on_scores':
      return 'On Scores from another folder';
    case 'id_clash':
      return `Same id as ${scores.name}; only one can be added`;
  }
}

/** Which Claude Code file listing plugins could not be used, and why. */
const pluginFileText: Record<GradingPluginProblem['file'], string> = {
  plugins_folder: 'Claude Code’s plugins folder',
  installed_plugins: 'Claude Code’s installed_plugins.json',
  settings: 'Claude Code’s settings.json',
};
const pluginFailureText: Record<GradingPluginProblem['failure'], string> = {
  link: 'is a link',
  unreadable: 'could not be read',
  malformed: 'is not valid JSON',
  unknown_shape: 'has a layout XTrace does not know',
};

/** Why plugins' skills are not all listed, as a sentence. */
export const pluginsUnreadText = (problem: GradingPluginProblem) =>
  `Plugin skills are not all listed: ${pluginFileText[problem.file]} ${pluginFailureText[problem.failure]}.`;

/** The text a search matches an installed skill against. */
export const installedSearchText = (skill: GradingInstalledSkill) =>
  [skill.name, skill.description, skillSourceText(skill.source), skill.folder]
    .join(' ')
    .toLowerCase();

/** What an Add press said, by the native answer. */
export function addText(answer: GradingSkillAdd, name: string) {
  switch (answer.state) {
    case 'added':
      return `${name} is on Scores now.`;
    case 'not_found':
      return `${name} is no longer installed there, so it was not added.`;
    case 'not_ready':
      return `${name} has no valid rubric now, so it was not added.`;
    case 'already_on_scores':
      return `A skill with the same id as ${name} is already on Scores.`;
    case 'id_clash':
      return `Another installed skill has the same id as ${name}; only one can be added.`;
    case 'not_configured':
      return 'Adding skills is not available in this copy of XTrace.';
    case 'failed':
      return `${name} could not be added. Try again.`;
  }
}

/** An Add press whose request itself failed. */
export const ADD_FAILED = (name: string) => `${name} could not be added. Try again.`;

/** What a Remove from Scores press said when it did not remove anything. */
export function removeText(answer: Exclude<GradingSkillRemove, { state: 'removed' }>) {
  switch (answer.state) {
    case 'built_in':
      return 'Built-in skills cannot be removed.';
    case 'not_found':
      return 'This skill is already off Scores.';
    case 'running':
      return 'This skill is grading a project now or waiting to. Wait for it to finish or cancel it, then remove it.';
    case 'not_configured':
      return 'Removing skills is not available in this copy of XTrace.';
    case 'failed':
      return 'The skill could not be removed. Try again.';
  }
}

/** The cost note under "Your skills". */
export const MAKE_RUBRIC_NOTE =
  'Make a rubric runs Claude Code in the background to write one for that skill; it uses about a minute of your Claude usage.';

/** A rubric being made, in words, with its elapsed time when known. */
export const makingText = (elapsed: number | undefined) =>
  elapsed === undefined ? 'Making a rubric…' : `Making a rubric · ${elapsedText(elapsed)}`;

/** Why no rubric was made, as the end of a sentence. */
export function rubricFailureText(failure: GradingRubricFailure) {
  switch (failure.reason) {
    case 'run':
      return failureText[failure.failure];
    case 'rubric_missing':
      return 'Claude wrote no rubric, even on a second try';
    case 'rubric_invalid':
      return `the rubric Claude wrote was not valid on a second try either: ${rubricProblemText[failure.rubric]}`;
    case 'refusal_unreadable':
      return "Claude's reason for not making one could not be read, even on a second try";
    case 'both_written':
      return 'Claude wrote both a rubric and a refusal, even on a second try';
    case 'id_unavailable':
      return 'other skills already use every variant of its id';
    case 'skill_changed':
      return 'the skill moved or got a rubric of its own meanwhile, so nothing was saved';
    case 'folder_not_writable':
      return 'its folder has its own rubric and XTrace cannot change files there';
    case 'save_failed':
      return 'the rubric could not be saved';
    case 'helper_unavailable':
      return 'the rubric maker that comes with XTrace could not be read';
    case 'internal_error':
      return 'XTrace hit an error while making it';
  }
}

/** A made rubric's criteria, as one short line. */
export const criteriaPreview = (criteria: readonly GradingRubricCriterion[]) =>
  criteria.map((criterion) => `${criterion.name} ${criterion.points}`).join(' · ');

/** What a Make a rubric press that did not start a job says. */
export function makeText(
  answer: Exclude<GradingRubricMake, { state: 'started' }>,
  name: string,
  busy: string | null,
) {
  switch (answer.state) {
    case 'busy':
      return (
        busy ??
        'Only one grading runs at a time, and making a rubric counts as one. Wait for the one going now to finish or cancel it.'
      );
    case 'not_found':
      return `${name} is no longer installed there, so no rubric was made.`;
    case 'already_ready':
      return `${name} already has a valid rubric.`;
    case 'not_offered':
      return `Scores already has a skill with the id of ${name}, so no rubric is made for it.`;
    case 'helper_unavailable':
      return 'The rubric maker that comes with XTrace could not be read, so no rubric was made.';
    case 'claude_unavailable':
      return 'Claude Code was not found on this Mac, so no rubric was made.';
    case 'not_configured':
      return 'Making rubrics is not available in this copy of XTrace.';
    case 'stopped':
      return 'XTrace is closing, so no rubric was made.';
    case 'failed':
      return `A rubric for ${name} could not be started. Try again.`;
  }
}

/** A Make a rubric press whose request itself failed. */
export const MAKE_FAILED = (name: string) =>
  `A rubric for ${name} could not be started. Try again.`;

/** The help line under the "Add from a link" field. */
export const LINK_HELP = 'Paste a public GitHub link to a skill or a repository of skills.';

/** What keeping does, under a preview. */
export const LINK_KEEP_NOTE =
  'Keeping copies a skill into ~/.agents/skills and links it in ~/.claude/skills when that name is free there. Nothing in it runs now. Once linked, it loads in your normal Claude Code sessions, outside any sandbox.';

/** Which bound a download passed, as the end of a sentence. */
const linkLimitText: Record<GradingLinkLimit, string> = {
  download: 'it is larger than XTrace downloads',
  entries: 'it has more files than XTrace unpacks',
  unpacked: 'it unpacks to more data than XTrace reads',
};

/** What a Look up that found nothing to preview says. */
export function linkLookupText(answer: Exclude<GradingLinkLookup, { state: 'found' }>) {
  switch (answer.state) {
    case 'not_github_link':
      return 'Only public GitHub links work for now.';
    case 'not_found':
      return 'GitHub has no public repository, branch, tag or commit by that name.';
    case 'path_not_found':
      return 'That folder is not in the repository at that version.';
    case 'no_skill':
      return 'No skill found at this link.';
    case 'rate_limited':
      return 'GitHub is not answering more look-ups from this network for now. Try again later.';
    case 'too_large':
      return `That repository can’t be looked up: ${linkLimitText[answer.limit]}. Link the skill’s own folder instead.`;
    case 'unsafe_archive':
      return 'GitHub’s download has a link out of the repository or a special file, so XTrace did not unpack it.';
    case 'bad_archive':
      return 'GitHub’s download could not be read. Try again.';
    case 'timed_out':
      return 'GitHub did not answer in time. Try again.';
    case 'cancelled':
      return 'Look-up cancelled.';
    case 'network_failed':
      return 'GitHub could not be reached or answered with an error. Try again.';
    case 'not_configured':
      return 'Adding from a link is not available in this copy of XTrace.';
    case 'failed':
      return 'The link could not be looked up. Try again.';
  }
}

/** Where a preview's skills come from: the repository and the commit. */
export const linkSourceText = (source: GradingLinkSource) =>
  `${source.repository} · ${source.short_commit}`;

/** Which folder and version a preview shows. */
export const linkWhereText = (source: GradingLinkSource) =>
  `${source.path === '' ? 'Top of the repository' : source.path} · ${
    source.reference === null ? 'default branch' : source.reference
  }`;

/** A found skill's size. */
export const linkSizeText = (files: number, bytes: number) =>
  `${plural(files, 'file')} · ${byteSize(bytes)}`;

/** A found skill's rubric, in words. */
export function linkRubricText(rubric: GradingInstalledRubric) {
  switch (rubric.state) {
    case 'ready':
      return `Has a rubric (${rubric.id}): it can go on Scores once kept`;
    case 'missing':
      return 'No rubric yet: XTrace can make one once it is kept';
    case 'problem':
      return `Its rubric has a problem: ${rubricProblemText[rubric.reason]}`;
  }
}

/** Why a found skill can't be kept, or nothing when it can. */
export function linkKeepStateText(state: GradingLinkKeepState) {
  switch (state.state) {
    case 'keepable':
      return null;
    case 'name_in_agents':
    case 'name_in_claude':
      return `${state.folder} already exists, so it can’t be kept`;
    case 'same_name_above':
      return `${state.path}, listed above, gets the same folder name; only that one can be kept`;
    case 'unusable_name':
      return 'Its name can’t be used as a folder name';
    case 'too_large':
      return 'It has more files or data than XTrace copies for a grading';
    case 'link_leaves_folder':
      return 'It has a link to a folder, to nothing or out of its folder';
    case 'instructions_unreadable':
      return 'Its SKILL.md can’t be read';
  }
}

/** What one chosen skill's keeping did, as a sentence. */
function keptLine(skill: GradingLinkKept) {
  const result = skill.result;
  switch (result.state) {
    case 'kept':
      switch (result.claude.state) {
        case 'linked':
          return `${skill.name} is in ${result.folder} and linked in ${result.claude.link}.`;
        case 'name_taken':
          return `${skill.name} is in ${result.folder}; ~/.claude/skills already has that name, so no link was made there.`;
        case 'no_claude_folder':
          return `${skill.name} is in ${result.folder}; there is no ~/.claude folder, so no link was made.`;
        case 'failed':
          return `${skill.name} is in ${result.folder}; its link in ~/.claude/skills could not be made.`;
      }
      break;
    case 'name_taken':
      return `${skill.name} was not kept: its name was taken in ~/.agents/skills meanwhile.`;
    case 'failed':
      return `${skill.name} could not be copied, so it was not kept.`;
  }
}

/** What a Keep press said, by the native answer. */
export function linkKeepText(answer: GradingLinkKeep) {
  switch (answer.state) {
    case 'done': {
      const kept = answer.skills.filter((skill) => skill.result.state === 'kept').length;
      return [
        ...answer.skills.map(keptLine),
        kept === 0
          ? ''
          : `${kept === 1 ? 'It is' : 'They are'} listed below with your other skills.`,
        kept > 0 && !answer.recorded ? 'XTrace could not save where it came from.' : '',
      ]
        .filter(Boolean)
        .join(' ');
    }
    case 'changed':
      return 'Something changed since the preview (a name is taken now), so nothing was kept. Check the list again.';
    case 'nothing_chosen':
      return 'Choose at least one skill to keep.';
    case 'expired':
      return 'This preview has expired. Look the link up again.';
    case 'agents_folder_unusable':
      return '~/.agents/skills is not a folder XTrace can use, so nothing was kept.';
    case 'not_configured':
      return 'Adding from a link is not available in this copy of XTrace.';
    case 'failed':
      return 'The skills could not be kept. Try again.';
  }
}

/** A Keep press whose request itself failed. */
export const KEEP_FAILED = 'The skills could not be kept. Try again.';
/** A Look up press whose request itself failed. */
export const LOOKUP_FAILED = 'The link could not be looked up. Try again.';
