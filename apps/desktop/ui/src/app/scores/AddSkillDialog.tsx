import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useData } from '../../data/DataProvider';
import type { GradingInstalledGroup } from '../../data/generated/GradingInstalledGroup';
import type { GradingInstalledLoaded } from '../../data/generated/GradingInstalledLoaded';
import type { GradingInstalledSkill } from '../../data/generated/GradingInstalledSkill';
import { Button } from '../../kit/Button';
import { EmptyState, LoadingRows } from '../../kit/EmptyState';
import { Modal, ModalClose } from '../../kit/Modal';
import { Search } from '../../kit/Search';
import { plural } from '../dashboard/present';
import { AddFromLink } from './AddFromLink';
import {
  ADD_FAILED,
  addText,
  criteriaPreview,
  installedSearchText,
  installedStateText,
  MAKE_FAILED,
  MAKE_RUBRIC_NOTE,
  makeText,
  makingText,
  oneAtATime,
  pluginsUnreadText,
  rubricFailureText,
  rubricOneAtATime,
  rubricProblemLine,
  skillSourceText,
  useElapsed,
  useGradingOverview,
  useInstalledSkills,
  useRefreshInstalled,
  useRubricProgress,
} from './scores';

/**
 * The last card on Scores: it opens "Add a grading skill": "Add from a link" at the
 * top (`AddFromLink`), then the skills installed on this Mac under three
 * steps: Ready to add, Your skills, then two groups
 * folded at the bottom (Can't be scored, Already on Scores). Which group a
 * skill is in, the counts, their order, whether one can be added, whether a
 * rubric can be made for it and how its latest rubric-making job stands are
 * the native answer's (`GradingInstalledSkills`); this only shows them,
 * filters them by the search text, folds and unfolds the two groups, and
 * sends Add, Make a rubric or Cancel with the location or job the answer
 * named.
 */
export function AddSkillCard() {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const refresh = useRefreshInstalled();
  return (
    <li className="xt-score-add">
      <button
        ref={trigger}
        type="button"
        // WebKit skips a plain button on Tab without an explicit tab stop.
        tabIndex={0}
        className="xt-score-add-button"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => {
          // Read fresh, so a skill installed meanwhile shows up.
          refresh();
          setOpen(true);
        }}
      >
        <span className="xt-score-add-plus" aria-hidden="true">
          +
        </span>
        <span className="xt-score-add-label">Add a grading skill</span>
      </button>
      <AddSkillDialog open={open} onOpenChange={setOpen} returnFocusRef={trigger} />
    </li>
  );
}

function AddSkillDialog({
  open,
  onOpenChange,
  returnFocusRef,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef: RefObject<HTMLButtonElement | null>;
}) {
  const title = useId();
  const installed = useInstalledSkills(open);
  const refresh = useRefreshInstalled();
  const progress = useRubricProgress(open);
  const [query, setQuery] = useState('');
  const loaded = installed.data?.state === 'loaded' ? installed.data : null;
  return (
    <Modal
      open={open}
      onOpenChange={(next) => {
        // Each opening starts with an empty search.
        if (!next) setQuery('');
        onOpenChange(next);
      }}
      returnFocusRef={returnFocusRef}
      aria-labelledby={title}
      className="xt-scores-modal xt-score-add-modal"
    >
      {/* Close comes first, so opening by keyboard lands on it. */}
      <header className="xt-scores-modal-header">
        <h2 id={title}>Add a grading skill</h2>
        <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
          Close
        </ModalClose>
      </header>
      <AddFromLink />
      <ol className="xt-score-add-steps" aria-label="How a skill gets on Scores">
        {STEPS.map(([title, detail], index) => (
          <li key={title}>
            <span className="xt-score-add-step-number" aria-hidden="true">
              {index + 1}
            </span>
            <span className="xt-score-add-step-text">
              <strong>{title}</strong>
              <span>{detail}</span>
            </span>
          </li>
        ))}
      </ol>
      {installed.data?.state === 'not_configured' ? (
        <p className="xt-scores-detail" role="status">
          This copy of XTrace runs on sample data, so it reads no installed skills.
        </p>
      ) : installed.isError || installed.data?.state === 'failed' ? (
        <div className="xt-scores-modal-error" role="alert">
          <p>Installed skills could not be read.</p>
          <Button variant="outline" height={28} onClick={refresh}>
            Try again
          </Button>
        </div>
      ) : !loaded ? (
        <LoadingRows rows={3} />
      ) : (
        <>
          <Search
            label="Search installed skills"
            placeholder="Search by name, description or source"
            value={query}
            onValueChange={setQuery}
          />
          <Groups
            loaded={loaded}
            updatedAt={installed.dataUpdatedAt}
            reading={installed.isFetching}
            query={query.trim().toLowerCase()}
            progress={progress}
          />
        </>
      )}
    </Modal>
  );
}

