import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useKept, type Diagnose, type Kept } from './useSessionDiagnosis';
import { isRunFor } from '../data/diagnosis-runs';
import type { DiagnosisToolTried } from '../data/generated/DiagnosisToolTried';
import type { SavedSkill } from '../data/generated/SavedSkill';
import type { SessionDiagnosis } from '../data/generated/SessionDiagnosis';
import type { SessionWarningItem } from '../data/generated/SessionWarningItem';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import '../styles/session-diagnosis.css';

/**
 * Diagnose on a session's page: one warning's moment is sent, on a confirmed
 * click, to one coding tool's hosted model, and its answer is shown under
 * the warning. Which messages are sent, to which tool, every sentence of the
 * answer and every refusal are the native side's (`SessionDiagnosis`,
 * `SavedSkill`, `SESSION_WARNINGS_TEXT.diagnose`); nothing here decides or
 * words one. One diagnosis runs at a time, owned by the source runtime
 * (`DiagnosisRuns`), so closing a view leaves it running. An answer is kept
 * in memory (the query cache, outside every key an ingest re-reads) and
 * nothing is stored.
 */
const WORDS = SESSION_WARNINGS_TEXT.diagnose;

/** The warning's own Diagnose button, in its row's buttons. */
export function DiagnoseButton({
  diagnose,
  warning,
  primary = false,
}: {
  diagnose: Diagnose;
  warning: SessionWarningItem;
  /** The window's main action: drawn as the primary button, with its mark. */
  primary?: boolean;
}) {
  const kept = useKept(diagnose.sessionId, warning.id);
  // One at a time: every Diagnose button waits while any is asked or runs,
  // and this one while its answer is shown.
  const waiting = diagnose.stage !== null;
  const shown = kept !== undefined;
  return (
    <button
      type="button"
      className="xt-warning-action"
      data-action="diagnose"
      data-variant={primary ? 'primary' : undefined}
      aria-label={
        shown
          ? `${WORDS.close}: ${warning.label}`
          : WORDS.button_name.replace('{label}', warning.label)
      }
      aria-disabled={(!shown && waiting) || undefined}
      aria-expanded={isRunFor(diagnose.stage, diagnose.sessionId, warning.id) || kept !== undefined}
      // WebKit leaves buttons out of the Tab order unless told.
      tabIndex={0}
      onClick={
        shown
          ? () => diagnose.close(warning.id)
          : waiting
            ? undefined
            : () => diagnose.ask(warning.id)
      }
    >
      {primary && !shown && (
        <span className="xt-diagnosis-mark" aria-hidden="true">
          ✧
        </span>
      )}
      {shown ? WORDS.close : WORDS.button}
    </button>
  );
}

/** The row's Diagnose button, found from inside the row. */
const rowButton = (row: Element | null) =>
  row?.querySelector<HTMLElement>('[data-action="diagnose"], [data-action="restore"]') ?? null;

/**
 * Gives focus back to the row's Diagnose button once a panel has gone. Where
 * the panel stood in for the button (the Dashboard's window), the button is
 * drawn again only after the panel's state has changed, so this waits a few
 * frames for it. `now` focuses a button already there at once.
 */
function refocusRow(from: Element | null, now: boolean) {
  const row = from?.closest('[data-warning-id]') ?? null;
  const there = rowButton(row);
  if (now && there?.isConnected) {
    there.focus();
    return;
  }
  // Hold focus on the row while the panel's control (Not now, Hide
  // diagnosis) goes, so focus is never left on a removed element (a window
  // that closes on a press outside would take it back to itself), then hand
  // it to the button once it is drawn again.
  if (row instanceof HTMLElement && row.tabIndex >= -1) row.focus({ preventScroll: true });
  let frames = 0;
  const attempt = () => {
    const button = rowButton(row);
    if (button?.isConnected) button.focus();
    else if (++frames < 5 && row?.isConnected) requestAnimationFrame(attempt);
  };
  requestAnimationFrame(attempt);
}

/**
 * Whether the panel under a warning shows something (the sentence before
 * sending, the running diagnosis, or a kept answer), so a layout that puts
 * the panel in place of the warning's text knows when to.
 */
