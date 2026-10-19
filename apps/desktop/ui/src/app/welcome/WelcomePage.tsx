import { useId, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DbCounts } from '../../data/generated/DbCounts';
import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { count } from '../../kit/format';
import { HostGlyph } from '../../kit/HostGlyph';
import { Icon } from '../../kit/icons';
import { MetricCell } from '../../kit/MetricCell';
import { hostScanText } from '../index-host-state';
import { freshnessText } from '../native-index-text';
import { pages } from '../routes';
import { useDbCounts, useNativeIndexStatus } from '../useAppInfo';
import { TelemetryNotice } from '../telemetry/TelemetryNotice';
import { completeWelcome } from './welcome-completion';
import {
  attentionCount,
  hostName,
  hostSummary,
  phasePill,
  plural,
  readerNote,
  welcomeTour,
} from './welcome-text';
import '../../styles/welcome.css';

const TITLE = 'Welcome to XTrace';
const COUNTS_READING = 'Reading database counts';
const COUNTS_FAILED = 'Database counts could not be read';

/**
 * First launch, as three short steps for someone new: which agent history
 * was found on this Mac, what the app shows, and that it stays local. Every
 * value is read from the status the app already publishes, never estimated;
 * the technical facts sit behind Details, and the Dashboard is one click away.
 */
export function WelcomePage() {
  const { source } = useData();
  const navigate = useNavigate();
  const index = useNativeIndexStatus();
  const counts = useDbCounts();
  const [details, setDetails] = useState(false);
  const detailsId = useId();
  const toggleId = useId();
  const preview = source.kind === 'preview';
  const failed = index.isError || counts.isError;
  const retry = () => {
    if (index.isError) void index.refetch();
    if (counts.isError) void counts.refetch();
  };
  // The explicit continue: stored when storage allows, and never a gate.
  const open = () => {
    completeWelcome();
    void navigate('/dashboard');
  };
  return (
    <section className="xt-welcome" aria-busy={index.isFetching || counts.isFetching || undefined}>
      <header className="xt-welcome-head">
        <h1>{TITLE}</h1>
        <p className="xt-welcome-subline">
          See how your coding agents spend your time, from history already on this Mac.
        </p>
      </header>
      <div className="xt-welcome-steps">
        <Step
          number={1}
          title="Finding your agent history"
          right={index.data && !preview && <PhasePill status={index.data} />}
        >
          {preview ? (
            <p className="xt-welcome-status">Open the desktop app to read local history.</p>
          ) : index.data ? (
            <PhaseSentence status={index.data} />
          ) : index.isError ? null : (
            <p role="status" className="xt-welcome-status">
              Reading local index status…
            </p>
          )}
          {failed && (
            <p role="alert" className="xt-welcome-alert">
              {index.isError && counts.isError
                ? 'Local index status and database counts could not be read.'
                : index.isError
                  ? 'Local index status could not be read.'
                  : 'Database counts could not be read.'}{' '}
              <Button
                variant="outline"
                height={28}
                disabled={index.isFetching || counts.isFetching}
                onClick={retry}
              >
                Retry
              </Button>
            </p>
          )}
          {preview ? (
            <p className="xt-welcome-note">The desktop app lists what it finds on this Mac.</p>
          ) : index.data ? (
            <Sources status={index.data} />
          ) : index.isError ? (
            <p className="xt-welcome-note">Sources are unknown until the status is read.</p>
          ) : (
            <p className="xt-welcome-note">Reading sources…</p>
          )}
          {!preview && <SessionTotal counts={counts.data} failed={counts.isError} />}
          {!preview && index.data && index.data.phase.phase !== 'disabled' && (
            <Checks status={index.data} />
          )}
        </Step>
        <Step number={2} title="What you'll find">
          <ul className="xt-welcome-tour">
            {welcomeTour.map(({ path, icon, description }) => (
              <li key={path} className="xt-welcome-tour-item">
                <span className="xt-welcome-tour-icon" aria-hidden="true">
                  <Icon name={icon} size={15} />
                </span>
                <span className="xt-welcome-tour-title">
                  {pages.find((page) => page.path === path)?.title ?? path}
                </span>
                <span className="xt-welcome-tour-text">{description}</span>
              </li>
            ))}
          </ul>
        </Step>
        <Step number={3} title="Stays on this Mac">
          <p className="xt-welcome-status">
            Your agent history is read and kept on this Mac. No account, plugin or cloud service is
            needed.
          </p>
          <TelemetryNotice closable={false} />
        </Step>
      </div>
      <div className="xt-welcome-actions">
        <Button variant="primary" height={34} className="xt-welcome-cta" onClick={open}>
          Open Dashboard
          <span className="xt-welcome-cta-arrow" aria-hidden="true">
            →
          </span>
        </Button>
        {!preview && (
          <Button
            id={toggleId}
            variant="ghost"
            height={34}
            className="xt-welcome-details-toggle"
            aria-expanded={details}
            aria-controls={detailsId}
            onClick={() => setDetails((shown) => !shown)}
          >
            <svg
              className="xt-welcome-chevron"
              data-open={details}
              width={12}
              height={12}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={2.4}
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
              focusable="false"
            >
              <path d="M9 6l6 6-6 6" />
            </svg>
            Details
          </Button>
        )}
      </div>
      {!preview && (
        <div
          id={detailsId}
          role="region"
          aria-labelledby={toggleId}
          className="xt-welcome-details"
          hidden={!details}
        >
          {details && (
            <Details status={index.data} counts={counts.data} countsFailed={counts.isError} />
          )}
        </div>
      )}
    </section>
  );
}