/** The three steps at the top of the dialog. */
const STEPS = [
  ['Pick a skill', 'One you already use'],
  ['Make a rubric', 'XTrace writes it, about 1 minute'],
  ['Add it', 'It gets its own score card'],
] as const;

/** The two groups folded at the bottom, closed at each opening. */
const FOLDED = [
  { group: 'cant_score', title: 'Can’t be scored' },
  { group: 'on_scores', title: 'Already on Scores' },
] as const;

/**
 * An Add being answered (`until` null), or answered and waiting for the
 * installed list to be read again: until that row is no longer addable, or
 * until `until` reads have finished (one more when a read was already going
 * at the answer, since that read may predate it).
 */
type Adding = { location: string; until: number | null };

/**
 * A Make a rubric (or Cancel) press, held like an Add (`Adding`), or until
 * the row's rubric job is no longer the one it showed when pressed.
 */
type Pressed = Adding & { job: string };

/** A row's rubric job, as one comparable value. */
const jobKey = (skill: GradingInstalledSkill) =>
  skill.rubric_job ? `${skill.rubric_job.state} ${skill.rubric_job.job_id}` : '';

/** The groups in one list that scrolls inside the dialog. */
function Groups({
  loaded,
  updatedAt,
  reading,
  query,
  progress,
}: {
  loaded: GradingInstalledLoaded;
  /** When the installed list was last read; each finished read moves it. */
  updatedAt: number;
  /** Whether a read of it is going now. */
  reading: boolean;
  query: string;
  /** Elapsed seconds the rubric-making progress reported, by job ID. */
  progress: Record<string, number>;
}) {
  const { source } = useData();
  const refresh = useRefreshInstalled();
  const overview = useGradingOverview();
  const [message, setMessage] = useState<string | null>(null);
  const [adding, setAdding] = useState<Adding | null>(null);
  const [pressed, setPressed] = useState<Pressed | null>(null);
  // A Try again started from Can't be scored: the row moves to Your skills,
  // and keyboard focus follows it there.
  const [follow, setFollow] = useState<{ location: string; name: string } | null>(null);
  // The status line takes focus once it shows the text set for that.
  const [focusStatus, setFocusStatus] = useState(false);
  const list = useRef<HTMLDivElement>(null);
  const status = useRef<HTMLParagraphElement>(null);
  // How many reads have finished while this list is shown.
  const [reads, setReads] = useState(0);
  const lastRead = useRef(updatedAt);
  useEffect(() => {
    if (updatedAt === lastRead.current) return;
    lastRead.current = updatedAt;
    setReads((count) => count + 1);
  }, [updatedAt]);
  const latest = useRef({ reads, reading });
  useEffect(() => {
    latest.current = { reads, reading };
  });
  // Add stays busy until the list read after its answer shows the row:
  // there is no moment where an added row shows Add again.
  useEffect(() => {
    if (!adding || adding.until === null) return;
    const row = loaded.skills.find((skill) => skill.location === adding.location);
    if (!row || row.scores.state !== 'addable' || reads >= adding.until) setAdding(null);
  }, [adding, loaded, reads]);
  // Make a rubric and Cancel stay busy until the list read after their
  // answer has been shown, so a row never offers the same press twice.
  useEffect(() => {
    if (!pressed) return;
    const row = loaded.skills.find((skill) => skill.location === pressed.location);
    if (!row || jobKey(row) !== pressed.job || (pressed.until !== null && reads >= pressed.until))
      setPressed(null);
  }, [pressed, loaded, reads]);
  const held = (skill: GradingInstalledSkill) => {
    const now = latest.current;
    setPressed({
      location: skill.location,
      until: now.reads + (now.reading ? 2 : 1),
      job: jobKey(skill),
    });
    refresh();
  };
  // The one grading or rubric job going now, as the Scores page names it.
  const going = overview.data?.state === 'loaded' ? overview.data : null;
  const busy = going?.running
    ? oneAtATime(going.running)
    : going?.making_rubric
      ? rubricOneAtATime(going.making_rubric)
      : null;
  const make = async (skill: GradingInstalledSkill) => {
    if (!source.grading) return;
    // Another grading or rubric is going: say so, and start nothing.
    if (busy && going?.making_rubric?.location !== skill.location) return setMessage(busy);
    if (pressed) return;
    setPressed({ location: skill.location, until: null, job: jobKey(skill) });
    setMessage(null);
    let text: string | null;
    try {
      const answer = await source.grading.makeRubric(skill.location);
      text = answer.state === 'started' ? null : makeText(answer, skill.name, busy);
      if (answer.state === 'started' && skill.group === 'cant_score')
        setFollow({ location: skill.location, name: skill.name });
    } catch {
      text = MAKE_FAILED(skill.name);
    }
    setMessage(text);
    held(skill);
  };
  // Once a read shows the tried-again row out of Can't be scored, focus its
  // button; when it is not shown (gone, or in a folded group), the status
  // line. While its press is still held there, wait.
  useEffect(() => {
    if (!follow) return;
    const row = loaded.skills.find((skill) => skill.location === follow.location);
    if (row?.group === 'cant_score') {
      if (!pressed) setFollow(null);
      return;
    }
    setFollow(null);
    const shown = [
      ...(list.current?.querySelectorAll<HTMLElement>('.xt-score-add-row') ?? []),
    ].find((item) => item.dataset.location === follow.location);
    const button = shown?.querySelector<HTMLElement>('.xt-score-add-action button');
    if (button) return button.focus();
    setMessage(`Making a rubric for ${follow.name} again.`);
    setFocusStatus(true);
  }, [follow, loaded, pressed]);
  useEffect(() => {
    if (!focusStatus) return;
    setFocusStatus(false);
    status.current?.focus();
  }, [focusStatus]);
  const cancel = async (skill: GradingInstalledSkill, jobId: string) => {
    if (pressed || !source.grading) return;
    setPressed({ location: skill.location, until: null, job: jobKey(skill) });
    let stopping: boolean;
    try {
      stopping = await source.grading.cancel(jobId);
    } catch {
      stopping = false;
    }
    setMessage(stopping ? null : `Making a rubric for ${skill.name} had already ended.`);
    held(skill);
  };
  const add = async (skill: GradingInstalledSkill) => {
    if (adding || !source.grading) return;
    setAdding({ location: skill.location, until: null });
    setMessage(null);
    let text: string;
    try {
      text = addText(await source.grading.addSkill(skill.location), skill.name);
    } catch {
      text = ADD_FAILED(skill.name);
    }
    setMessage(text);
    const now = latest.current;
    setAdding({ location: skill.location, until: now.reads + (now.reading ? 2 : 1) });
    refresh();
  };
  const shown = loaded.skills.filter(
    (skill) => query === '' || installedSearchText(skill).includes(query),
  );
  const inGroup = (group: GradingInstalledGroup) => shown.filter((skill) => skill.group === group);
  const count = (matches: number, total: number) =>
    query === '' ? total : `${matches} of ${total}`;
  const rows = (skills: GradingInstalledSkill[], title: string) => (
    <ul role="list" aria-label={title}>
      {skills.map((skill) => (
        <SkillRow
          key={skill.location}
          skill={skill}
          adding={adding?.location === skill.location}
          pressed={pressed?.location === skill.location}
          reported={
            skill.rubric_job?.state === 'running' ? progress[skill.rubric_job.job_id] : undefined
          }
          onAdd={() => void add(skill)}
          onMake={() => void make(skill)}
          onCancel={(jobId) => void cancel(skill, jobId)}
        />
      ))}
    </ul>
  );
  const ready = inGroup('ready');
  const yours = inGroup('yours');
  return (
    <>
      {/* Focusable by script only: focus lands here when a row it followed
          is not shown. */}
      <p ref={status} className="xt-scores-message" role="status" tabIndex={-1}>
        {message}
      </p>
      {loaded.skills.length === 0 ? (
        <EmptyState message="No installed skills were found for Claude Code, Codex or shared agents." />
      ) : (
        <div
          ref={list}
          className="xt-score-add-list"
          role="region"
          aria-label="Installed skills"
          tabIndex={0}
        >
          <section className="xt-score-add-group" data-group="ready">
            <h3>
              Ready to add
              <span className="xt-score-add-count">{count(ready.length, loaded.ready)}</span>
            </h3>
            {loaded.ready === 0 ? (
              <p className="xt-score-add-none">Make a rubric for one of your skills below.</p>
            ) : ready.length === 0 ? (
              <p className="xt-score-add-none">None match</p>
            ) : (
              rows(ready, 'Ready to add')
            )}
          </section>
          <section className="xt-score-add-group" data-group="yours">
            <h3>
              Your skills
              <span className="xt-score-add-count">{count(yours.length, loaded.yours)}</span>
            </h3>
            {loaded.yours > 0 && <p className="xt-score-add-soon">{MAKE_RUBRIC_NOTE}</p>}
            {loaded.yours === 0 ? (
              <p className="xt-score-add-none">
                {loaded.cant_score > 0
                  ? 'No other skill needs a rubric.'
                  : 'Every skill here has a rubric or is on Scores.'}
              </p>
            ) : yours.length === 0 ? (
              <p className="xt-score-add-none">None match</p>
            ) : (
              rows(yours, 'Your skills')
            )}
          </section>
          {FOLDED.map(({ group, title }) => {
            const total = group === 'cant_score' ? loaded.cant_score : loaded.on_scores;
            const skills = inGroup(group);
            return (
              total > 0 && (
                <FoldedGroup
                  key={group}
                  group={group}
                  title={title}
                  count={count(skills.length, total)}
                >
                  {skills.length === 0 ? (
                    <p className="xt-score-add-none">None match</p>
                  ) : (
                    rows(skills, title)
                  )}
                </FoldedGroup>
              )
            );
          })}
        </div>
      )}
      {(loaded.skipped > 0 || loaded.incomplete) && (
        <p className="xt-scores-note">
          {loaded.skipped > 0 &&
            `${plural(loaded.skipped, 'skill folder')} could not be read (a link, or a SKILL.md that is too large or unreadable). `}
          {loaded.incomplete &&
            (loaded.plugins_unread
              ? pluginsUnreadText(loaded.plugins_unread)
              : 'Some folders hold more than XTrace reads, so not every skill is listed.')}
        </p>
      )}
    </>
  );
}