export function usePanelShown(diagnose: Diagnose | null, warning: SessionWarningItem) {
  const kept = useKept(diagnose?.sessionId ?? '', warning.id);
  if (!diagnose) return false;
  return isRunFor(diagnose.stage, diagnose.sessionId, warning.id) || kept !== undefined;
}

/**
 * Under a warning: the sentence before sending with its confirm, the run in
 * progress with Stop, or the answer.
 */
export function DiagnosisPanel({
  diagnose,
  warning,
}: {
  diagnose: Diagnose;
  warning: SessionWarningItem;
}) {
  const kept = useKept(diagnose.sessionId, warning.id);
  const stage = isRunFor(diagnose.stage, diagnose.sessionId, warning.id) ? diagnose.stage : null;
  if (stage?.kind === 'confirm') return <Confirm diagnose={diagnose} warningId={warning.id} />;
  if (stage?.kind === 'running') return <Running diagnose={diagnose} readId={stage.readId} />;
  if (!kept) return null;
  return (
    <Result
      kept={kept}
      diagnose={diagnose}
      warningId={warning.id}
      // A new answer is a new result, focused when it arrives.
      key={JSON.stringify([
        diagnose.sessionId,
        warning.id,
        warning.points_at.record_uuid,
        warning.points_at.block,
        kept.readId,
      ])}
    />
  );
}

/**
 * Keeps Stop available for a running diagnosis whose row this view does not
 * show. Another warning's run (another session's, or another of this
 * session's) says one runs at a time, with Stop; this session's own warning
 * whose row a dismissal removed, or `only`'s, shows its run as it would.
 */
export function DetachedDiagnosis({
  diagnose,
  warnings,
  visibleWarningId,
  only,
}: {
  diagnose: Diagnose;
  warnings: readonly SessionWarningItem[] | undefined;
  visibleWarningId?: string | null;
  /** Shows only this warning's run, wherever it is. */
  only?: { sessionId: string; warningId: string };
}) {
  const stage = diagnose.stage;
  if (stage?.kind !== 'running') return null;
  if (only)
    return isRunFor(stage, only.sessionId, only.warningId) ? (
      <Running diagnose={diagnose} readId={stage.readId} key={stage.readId} />
    ) : null;
  const listed = warnings?.some((warning) => isRunFor(stage, diagnose.sessionId, warning.id));
  if (listed && (visibleWarningId === undefined || stage.warningId === visibleWarningId))
    return null;
  return listed || stage.sessionId !== diagnose.sessionId ? (
    <BusyDiagnosis diagnose={diagnose} key={stage.readId} />
  ) : (
    <Running diagnose={diagnose} readId={stage.readId} key={stage.readId} />
  );
}

/**
 * Another warning is being diagnosed: one runs at a time, said the same way
 * wherever it shows, with its Stop.
 */
export function BusyDiagnosis({ diagnose }: { diagnose: Diagnose }) {
  return (
    <div className="xt-diagnosis" data-testid="diagnosis-busy">
      <p className="xt-diagnosis-busy">{WORDS.busy}</p>
      <span className="xt-diagnosis-buttons">
        <StopButton diagnose={diagnose} />
      </span>
    </div>
  );
}

/**
 * Stop for the running diagnosis. It waits in every view once Stop was asked,
 * and stays waiting once accepted until the run ends (`DiagnosisRun.stop`).
 */
function StopButton({
  diagnose,
  buttonRef,
}: {
  diagnose: Diagnose;
  buttonRef?: RefObject<HTMLButtonElement | null>;
}) {
  const stage = diagnose.stage;
  const waiting = stage?.kind !== 'running' || stage.stop !== 'none';
  return (
    <button
      ref={buttonRef}
      type="button"
      className="xt-warning-action"
      data-action="stop"
      tabIndex={0}
      aria-disabled={waiting || undefined}
      onClick={waiting ? undefined : () => void diagnose.stop()}
    >
      {WORDS.stop}
    </button>
  );
}