/** One numbered step: the number is decoration, the heading is the real structure. */
function Step({
  number,
  title,
  right,
  children,
}: {
  number: number;
  title: string;
  right?: ReactNode;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <section className="xt-welcome-step" aria-labelledby={id}>
      <span className="xt-welcome-step-number" aria-hidden="true">
        {number}
      </span>
      <div className="xt-welcome-step-body">
        <div className="xt-welcome-step-head">
          <h2 id={id}>{title}</h2>
          {right}
        </div>
        {children}
      </div>
    </section>
  );
}

function PhasePill({ status }: { status: NativeIndexStatus }) {
  const pill = phasePill(status);
  return (
    <StatePill tone={pill.tone} live={status.phase.phase === 'scanning'}>
      {pill.label}
    </StatePill>
  );
}

function PhaseSentence({ status }: { status: NativeIndexStatus }) {
  const { phase } = status;
  const attention = attentionCount(status.hosts);
  return (
    <p className="xt-welcome-status" data-testid="welcome-phase">
      {phase.phase === 'scanning'
        ? 'The initial scan of your local history is running. Counts below grow as it goes.'
        : phase.phase === 'ready'
          ? attention > 0
            ? `The initial scan finished; ${plural(attention, 'source')} reported a problem.`
            : 'The initial scan finished.'
          : phase.phase === 'disabled'
            ? `The local index is not running: ${phase.reason}`
            : 'The local index has stopped.'}
    </p>
  );
}

function Sources({ status }: { status: NativeIndexStatus }) {
  if (status.hosts.length === 0)
    return (
      <p className="xt-welcome-note">
        {status.phase.phase === 'disabled'
          ? 'No sources are read while the local index is not running.'
          : 'No sources have been reported yet.'}
      </p>
    );
  return (
    <ul className="xt-welcome-hosts" aria-label="Local history sources">
      {status.hosts.map((host) => (
        <HostRow key={host.host} host={host} />
      ))}
    </ul>
  );
}

function HostRow({ host }: { host: NativeHostStatus }) {
  const outcome = hostScanText(host);
  return (
    <li className="xt-welcome-host" data-attention={host.needs_attention || undefined}>
      <HostGlyph host={host.host} size={20} />
      <span className="xt-welcome-host-name">{hostName(host.host)}</span>
      <span className="xt-welcome-host-summary">{hostSummary(host)}</span>
      <StatePill tone={outcome.tone}>{outcome.label}</StatePill>
    </li>
  );
}

/** Sessions in the local database now: the one total a first-time user needs. */
function SessionTotal({ counts, failed }: { counts: DbCounts | undefined; failed: boolean }) {
  return (
    <p className="xt-welcome-total">
      <span className="xt-welcome-total-value">
        <MetricCell
          value={counts?.sessions}
          format={count}
          size={18}
          reason={failed ? COUNTS_FAILED : COUNTS_READING}
        />
        <span className="xt-welcome-total-unit">
          {counts?.sessions === 1 ? 'session' : 'sessions'}
        </span>
      </span>
      <span className="xt-welcome-total-caption">In the local database now</span>
    </p>
  );
}

/** The quiet checks a first-time user may glance at; the reasons are under Details. */
function Checks({ status }: { status: NativeIndexStatus }) {
  const { python, readers } = status;
  return (
    <p className="xt-welcome-checks">
      <span
        className="xt-welcome-check"
        data-tone={readers.state === 'verified' ? 'success' : 'warning'}
      >
        <Icon name={readers.state === 'verified' ? 'check' : 'info'} size={12} />
        <span>
          {readers.state === 'verified'
            ? 'Bundled readers verified'
            : 'Bundled readers not verified'}
        </span>
      </span>
      <span className="xt-welcome-check-sep" aria-hidden="true">
        ·
      </span>
      <span
        className="xt-welcome-check"
        data-tone={
          python.state === 'available'
            ? 'success'
            : python.state === 'missing'
              ? 'warning'
              : undefined
        }
      >
        <Icon name={python.state === 'available' ? 'check' : 'info'} size={12} />
        <span>
          {python.state === 'available'
            ? 'Python found'
            : python.state === 'resolving'
              ? 'Looking for Python…'
              : 'Python not found'}
        </span>
      </span>
    </p>
  );
}

