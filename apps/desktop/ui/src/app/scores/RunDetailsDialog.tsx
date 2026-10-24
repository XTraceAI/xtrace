import { useEffect, useId, useState, type RefObject } from 'react';
import { useData } from '../../data/DataProvider';
import type { GradingInsight } from '../../data/generated/GradingInsight';
import type { GradingRunDetailsLoaded } from '../../data/generated/GradingRunDetailsLoaded';
import type { GradingRunKept } from '../../data/generated/GradingRunKept';
import type { GradingRunReport } from '../../data/generated/GradingRunReport';
import { Button } from '../../kit/Button';
import { LoadingRows } from '../../kit/EmptyState';
import { Icon } from '../../kit/icons';
import { Modal, ModalClose } from '../../kit/Modal';
import {
  criteriaGapText,
  detailsText,
  FINDINGS_UNREADABLE,
  OPEN_REPORT_FAILED,
  openReportText,
  reportGapText,
  severity,
  timeText,
  useRunDetails,
} from './scores';

/** The project row whose shown score's details are open. */
export interface DetailsTarget {
  runId: string;
  project: string;
  path: string;
}

/**
 * The details of the run behind a project's shown score: the score, when
 * it was graded and whether it was complete, the summary, the points per
 * rubric criterion, every finding with its explanation and fix, and "Open
 * full report". What is shown, in which order, and why criteria or a report
 * are missing are the native answer's (`GradingRunDetails`); this only
 * lays them out and sends Open full report with the run's ID.
 */
export function RunDetailsDialog({
  target,
  skill,
  onClose,
  returnFocusRef,
}: {
  target: DetailsTarget | null;
  skill: string;
  onClose: () => void;
  returnFocusRef: RefObject<HTMLElement | null>;
}) {
  const title = useId();
  // While it closes, the dialog still names the project it showed.
  const [last, setLast] = useState(target);
  if (target !== null && target !== last) setLast(target);
  const shown = target ?? last;
  const details = useRunDetails(shown?.runId ?? null, target !== null);
  const answer = details.data;
  return (
    <Modal
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      returnFocusRef={returnFocusRef}
      aria-labelledby={title}
      className="xt-scores-modal xt-score-details-modal"
    >
      {/* Close comes first, so opening by keyboard lands on it. */}
      <header className="xt-scores-modal-header">
        <h2 id={title}>
          {shown?.project} · {skill}
        </h2>
        <ModalClose className="xt-button" data-variant="outline" style={{ height: 28 }}>
          Close
        </ModalClose>
      </header>
      <p className="xt-scores-note xt-score-details-path" title={shown?.path}>
        {shown?.path}
      </p>
      {details.isError || (answer && answer.state !== 'loaded') ? (
        <div className="xt-scores-modal-error" role="alert">
          <p>
            {answer && answer.state !== 'loaded'
              ? detailsText(answer)
              : detailsText({ state: 'failed' })}
          </p>
          {(details.isError || answer?.state === 'failed') && (
            <Button variant="outline" height={28} onClick={() => void details.refetch()}>
              Try again
            </Button>
          )}
        </div>
      ) : !answer ? (
        <LoadingRows rows={4} />
      ) : (
        <Loaded details={answer} />
      )}
    </Modal>
  );
}

function Loaded({ details }: { details: GradingRunDetailsLoaded }) {
  return (
    <>
      <div className="xt-score-details-top">
        <span className="xt-score-big">{details.score}</span>
        <span className="sr-only">Score {details.score}</span>
        <dl className="xt-score-details-facts">
          <div>
            <dt>Graded</dt>
            <dd>
              <time dateTime={new Date(details.graded_at_ms).toISOString()}>
                {timeText(details.graded_at_ms)}
              </time>
            </dd>
          </div>
          <div>
            <dt>Result</dt>
            <dd data-state={details.complete ? 'complete' : 'partial'}>
              {details.complete
                ? 'Complete'
                : 'Partial: the skill skipped a step, so this score may miss something'}
            </dd>
          </div>
        </dl>
      </div>
      <div
        className="xt-score-details-body"
        role="region"
        aria-label="Grade details"
        // WebKit skips a scrolling region on Tab without an explicit tab stop.
        tabIndex={0}
      >
        {details.content.state === 'deleted' ? (
          <p className="xt-score-details-gap">
            Stored content was deleted. The score stays; its summary, findings, points and full
            report are gone.
          </p>
        ) : (
          <Kept kept={details.content} />
        )}
      </div>
      {details.content.state === 'kept' && (
        <Report runId={details.run_id} report={details.content.report} />
      )}
    </>
  );
}