function Confirm({ diagnose, warningId }: { diagnose: Diagnose; warningId: string }) {
  const sentence = useId();
  const confirm = useRef<HTMLButtonElement>(null);
  useEffect(() => confirm.current?.focus(), []);
  const back = () => {
    const from = confirm.current;
    diagnose.notNow();
    refocusRow(from, true);
  };
  return (
    <div
      className="xt-diagnosis"
      data-testid="diagnosis-confirm"
      role="group"
      aria-labelledby={sentence}
      onKeyDown={(event) => {
        if (event.key !== 'Escape') return;
        event.stopPropagation();
        back();
      }}
    >
      <p id={sentence} className="xt-diagnosis-before">
        {SESSION_WARNINGS_TEXT.diagnose_before_sending}
      </p>
      <span className="xt-diagnosis-buttons">
        <button
          ref={confirm}
          type="button"
          className="xt-warning-action"
          data-variant="primary"
          tabIndex={0}
          onClick={() => diagnose.send(warningId)}
        >
          {WORDS.confirm}
        </button>
        <button type="button" className="xt-warning-action" tabIndex={0} onClick={back}>
          {WORDS.not_now}
        </button>
      </span>
    </div>
  );
}

/** The small turning ring shown beside a "…" while something is read. */
export function Spinner() {
  return <span className="xt-diagnosis-spinner" aria-hidden="true" />;
}

function Running({ diagnose, readId }: { diagnose: Diagnose; readId: string }) {
  const stop = useRef<HTMLButtonElement>(null);
  // Stop takes focus right after this view's own send; a view opened while
  // the diagnosis runs leaves focus where it is.
  const sentHere = diagnose.sentHere === readId;
  useEffect(() => {
    if (sentHere) stop.current?.focus();
  }, [sentHere]);
  return (
    <div className="xt-diagnosis" data-testid="diagnosis-running" aria-busy="true">
      <p className="xt-diagnosis-running" role="status">
        <Spinner />
        {WORDS.running}
      </p>
      <span className="xt-diagnosis-buttons">
        <StopButton diagnose={diagnose} buttonRef={stop} />
      </span>
    </div>
  );
}

/** The tools not used, and why, as the native side says it. */
function Tried({ tried }: { tried: readonly DiagnosisToolTried[] }) {
  if (tried.length === 0) return null;
  return (
    <ul className="xt-diagnosis-tried" data-testid="diagnosis-tried">
      {tried.map((one) => (
        <li key={`${one.tool}:${one.text}`}>{one.text}</li>
      ))}
    </ul>
  );
}

/** One labelled part of an answer. */
function Part({
  label,
  children,
  testId,
}: {
  label: string;
  children: ReactNode;
  testId?: string;
}) {
  const id = useId();
  return (
    <div className="xt-diagnosis-part" data-testid={testId} role="group" aria-labelledby={id}>
      <span id={id} className="xt-diagnosis-label">
        {label}
      </span>
      <div className="xt-diagnosis-body">{children}</div>
    </div>
  );
}

