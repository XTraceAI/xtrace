import { useId, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { GradingProjectRow } from '../../data/generated/GradingProjectRow';
import type { GradingRubricNow } from '../../data/generated/GradingRubricNow';
import type { GradingRunningNow } from '../../data/generated/GradingRunningNow';
import type { GradingSkillOverview } from '../../data/generated/GradingSkillOverview';
import type { GradingTopInsight } from '../../data/generated/GradingTopInsight';
import type { ProjectEntry } from '../../data/generated/ProjectEntry';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { DataTable, type Column } from '../../kit/DataTable';
import { EmptyState, LoadingRows } from '../../kit/EmptyState';
import { Icon } from '../../kit/icons';
import { Modal, ModalClose } from '../../kit/Modal';
import { SectionCard } from '../../kit/SectionCard';
import { plural } from '../dashboard/present';
import { AddSkillCard } from './AddSkillDialog';
import { RunDetailsDialog, type DetailsTarget } from './RunDetailsDialog';
import {
  attemptText,
  cancelText,
  elapsedText,
  oneAtATime,
  removeText,
  rubricOneAtATime,
  runningElsewhereText,
  scoreText,
  severity,
  skillProblemText,
  sourceNames,
  START_FAILED,
  STARTING,
  startText,
  timeText,
  useElapsed,
  useGradingOverview,
  useGradingProgress,
  useRefreshOverview,
  type Loaded,
} from './scores';
import '../../styles/scores.css';

/**
 * `/scores`: one card per grading skill (built in, then added) with its
 * average over the listed projects, how many are graded, and its top
 * findings; the "Your projects" list one press away, and the "Add a grading
 * skill" card last. Grading itself happens on a skill's page.
 * Every value is the native overview's (`GradingOverview`), only formatted
 * here.
 */
export function ScoresPage() {
  const overview = useGradingOverview();
  const loaded = overview.data?.state === 'loaded' ? overview.data : null;
  const progress = useGradingProgress(loaded?.running?.run_id);
  // A failed re-read keeps the list read last, so an open pop-up stays open.
  const projects = useLastLoaded(loaded?.projects);
  const readFailed = overview.isError || overview.data?.state === 'failed';
  const [message, setMessage] = useState<string | null>(null);
  const cancel = useCancel(setMessage);
  return (
    <section className="xt-scores" data-view="overview">
      <div className="xt-scores-heading">
        <div>
          <h1>Scores</h1>
          <p>Each skill grades your projects from 0 to 100 and says what to fix first.</p>
        </div>
        {projects && <YourProjects projects={projects} failed={readFailed} />}
      </div>
      <Answer overview={overview}>
        {(loaded) => (
          <>
            <p className="xt-scores-message" role="status">
              {message}
            </p>
            {/* No card has a row: a run no row shows is cancelled here. */}
            {loaded.running && !loaded.skills.some((skill) => shownOnARow(skill.rows)) && (
              <RunningElsewhere
                running={loaded.running}
                reported={progress[loaded.running.run_id]}
                onCancel={cancel}
              />
            )}
            {loaded.making_rubric && (
              <MakingRubric making={loaded.making_rubric} onCancel={cancel} />
            )}
            <ul className="xt-scores-grid" role="list" aria-label="Grading skills">
              {loaded.skills.map((skill) => (
                <SkillCard
                  key={skill.skill.id}
                  skill={skill}
                  startedAtMs={skill.running ? loaded.running?.started_at_ms : undefined}
                  reported={
                    skill.running && loaded.running ? progress[loaded.running.run_id] : undefined
                  }
                />
              ))}
              <AddSkillCard />
            </ul>
          </>
        )}
      </Answer>
    </section>
  );
}

/** The value when there is one, otherwise the last one there was. */
function useLastLoaded<T>(value: T | undefined): T | undefined {
  const [last, setLast] = useState(value);
  if (value !== undefined && value !== last) setLast(value);
  return value ?? last;
}

/** Whether one of these rows is the run going now (the overview's own flag). */
const shownOnARow = (rows: readonly GradingProjectRow[]) => rows.some((row) => row.running);

/** Cancel a run, saying whether it was still going, then read again. */
function useCancel(setMessage: (text: string) => void) {
  const { source } = useData();
  const refresh = useRefreshOverview();
  return async (runId: string) => {
    if (!source.grading) return;
    try {
      setMessage(cancelText(await source.grading.cancel(runId)));
    } catch {
      setMessage(cancelText(false));
    } finally {
      refresh();
    }
  };
}

/**
 * The run going now when no row on the page shows it (its project is no
 * longer listed, or it grades with another skill), with its own Cancel.
 */
function RunningElsewhere({
  running,
  reported,
  onCancel,
}: {
  running: GradingRunningNow;
  reported: number | undefined;
  onCancel: (runId: string) => Promise<void>;
}) {
  const elapsed = useElapsed(running.started_at_ms, reported);
  const text = runningElsewhereText(running);
  return (
    <div className="xt-scores-notice xt-scores-running" data-testid="running-elsewhere">
      <div>
        <p className="xt-scores-lead">
          <i className="xt-scores-live" aria-hidden="true" />
          {text.lead}
          {elapsed !== undefined && ` · ${elapsedText(elapsed)}`}
        </p>
        {text.detail && <p className="xt-scores-detail">{text.detail}</p>}
      </div>
      <Button
        variant="outline"
        height={28}
        aria-label={`Cancel grading ${running.project_name ?? running.project_path}`}
        onClick={() => void onCancel(running.run_id)}
      >
        Cancel
      </Button>
    </div>
  );
}

/**
 * A rubric being made for an installed skill: it holds the one grading slot,
 * so it is named, with its own Cancel, wherever a grading could be started.
 */
function MakingRubric({
  making,
  onCancel,
}: {
  making: GradingRubricNow;
  onCancel: (runId: string) => Promise<void>;
}) {
  const elapsed = useElapsed(making.started_at_ms, undefined);
  return (
    <div className="xt-scores-notice xt-scores-running" data-testid="making-rubric">
      <div>
        <p className="xt-scores-lead">
          <i className="xt-scores-live" aria-hidden="true" />
          {`A rubric for ${making.name} is being made`}
          {elapsed !== undefined && ` · ${elapsedText(elapsed)}`}
        </p>
        <p className="xt-scores-detail">Nothing can be graded until it is done.</p>
      </div>
      <Button
        variant="outline"
        height={28}
        aria-label={`Cancel making a rubric for ${making.name}`}
        onClick={() => void onCancel(making.job_id)}
      >
        Cancel
      </Button>
    </div>
  );
}

/** The overview's state: its answer when loaded, or why there is none. */
function Answer({
  overview,
  children,
}: {
  overview: ReturnType<typeof useGradingOverview>;
  children: (loaded: Loaded) => ReactNode;
}) {
  const refresh = useRefreshOverview();
  const { source } = useData();
  if (!source.grading || overview.data?.state === 'not_configured')
    return (
      <Notice lead="Scores are not available here">
        This copy of XTrace runs on sample data, so it has no grading skills and reads no projects.
      </Notice>
    );
  if (overview.isError || overview.data?.state === 'failed')
    return (
      <Notice lead="Scores could not be read">
        <Button variant="outline" height={28} onClick={refresh}>
          Try again
        </Button>
      </Notice>
    );
  if (!overview.data) return <LoadingRows rows={3} />;
  return <>{children(overview.data)}</>;
}

function Notice({ lead, children }: { lead: string; children: ReactNode }) {
  return (
    <div className="xt-scores-notice" role="status">
      <p className="xt-scores-lead">{lead}</p>
      <div className="xt-scores-detail">{children}</div>
    </div>
  );
}

function SkillCard({
  skill,
  startedAtMs,
  reported,
}: {
  skill: GradingSkillOverview;
  startedAtMs: number | undefined;
  reported: number | undefined;
}) {
  const name = useId();
  const elapsed = useElapsed(startedAtMs, reported);
  const none = skill.graded === 0;
  return (
    <li className="xt-score-card" aria-labelledby={name} data-running={skill.running || undefined}>
      <div className="xt-score-card-top">
        <h2 id={name}>
          {/* The whole card opens the skill's page; the link is its name. */}
          <Link className="xt-score-card-link" to={`/scores/${encodeURIComponent(skill.skill.id)}`}>
            {skill.skill.name}
          </Link>
        </h2>
        {skill.running && (
          <StatePill tone="accent" height={20} live>
            {elapsed === undefined ? 'Grading now' : `Grading · ${elapsedText(elapsed)}`}
          </StatePill>
        )}
      </div>
      <p className="xt-score-card-value">
        <span className="xt-score-big" data-empty={skill.average === null || undefined}>
          {scoreText(skill.average)}
        </span>
        <span className="xt-score-card-count">
          {none ? 'Not graded yet' : `${skill.graded} of ${plural(skill.total, 'project')} graded`}
        </span>
      </p>
      {skill.average !== null && <span className="sr-only">Average score {skill.average}</span>}
      {skill.skill.problem && (
        <p className="xt-score-card-problem">
          Can’t grade now. {skillProblemText(skill.skill.problem)}
        </p>
      )}
      {skill.top_insights.length > 0 && (
        <ul
          className="xt-score-insights"
          role="list"
          aria-label={`Top findings, ${skill.skill.name}`}
        >
          {skill.top_insights.map((insight) => (
            <InsightLine key={insight.title} insight={insight} />
          ))}
        </ul>
      )}
    </li>
  );
}

function InsightLine({ insight }: { insight: GradingTopInsight }) {
  const weight = severity[insight.severity];
  return (
    <li className="xt-score-insight">
      <span
        className="xt-score-insight-icon xt-control-tone"
        data-tone={weight.tone}
        role="img"
        aria-label={weight.label}
        title={weight.label}
      >
        <Icon name={insight.severity === 'pass' ? 'check' : 'info'} size={12} />
      </span>
      <span className="xt-score-insight-title" title={insight.title}>
        {insight.title}
      </span>
      <span className="xt-score-insight-count">{plural(insight.projects, 'project')}</span>
    </li>
  );
}

/** Every listed project and the apps it was found in; it only lists. */
function YourProjects({
  projects,
  failed,
}: {
  projects: readonly ProjectEntry[];
  failed: boolean;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const title = useId();
  const refresh = useRefreshOverview();
  return (
    <>
      <Button
        ref={trigger}
        variant="outline"
        height={28}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => {
          // The list is read fresh, so an app's new project shows up here.
          refresh();
          setOpen(true);
        }}
      >
        Your projects · {projects.length}
      </Button>
      <Modal
        open={open}
        onOpenChange={setOpen}
        returnFocusRef={trigger}
        aria-labelledby={title}
        className="xt-scores-modal"
      >
        {/* Close comes first, so opening by keyboard lands on it. */}
        <header className="xt-scores-modal-header">
          <h2 id={title}>Your projects</h2>
          <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
            Close
          </ModalClose>
        </header>
        <p className="xt-scores-note">
          Found in the coding apps’ own saved settings. Grade a project on a skill’s page.
        </p>
        {failed && (
          <p className="xt-scores-modal-error" role="alert">
            The project list could not be read again. This is the list read last.
          </p>
        )}
        {projects.length === 0 ? (
          <EmptyState message="No projects found in Claude Code, the Claude app, Codex or Cursor." />
        ) : (
          <div
            className="xt-scores-project-list"
            role="region"
            aria-label="Project list"
            tabIndex={0}
          >
            <ul role="list" aria-label="Projects">
              {projects.map((project) => (
                <li key={project.path} className="xt-scores-project">
                  <span className="xt-scores-project-name">{project.name}</span>
                  <span className="xt-scores-project-path" title={project.path}>
                    {project.path}
                  </span>
                  <span className="xt-scores-project-apps">
                    {project.sources.map((source) => sourceNames[source]).join(' · ')}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </Modal>
    </>
  );
}

/**
 * `/scores/:skillId`: one skill's score for each listed project, with Grade
 * and, on the running row, Cancel. One grading runs at a time: while one
 * runs, another row's Grade explains that instead of being disabled. A
 * shown score opens the details of the run behind it.
 */
export function SkillScoresPage() {
  const { skillId = '' } = useParams();
  const overview = useGradingOverview();
  const loaded = overview.data?.state === 'loaded' ? overview.data : null;
  const progress = useGradingProgress(loaded?.running?.run_id);
  return (
    <section className="xt-scores" data-view="skill">
      <Link className="xt-scores-back" to="/scores" aria-label="Back to Scores">
        Scores ›
      </Link>
      {/* Until a skill on Scores is known, the page is named for what it shows. */}
      {!loaded?.skills.some((entry) => entry.skill.id === skillId) && (
        <div className="xt-scores-heading">
          <div>
            <h1>Skill scores</h1>
          </div>
        </div>
      )}
      <Answer overview={overview}>
        {(loaded) => {
          const skill = loaded.skills.find((entry) => entry.skill.id === skillId);
          return skill ? (
            <SkillDetail loaded={loaded} skill={skill} progress={progress} />
          ) : (
            <Notice lead="This skill is not on Scores">Choose a skill on the Scores page.</Notice>
          );
        }}
      </Answer>
    </section>
  );
}

function SkillDetail({
  loaded,
  skill,
  progress,
}: {
  loaded: Loaded;
  skill: GradingSkillOverview;
  progress: Record<string, number>;
}) {
  const { source } = useData();
  const refresh = useRefreshOverview();
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  // Read synchronously, so a second press in the same moment is seen too.
  const starting = useRef(false);
  const cancel = useCancel(setMessage);
  const problem = skill.skill.problem;
  const [details, setDetails] = useState<DetailsTarget | null>(null);
  const detailsTrigger = useRef<HTMLElement | null>(null);
  const grade = async (row: GradingProjectRow) => {
    // A skill shown as unable to grade still asks: it may be fixed by now,
    // and the native answer says whether it can (`skill_unavailable`).
    // A start is being answered: say so, and start nothing more.
    if (starting.current) return setMessage(STARTING);
    // Another run, or a rubric being made, is going: say so, and start nothing.
    if (loaded.running) return setMessage(oneAtATime(loaded.running));
    if (loaded.making_rubric) return setMessage(rubricOneAtATime(loaded.making_rubric));
    if (!source.grading) return;
    starting.current = true;
    setPending(true);
    setMessage(null);
    try {
      const answer = await source.grading.start(row.path, skill.skill.id);
      setMessage(answer.state === 'started' ? null : startText(answer, row.name));
    } catch {
      setMessage(START_FAILED);
    } finally {
      starting.current = false;
      setPending(false);
      refresh();
    }
  };
  const columns: readonly Column<GradingProjectRow>[] = [
    {
      key: 'project',
      header: 'Project',
      width: 'minmax(0, 1fr)',
      render: (row) => (
        <span className="xt-title-cell xt-scores-row-project">
          <span title={row.name}>{row.name}</span>
          <small title={row.path}>{row.path}</small>
        </span>
      ),
    },
    {
      key: 'score',
      header: 'Score',
      width: '84px',
      align: 'right',
      render: (row) =>
        row.shown ? (
          <button
            type="button"
            // WebKit skips a plain button on Tab without an explicit tab stop.
            tabIndex={0}
            className="xt-scores-score xt-scores-score-button"
            aria-haspopup="dialog"
            // Starts with the number shown, so a spoken "75" finds it too.
            aria-label={`${row.shown.score}, details for ${row.name}`}
            title="Show how this score was given"
            onClick={(event) => {
              detailsTrigger.current = event.currentTarget;
              if (row.shown)
                setDetails({ runId: row.shown.run_id, project: row.name, path: row.path });
            }}
          >
            {row.shown.score}
          </button>
        ) : (
          <span className="xt-scores-muted">not graded</span>
        ),
    },
    {
      key: 'graded',
      header: 'Graded',
      width: '150px',
      render: (row) =>
        row.shown ? (
          <time
            className="xt-scores-muted"
            dateTime={new Date(row.shown.graded_at_ms).toISOString()}
          >
            {timeText(row.shown.graded_at_ms)}
          </time>
        ) : (
          <span className="xt-scores-muted">—</span>
        ),
    },
    {
      key: 'state',
      header: 'State',
      width: 'minmax(0, 1.5fr)',
      render: (row) => <RowState row={row} progress={progress} />,
    },
    {
      key: 'action',
      header: <span className="sr-only">Action</span>,
      width: '116px',
      align: 'right',
      render: (row) => (
        <span className="xt-scores-actions">
          {row.attempt?.state === 'running' ? (
            <Button
              variant="outline"
              height={28}
              aria-label={`Cancel grading ${row.name}`}
              onClick={() => row.attempt?.state === 'running' && void cancel(row.attempt.run_id)}
            >
              Cancel
            </Button>
          ) : (
            <Button
              variant={row.shown ? 'outline' : 'primary'}
              height={28}
              aria-label={`${row.shown ? 'Grade again' : 'Grade'} ${row.name}`}
              aria-disabled={pending || undefined}
              onClick={() => void grade(row)}
            >
              {row.shown ? 'Grade again' : 'Grade'}
            </Button>
          )}
        </span>
      ),
    },
  ];
  return (
    <>
      <div className="xt-scores-heading">
        <div>
          <h1>{skill.skill.name}</h1>
          <p>{skill.skill.description}</p>
        </div>
        <p className="xt-scores-average">
          <span className="xt-score-big" data-empty={skill.average === null || undefined}>
            {scoreText(skill.average)}
          </span>
          <span className="xt-score-card-count">
            {skill.graded === 0
              ? 'Not graded yet'
              : `average · ${skill.graded} of ${plural(skill.total, 'project')} graded`}
          </span>
        </p>
      </div>
      {skill.skill.origin.kind === 'added' && (
        <AddedFrom
          skillId={skill.skill.id}
          folder={skill.skill.origin.folder}
          plugin={skill.skill.origin.plugin}
          setMessage={setMessage}
        />
      )}
      {problem && (
        <Notice lead="This skill can’t grade right now">
          {skillProblemText(problem)} Its scores stay. Fix the skill, or remove it from Scores.
        </Notice>
      )}
      <p className="xt-scores-note">
        Each grading runs Claude Code on your Claude account and takes a few minutes.
      </p>
      <p className="xt-scores-message" role="status">
        {message}
      </p>
      {loaded.running && !shownOnARow(skill.rows) && (
        <RunningElsewhere
          running={loaded.running}
          reported={progress[loaded.running.run_id]}
          onCancel={cancel}
        />
      )}
      {loaded.making_rubric && <MakingRubric making={loaded.making_rubric} onCancel={cancel} />}
      <SectionCard
        title={skill.graded === 0 && skill.total > 0 ? 'Grade your first project' : 'Projects'}
        meta={`${skill.total} listed`}
        padding={0}
      >
        <DataTable
          label={`${skill.skill.name} scores`}
          columns={columns}
          rows={skill.rows}
          getRowKey={(row) => row.path}
          rowHeight={44}
          stickyHeader
          emptyMessage="No projects found in Claude Code, the Claude app, Codex or Cursor."
        />
      </SectionCard>
      <RunDetailsDialog
        target={details}
        skill={skill.skill.name}
        onClose={() => setDetails(null)}
        returnFocusRef={detailsTrigger}
      />
    </>
  );
}

/**
 * Where an added skill came from, and Remove from Scores: it takes the skill
 * off Scores (its stored grades and its files stay) and goes back to Scores.
 */
function AddedFrom({
  skillId,
  folder,
  plugin,
  setMessage,
}: {
  skillId: string;
  /** The folder it is opened from now; none when its plugin is gone. */
  folder: string | null;
  /** The Claude Code plugin it comes from, if any. */
  plugin: string | null;
  setMessage: (text: string | null) => void;
}) {
  const { source } = useData();
  const refresh = useRefreshOverview();
  const navigate = useNavigate();
  const [removing, setRemoving] = useState(false);
  const remove = async () => {
    if (removing || !source.grading) return;
    setRemoving(true);
    setMessage(null);
    try {
      const answer = await source.grading.removeSkill(skillId);
      if (answer.state === 'removed') {
        refresh();
        void navigate('/scores');
        return;
      }
      setMessage(removeText(answer));
    } catch {
      setMessage(removeText({ state: 'failed' }));
    } finally {
      setRemoving(false);
    }
  };
  return (
    <div className="xt-scores-added">
      <p className="xt-scores-note" title={folder ?? undefined}>
        {plugin === null ? 'Added from ' : `Added from plugin ${plugin}`}
        {plugin !== null && folder !== null && ' · '}
        {folder !== null && <span className="xt-scores-added-folder">{folder}</span>}
      </p>
      <Button
        variant="outline"
        height={28}
        aria-disabled={removing || undefined}
        onClick={() => void remove()}
      >
        Remove from Scores
      </Button>
    </div>
  );
}

/** A row's newer run, or that its shown score is partial, or nothing. */
function RowState({ row, progress }: { row: GradingProjectRow; progress: Record<string, number> }) {
  const attempt = row.attempt;
  const elapsed = useElapsed(
    attempt?.state === 'running' ? attempt.started_at_ms : undefined,
    attempt ? progress[attempt.run_id] : undefined,
  );
  if (attempt) {
    const running = attempt.state === 'running';
    const text = attemptText(attempt, running ? elapsed : undefined);
    const when =
      attempt.state === 'running'
        ? `Started ${timeText(attempt.started_at_ms)}`
        : `Ended ${timeText(attempt.finished_at_ms)}`;
    return (
      <span className="xt-scores-state" data-state={attempt.state} title={`${text}. ${when}.`}>
        {running && <i className="xt-scores-live" aria-hidden="true" />}
        {text}
      </span>
    );
  }
  if (row.shown?.partial)
    return (
      <span
        className="xt-scores-state"
        data-state="partial"
        title="The skill skipped a step, so this score may miss something."
      >
        Partial result
      </span>
    );
  return null;
}