/** A kept run's summary, points per criterion and findings. */
function Kept({ kept }: { kept: GradingRunKept }) {
  const summary = useId();
  const criteria = useId();
  const findings = useId();
  return (
    <>
      <section className="xt-score-details-section" aria-labelledby={summary}>
        <h3 id={summary}>Summary</h3>
        <p className="xt-score-details-summary">{kept.summary}</p>
      </section>
      <section className="xt-score-details-section" aria-labelledby={criteria}>
        <h3 id={criteria}>Points by criterion</h3>
        {kept.criteria.state === 'reported' ? (
          <ul className="xt-score-criteria" role="list">
            {kept.criteria.criteria.map((criterion) => (
              <li key={criterion.name} className="xt-score-criterion">
                <span className="xt-score-criterion-name">{criterion.name}</span>
                <span className="xt-score-criterion-points">
                  {criterion.awarded} / {criterion.max}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="xt-score-details-gap">{criteriaGapText[kept.criteria.reason]}</p>
        )}
      </section>
      <section className="xt-score-details-section" aria-labelledby={findings}>
        <h3 id={findings}>
          Findings
          {kept.findings.state === 'listed' && (
            <span className="xt-score-details-count">{kept.findings.findings.length}</span>
          )}
        </h3>
        {kept.findings.state === 'unreadable' ? (
          <p className="xt-score-details-gap">{FINDINGS_UNREADABLE}</p>
        ) : kept.findings.findings.length === 0 ? (
          <p className="xt-score-details-gap">This grade has no findings.</p>
        ) : (
          <ol className="xt-score-findings" role="list">
            {kept.findings.findings.map((finding, index) => (
              <Finding key={index} finding={finding} />
            ))}
          </ol>
        )}
      </section>
    </>
  );
}

function Finding({ finding }: { finding: GradingInsight }) {
  const weight = severity[finding.severity];
  return (
    <li className="xt-score-finding">
      <span
        className="xt-score-insight-icon xt-control-tone"
        data-tone={weight.tone}
        role="img"
        aria-label={weight.label}
        title={weight.label}
      >
        <Icon name={finding.severity === 'pass' ? 'check' : 'info'} size={12} />
      </span>
      <div className="xt-score-finding-text">
        <p className="xt-score-finding-title">{finding.title}</p>
        {finding.detail && <p className="xt-score-finding-detail">{finding.detail}</p>}
        {finding.fix && (
          <p className="xt-score-finding-fix">
            <strong>Fix</strong> {finding.fix}
          </p>
        )}
      </div>
    </li>
  );
}

/** Open full report, or why there is none. */
function Report({ runId, report }: { runId: string; report: GradingRunReport }) {
  const { source } = useData();
  const [message, setMessage] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  // Another run's details say nothing of this one's press.
  useEffect(() => setMessage(null), [runId]);
  const open = async () => {
    if (opening || !source.grading) return;
    setOpening(true);
    setMessage(null);
    try {
      const answer = await source.grading.openReport(runId);
      setMessage(answer.state === 'opened' ? null : openReportText(answer));
    } catch {
      setMessage(OPEN_REPORT_FAILED);
    } finally {
      setOpening(false);
    }
  };
  return (
    <footer className="xt-score-details-footer">
      {report.state === 'kept' ? (
        <Button
          variant="primary"
          height={28}
          aria-disabled={opening || undefined}
          onClick={() => void open()}
        >
          Open full report
        </Button>
      ) : (
        <p className="xt-score-details-gap">{reportGapText[report.reason]}</p>
      )}
      <p className="xt-score-details-message" role="status">
        {message}
      </p>
    </footer>
  );
}