const sourcesMeta = (status: NativeIndexStatus) => {
  const read = status.hosts.filter((h) => h.state === 'complete' || h.state === 'incomplete');
  return `${read.length} of ${status.hosts.length} read · last scan since launch`;
};

/** The technical facts the first glance leaves out, read from the same status. */
function Details({
  status,
  counts,
  countsFailed,
}: {
  status: NativeIndexStatus | undefined;
  counts: DbCounts | undefined;
  countsFailed: boolean;
}) {
  return (
    <div className="xt-welcome-details-body">
      <div className="xt-welcome-details-section">
        <h3>Local index</h3>
        {status ? (
          <>
            <dl className="xt-welcome-dl">
              <dt>Claude transcripts read</dt>
              <dd data-testid="welcome-files">{count(status.files_scanned)}</dd>
              <dt>Updates</dt>
              <dd
                data-tone={status.freshness.freshness === 'degraded' ? 'warning' : undefined}
                data-testid="welcome-freshness"
              >
                {freshnessText(status)}
              </dd>
              <dt>Python</dt>
              <dd data-tone={status.python.state === 'missing' ? 'warning' : undefined}>
                {status.python.state === 'available'
                  ? `Found at ${status.python.path}`
                  : status.python.state === 'resolving'
                    ? 'Looking…'
                    : `Not found: ${status.python.reason}. Codex and Cursor history needs Python 3.10 or newer; Claude history is read without it.`}
              </dd>
              <dt>Readers</dt>
              <dd data-tone={status.readers.state === 'unavailable' ? 'warning' : undefined}>
                {status.readers.state === 'verified'
                  ? 'Verified'
                  : `Unavailable: ${status.readers.reason}`}
              </dd>
            </dl>
            <p className="xt-welcome-hint">
              Transcript files are counted as they are read; the total is not known in advance.
            </p>
          </>
        ) : (
          <p className="xt-welcome-hint">Unknown until the local index status is read.</p>
        )}
      </div>
      <div className="xt-welcome-details-section">
        <h3>Database counts</h3>
        <CountTiles counts={counts} failed={countsFailed} />
      </div>
      {status && status.hosts.length > 0 && (
        <div className="xt-welcome-details-section xt-welcome-details-wide">
          <h3>
            Sources <span className="xt-welcome-details-meta">{sourcesMeta(status)}</span>
          </h3>
          <ul className="xt-welcome-host-details" aria-label="Source details">
            {status.hosts.map((host) => (
              <HostDetails key={host.host} host={host} />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function CountTiles({ counts, failed }: { counts: DbCounts | undefined; failed: boolean }) {
  const reason = failed ? COUNTS_FAILED : COUNTS_READING;
  return (
    <div className="xt-welcome-counts" role="group" aria-label="Database counts">
      {(
        [
          ['Sessions', counts?.sessions],
          ['Records', counts?.records],
          ['Usage rows', counts?.usage],
        ] as const
      ).map(([label, value]) => (
        <div key={label} className="xt-welcome-count">
          <span className="xt-welcome-count-label">{label}</span>
          <MetricCell value={value} format={count} size={18} reason={reason} />
        </div>
      ))}
    </div>
  );
}

function HostDetails({ host }: { host: NativeHostStatus }) {
  const name = hostName(host.host);
  const read = host.state === 'complete' || host.state === 'incomplete';
  return (
    <li className="xt-welcome-host-detail-item">
      <span className="xt-welcome-host-name">{name}</span>
      {read && (
        <dl className="xt-welcome-dl">
          <dt>Sessions</dt>
          <dd>
            {count(host.sessions_imported)}
            {host.sessions_partial > 0 && ` · ${count(host.sessions_partial)} partial`}
            {host.sessions_skipped > 0 && ` · ${count(host.sessions_skipped)} skipped`}
          </dd>
          <dt>New records</dt>
          <dd>{count(host.records_new)}</dd>
          {host.diagnostics > 0 && (
            <>
              <dt>Diagnostics</dt>
              <dd data-tone="warning">{count(host.diagnostics)}</dd>
            </>
          )}
        </dl>
      )}
      {host.detail && (
        <p className="xt-welcome-host-detail" title={host.detail}>
          {host.detail}
        </p>
      )}
      <p className="xt-welcome-hint">{readerNote(host.host)}</p>
    </li>
  );
}