/**
 * A group folded at the bottom of the list: a button that shows its title
 * and count (the match count while searching) and opens or closes its rows.
 * Closed at each opening of the dialog.
 */
function FoldedGroup({
  group,
  title,
  count,
  children,
}: {
  group: GradingInstalledGroup;
  title: string;
  count: number | string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const panel = useId();
  return (
    <section className="xt-score-add-group" data-group={group} data-folded={!open}>
      <h3 className="xt-score-add-fold-heading">
        <Button
          variant="ghost"
          height={28}
          className="xt-score-add-fold"
          aria-expanded={open}
          aria-controls={panel}
          onClick={() => setOpen((shown) => !shown)}
        >
          <span className="xt-score-add-chevron" data-open={open} aria-hidden="true">
            ›
          </span>
          {title}
          <span className="xt-score-add-count">{count}</span>
        </Button>
      </h3>
      <div id={panel} hidden={!open}>
        {open && children}
      </div>
    </section>
  );
}

function SkillRow({
  skill,
  adding,
  pressed,
  reported,
  onAdd,
  onMake,
  onCancel,
}: {
  skill: GradingInstalledSkill;
  adding: boolean;
  /** A Make a rubric or Cancel press of this row is being answered. */
  pressed: boolean;
  /** The elapsed seconds its running rubric job last reported. */
  reported: number | undefined;
  onAdd: () => void;
  onMake: () => void;
  onCancel: (jobId: string) => void;
}) {
  const state = installedStateText(skill.scores);
  const job = skill.rubric_job;
  const elapsed = useElapsed(job?.state === 'running' ? job.started_at_ms : undefined, reported);
  const action = skill.rubric_action;
  // A job that ended without a rubric is offered again as "Try again".
  const retry = job?.state === 'failed' || job?.state === 'refused' || job?.state === 'cancelled';
  return (
    <li
      className="xt-score-add-row"
      data-location={skill.location}
      data-state={skill.scores.state}
      data-job={job?.state}
      aria-busy={job?.state === 'running' || undefined}
    >
      <span className="xt-score-add-name" title={skill.folder}>
        {skill.name}
      </span>
      <span className="xt-score-add-source">{skillSourceText(skill.source)}</span>
      <span className="xt-score-add-description" title={skill.description || undefined}>
        {skill.description || 'No description'}
      </span>
      {skill.rubric.state === 'problem' && (
        <span className="xt-score-add-reason">{rubricProblemLine(skill.rubric.reason)}</span>
      )}
      {job?.state === 'running' && (
        // Not a live region: the time moves every second.
        <span className="xt-score-add-reason" data-tone="live">
          <i className="xt-scores-live" aria-hidden="true" />
          {makingText(elapsed)}
        </span>
      )}
      {job?.state === 'failed' && (
        <span className="xt-score-add-reason" data-wrap="">
          {`No rubric was made: ${rubricFailureText(job.failure)}.`}
        </span>
      )}
      {job?.state === 'refused' && (
        // Under "Can't be scored": Claude's reason is the row's line.
        <span className="xt-score-add-reason" data-wrap="">
          {job.reason}
        </span>
      )}
      {job?.state === 'cancelled' && (
        <span className="xt-score-add-reason" data-tone="muted">
          Making a rubric was cancelled.
        </span>
      )}
      {job?.state === 'made' && (
        <span
          className="xt-score-add-reason xt-score-add-criteria"
          data-tone="made"
          title={criteriaPreview(job.criteria)}
        >
          {`Rubric made${job.kept_by_xtrace ? ' and kept by XTrace' : ''}: ${criteriaPreview(job.criteria)}`}
        </span>
      )}
      {skill.rubric_kept_by_xtrace && job?.state !== 'made' && (
        <span className="xt-score-add-reason" data-tone="muted">
          Rubric kept by XTrace
        </span>
      )}
      <span className="xt-score-add-action">
        {state && (
          <span
            className="xt-score-add-state"
            title={
              skill.scores.state === 'id_on_scores'
                ? skill.scores.location
                : skill.scores.state === 'id_clash'
                  ? skill.scores.folder
                  : undefined
            }
          >
            {state}
          </span>
        )}
        {skill.scores.state === 'addable' ? (
          <Button
            variant="primary"
            height={28}
            aria-label={`Add ${skill.name}`}
            aria-disabled={adding || undefined}
            onClick={onAdd}
          >
            {adding ? 'Adding…' : 'Add'}
          </Button>
        ) : job?.state === 'running' ? (
          <Button
            variant="outline"
            height={28}
            aria-label={`Cancel making a rubric for ${skill.name}`}
            aria-disabled={pressed || undefined}
            onClick={() => onCancel(job.job_id)}
          >
            {pressed ? 'Stopping…' : 'Cancel'}
          </Button>
        ) : (
          action && (
            <Button
              variant="outline"
              height={28}
              aria-label={`${
                retry
                  ? 'Try making a rubric again for'
                  : action === 'remake'
                    ? 'Remake the rubric of'
                    : 'Make a rubric for'
              } ${skill.name}`}
              aria-disabled={pressed || undefined}
              onClick={onMake}
            >
              {pressed
                ? 'Starting…'
                : retry
                  ? 'Try again'
                  : action === 'remake'
                    ? 'Remake rubric'
                    : 'Make a rubric'}
            </Button>
          )
        )}
      </span>
    </li>
  );
}