function Result({
  kept,
  diagnose,
  warningId,
}: {
  kept: Kept;
  diagnose: Diagnose;
  warningId: string;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  const title = useId();
  // Focus moves into the answer when it arrives, so a reader hears it.
  useEffect(() => heading.current?.focus({ preventScroll: false }), []);
  const close = () => {
    const from = heading.current;
    diagnose.close(warningId);
    // The button is told it may be pressed again (or drawn again) on the
    // next draw.
    refocusRow(from, false);
  };
  const answer = 'answer' in kept ? kept.answer : null;
  return (
    <section
      className="xt-diagnosis"
      data-testid="diagnosis-result"
      data-state={answer?.state ?? 'failed'}
      aria-labelledby={title}
      onKeyDown={(event) => {
        if (event.key !== 'Escape') return;
        event.stopPropagation();
        close();
      }}
    >
      <header className="xt-diagnosis-head">
        <h4 id={title} ref={heading} tabIndex={-1}>
          {WORDS.heading}
        </h4>
      </header>
      {answer ? <Answer answer={answer} diagnose={diagnose} /> : <p>{WORDS.failed}</p>}
      {answer && <RunDetails answer={answer} />}
      <span className="xt-diagnosis-buttons">
        <button type="button" className="xt-warning-action" tabIndex={0} onClick={close}>
          {WORDS.close}
        </button>
      </span>
    </section>
  );
}

function RunDetails({ answer }: { answer: SessionDiagnosis }) {
  if (
    answer.state !== 'diagnosed' &&
    answer.state !== 'no_agent_tool' &&
    answer.state !== 'answer_refused' &&
    answer.state !== 'run_failed'
  )
    return null;
  return (
    <details className="xt-diagnosis-details" data-testid="diagnosis-details">
      <summary tabIndex={0}>{WORDS.details}</summary>
      <div className="xt-diagnosis-parts">
        {'ran' in answer && (
          <p className="xt-diagnosis-ran" data-testid="diagnosis-ran">
            {answer.ran}
          </p>
        )}
        {'reason' in answer && <Part label={WORDS.reason}>{String(answer.reason)}</Part>}
        {'sent' in answer && answer.sent && (
          <Part label={WORDS.sent} testId="diagnosis-sent">
            <p>{answer.sent.text}</p>
          </Part>
        )}
        <Tried tried={answer.tried} />
        {'limits' in answer && answer.limits.length > 0 && (
          <ul className="xt-diagnosis-tried" data-testid="diagnosis-limits">
            {answer.limits.map((limit) => (
              <li key={limit.kind}>{limit.text}</li>
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}

function Answer({ answer, diagnose }: { answer: SessionDiagnosis; diagnose: Diagnose }) {
  if (answer.state !== 'diagnosed')
    return (
      <p className="xt-diagnosis-text" data-testid="diagnosis-text">
        {answer.text}
      </p>
    );
  return (
    <div className="xt-diagnosis-parts">
      <p
        className="xt-diagnosis-verdict"
        data-testid="diagnosis-real"
        data-value={answer.real_problem}
      >
        {answer.real_problem_text}
      </p>
      <Part label={WORDS.what_happened} testId="diagnosis-what">
        {answer.what_happened}
      </Part>
      <Evidence answer={answer} />
      <Part label={WORDS.next_action}>
        <p data-testid="diagnosis-suggestion">{answer.suggestion_text}</p>
        {answer.draft_kind === 'skill' && (
          <Part label={WORDS.skill_draft} testId="diagnosis-skill">
            <SkillDraft answer={answer} diagnose={diagnose} />
          </Part>
        )}
        {answer.draft_kind === 'instruction_line' && (
          <Part
            label={
              answer.instruction_file
                ? WORDS.line_for_file.replace('{file}', answer.instruction_file)
                : WORDS.line_no_file
            }
            testId="diagnosis-line"
          >
            <InstructionLine line={answer.draft} />
          </Part>
        )}
      </Part>
    </div>
  );
}

function Evidence({ answer }: { answer: Extract<SessionDiagnosis, { state: 'diagnosed' }> }) {
  const disclosure = useRef<HTMLDetailsElement>(null);
  const messages = useRef(new Map<number, HTMLElement>());
  const frame = useRef<number | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );
  const show = (number: number) => {
    if (!disclosure.current) return;
    disclosure.current.open = true;
    setSelected(number);
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => {
      messages.current.get(number)?.focus();
      frame.current = null;
    });
  };
  return (
    <Part label={WORDS.why} testId="diagnosis-why">
      <p className="xt-diagnosis-explanation">
        {answer.why_parts.map((part, index) =>
          part.kind === 'message' ? (
            <button
              key={index}
              type="button"
              className="xt-diagnosis-reference"
              tabIndex={0}
              onClick={() => show(part.number)}
            >
              {part.text}
            </button>
          ) : part.kind === 'unavailable' ? (
            <span key={index} title={WORDS.reference_unavailable}>
              {part.text}
            </span>
          ) : (
            <span key={index}>{part.text}</span>
          ),
        )}
      </p>
      {answer.why_parts.some((part) => part.kind === 'unavailable') && (
        <p className="xt-diagnosis-note">{WORDS.reference_unavailable}</p>
      )}
      <details ref={disclosure} className="xt-diagnosis-evidence" data-testid="diagnosis-evidence">
        <summary tabIndex={0}>{WORDS.evidence}</summary>
        <p className="xt-diagnosis-note">{WORDS.evidence_note}</p>
        <div className="xt-diagnosis-messages">
          {answer.messages.map((message) => (
            <article
              key={message.number}
              ref={(element) => {
                if (element) messages.current.set(message.number, element);
                else messages.current.delete(message.number);
              }}
              tabIndex={-1}
              aria-label={message.label}
              data-testid="diagnosis-message"
              data-number={message.number}
              data-selected={selected === message.number || undefined}
            >
              <h5>{message.label}</h5>
              <p>{message.text}</p>
            </article>
          ))}
        </div>
      </details>
    </Part>
  );
}

/** The whole suggested `SKILL.md`, where it would be saved, and Save. */
function SkillDraft({
  answer,
  diagnose,
}: {
  answer: Extract<SessionDiagnosis, { state: 'diagnosed' }>;
  diagnose: Diagnose;
}) {
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<SavedSkill | 'failed' | null>(null);
  const name = answer.skill_name;
  const save = () => {
    if (saving || !name) return;
    setSaving(true);
    diagnose.controls
      .saveSkill(diagnose.sessionId, name, answer.draft)
      .then(
        (result) => setSaved(result),
        () => setSaved('failed'),
      )
      .finally(() => setSaving(false));
  };
  const done = saved !== null && saved !== 'failed' && saved.state === 'saved';
  return (
    <>
      <pre
        className="xt-diagnosis-draft"
        data-testid="diagnosis-draft"
        // A scrolling region the keyboard can reach and scroll.
        tabIndex={0}
        aria-label={WORDS.skill_draft}
      >
        {answer.draft}
      </pre>
      {answer.skill_path ? (
        <p className="xt-diagnosis-path" data-testid="diagnosis-path">
          {WORDS.skill_path} <code>{answer.skill_path}</code>
        </p>
      ) : (
        <p className="xt-diagnosis-path">{WORDS.skill_no_path}</p>
      )}
      {answer.skill_note && (
        <p className="xt-diagnosis-note" data-testid="diagnosis-note">
          {answer.skill_note}
        </p>
      )}
      {answer.skill_path && name && (
        <span className="xt-diagnosis-buttons">
          <button
            type="button"
            className="xt-warning-action"
            data-variant="primary"
            data-action="save-skill"
            tabIndex={0}
            aria-disabled={saving || done || undefined}
            onClick={saving || done ? undefined : save}
          >
            {saving ? WORDS.saving : WORDS.save_skill}
          </button>
          {saved && (
            <span
              className="xt-diagnosis-saved"
              role="status"
              data-testid="diagnosis-saved"
              data-state={saved === 'failed' ? 'failed' : saved.state}
            >
              {saved === 'failed'
                ? WORDS.save_failed
                : saved.state === 'saved'
                  ? `${saved.text} ${saved.path}`
                  : saved.text}
            </span>
          )}
        </span>
      )}
    </>
  );
}

/** One line for the project's instruction file, to copy; nothing is written. */
function InstructionLine({ line }: { line: string }) {
  const [copied, setCopied] = useState<'copied' | 'failed' | null>(null);
  const copy = () => {
    const done = navigator.clipboard?.writeText(line);
    if (!done) {
      setCopied('failed');
      return;
    }
    done.then(
      () => setCopied('copied'),
      () => setCopied('failed'),
    );
  };
  return (
    <>
      <pre className="xt-diagnosis-line" data-testid="diagnosis-line-text" tabIndex={0}>
        {line}
      </pre>
      <span className="xt-diagnosis-buttons">
        <button
          type="button"
          className="xt-warning-action"
          data-action="copy-line"
          tabIndex={0}
          onClick={copy}
        >
          {WORDS.copy}
        </button>
        {copied && (
          <span className="xt-diagnosis-saved" role="status" data-testid="diagnosis-copied">
            {copied === 'copied' ? WORDS.copied : WORDS.copy_failed}
          </span>
        )}
      </span>
    </>
  );
}
