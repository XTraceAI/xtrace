import { Tooltip } from '@base-ui/react/tooltip';
import { useNavigate, useSearchParams } from 'react-router';
import { useSelectedRange } from './range';
import { listAddress, listState } from '../session-search';
import { WarningContextView } from './WarningContext';
import { useWarningContext } from './useWarningContext';
import { correctionQuote, type WarningContextRead } from './warning-context';
import {
  Fragment,
  useId,
  useImperativeHandle,
  useRef,
  type ReactNode,
  type RefObject,
} from 'react';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { Modal } from '../../kit/Modal';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import {
  Spinner,
  BusyDiagnosis,
  DetachedDiagnosis,
  DiagnoseButton,
  DiagnosisPanel,
  usePanelShown,
} from '../session-diagnosis';
import { useDiagnose } from '../useSessionDiagnosis';
import { isRunFor } from '../../data/diagnosis-runs';
import {
  DismissButton,
  WarningLine,
  WarningNotes,
  pointsAt,
  useSessionWarning,
  useWarningDismissals,
  warningHref,
} from '../session-warnings';

export type SelectedWarning = {
  warning: SessionWarningItem;
  name: string;
  sessionId: string;
  timezone: string;
  /** The session's host, as its row records it: named over the conversation. */
  host?: string;
};

/** One selected Dashboard warning, kept open while its source rows refresh. */
export function WarningDialog({
  selected,
  onClose,
  returnFocusRef,
}: {
  selected: SelectedWarning | null;
  onClose: () => void;
  returnFocusRef: RefObject<HTMLElement | null>;
}) {
  const title = useId();
  const session = useId();
  const requestClose = useRef<(() => void) | null>(null);
  return (
    <Modal
      open={selected !== null}
      onOpenChange={(open) => {
        if (!open) requestClose.current?.();
      }}
      returnFocusRef={returnFocusRef}
      // A press outside closes it, the same way × and Escape do.
      dismissOnOutsidePress
      aria-labelledby={`${title} ${session}`}
      className="xt-dash-modal xt-warning-modal"
      data-testid="dashboard-warning-modal"
    >
      {selected && (
        <SelectedIssue
          key={JSON.stringify([selected.sessionId, selected.warning.id])}
          selected={selected}
          title={title}
          session={session}
          onClose={onClose}
          requestClose={requestClose}
        />
      )}
    </Modal>
  );
}

/**
 * The window's frame: the session it belongs to with Close, then "Session
 * warning" and the warning's kind (and whether it was dismissed), then the
 * conversation on the left and why it was flagged, with its actions, on the
 * right. Asking to diagnose, the running diagnosis and its answer take the
 * place of the reason. Close only closes: a running diagnosis keeps running,
 * and its dot shows it.
 */
function SelectedIssue({
  selected,
  title,
  session,
  onClose,
  requestClose,
}: {
  selected: SelectedWarning;
  title: string;
  session: string;
  onClose: () => void;
  requestClose: RefObject<(() => void) | null>;
}) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const range = useSelectedRange();
  const why = useId();
  const carry = listState(params);
  for (const [key, value] of listAddress({ sessionId: selected.sessionId, host: '' }, range))
    carry.set(key, value);
  const query = useSessionWarning(selected.sessionId, selected.timezone, {
    refetchOnMount: 'always',
  });
  // Only an answer read after this window opened is acted on (the read on
  // opening is forced, `refetchOnMount: 'always'`). A later re-read, which
  // every ingest starts, keeps that finished answer shown until it lands, so
  // the conversation and a confirm step are not thrown away while it runs. A
  // failed read drops the actions.
  const read =
    query.isFetchedAfterMount && !query.isError && query.data?.state === 'read' ? query.data : null;
  const active = read?.warnings.find((warning) => warning.id === selected.warning.id);
  const dismissed = read?.dismissed.find((warning) => warning.id === selected.warning.id);
  const warning = active ?? dismissed;
  const shown = warning ?? selected.warning;
  const card = useRef<HTMLDivElement>(null);
  const rowProps = useWarningDismissals(selected.sessionId, read, card);
  const diagnose = useDiagnose(selected.sessionId, selected.timezone, read?.warnings);
  const panel = usePanelShown(diagnose, shown);
  const stage = diagnose?.stage;
  // One diagnosis runs at a time: another warning's keeps this one waiting.
  const elsewhere = stage?.kind === 'running' && !isRunFor(stage, selected.sessionId, shown.id);
  const close = () => {
    diagnose?.notNow();
    onClose();
  };
  useImperativeHandle(requestClose, () => close);
  const press = warning ? rowProps(warning, Boolean(dismissed)) : null;
  const reading = !query.isError && (query.isFetching || query.isPending);
  return (
    <>
      <div className="xt-warning-modal-bar">
        <span className="xt-warning-modal-crumb">
          XTrace / <span id={session}>{selected.name}</span>
        </span>
        <button
          type="button"
          className="xt-warning-modal-close"
          tabIndex={0}
          aria-label="Close"
          onClick={close}
        >
          <span aria-hidden="true">×</span>
        </button>
      </div>
      <header className="xt-warning-modal-head">
        <span className="xt-warning-modal-spark" aria-hidden="true">
          ✧
        </span>
        <h2>{SESSION_WARNINGS_TEXT.modal_heading}</h2>
        <span
          className="xt-warning-modal-tag"
          data-kind={shown.kind}
          data-dismissed={dismissed ? '' : undefined}
        >
          <span className="xt-warning-modal-dot" aria-hidden="true" />
          <span id={title}>{shown.label}</span>
          {dismissed && <span> · {SESSION_WARNINGS_TEXT.dismissed_heading}</span>}
        </span>
      </header>
      <div ref={card} className="xt-warning-modal-body">
        {warning ? (
          <div
            className="xt-warning-modal-item"
            data-kind={warning.kind}
            data-warning-id={warning.id}
            data-points-at={pointsAt(warning)}
            data-testid={dismissed ? 'dismissed-warning' : 'warning-item'}
            tabIndex={-1}
          >
            <WithWarningContext
              sessionId={selected.sessionId}
              warning={warning}
              timezone={selected.timezone}
            >
              {({ read, retry }) => (
                <>
                  <WarningContextView
                    read={read}
                    retry={retry}
                    warning={warning}
                    timezone={selected.timezone}
                    host={selected.host}
                    fullSession={{
                      href: warningHref(selected.sessionId, warning.id, carry),
                      open: () => {
                        diagnose?.notNow();
                        onClose();
                        navigate(warningHref(selected.sessionId, warning.id, carry));
                      },
                    }}
                  />
                  <section className="xt-warning-modal-side">
                    {panel && diagnose ? (
                      <DiagnosisPanel diagnose={diagnose} warning={warning} />
                    ) : (
                      <>
                        <div id={why} className="xt-warning-modal-why">
                          <p className="xt-warning-modal-label">
                            {SESSION_WARNINGS_TEXT.flagged_label}
                          </p>
                          <FlaggedHeading warning={warning} />
                          <WarningQuote read={read} />
                        </div>
                        {((active && diagnose) || press?.onPress) && (
                          <span className="xt-warning-actions">
                            {active && diagnose && (
                              <DiagnoseButton diagnose={diagnose} warning={warning} primary />
                            )}
                            {press?.onPress && (
                              <DismissButton
                                undo={Boolean(dismissed)}
                                busy={press.busy}
                                onPress={press.onPress}
                                describedBy={why}
                              />
                            )}
                          </span>
                        )}
                        {active && elsewhere && diagnose && <BusyDiagnosis diagnose={diagnose} />}
                      </>
                    )}
                    <WarningNotes
                      warning={warning}
                      undo={Boolean(dismissed)}
                      refused={Boolean(press?.refused)}
                    />
                  </section>
                </>
              )}
            </WithWarningContext>
          </div>
        ) : (
          <section className="xt-warning-modal-side" data-alone="">
            <WarningLine warning={selected.warning} />
            <p
              className="xt-session-detail-notice"
              role="status"
              data-testid="warnings-state"
              data-loading={reading || undefined}
            >
              {reading && <Spinner />}
              {query.isError
                ? SESSION_WARNINGS_TEXT.session_unread
                : reading
                  ? SESSION_WARNINGS_TEXT.session_reading
                  : query.data?.state === 'missing'
                    ? SESSION_WARNINGS_TEXT.session_missing
                    : SESSION_WARNINGS_TEXT.session_unavailable}
            </p>
            {diagnose && (
              <DetachedDiagnosis
                diagnose={diagnose}
                warnings={[]}
                only={{ sessionId: selected.sessionId, warningId: selected.warning.id }}
              />
            )}
          </section>
        )}
      </div>
    </>
  );
}

/**
 * Reads the warning's original messages once for the window: the
 * conversation column and a correction's quote both show this one read.
 */
function WithWarningContext({
  sessionId,
  warning,
  timezone,
  children,
}: {
  sessionId: string;
  warning: SessionWarningItem;
  timezone: string;
  children: (context: { read: WarningContextRead; retry: () => void }) => ReactNode;
}) {
  return children(useWarningContext(sessionId, warning, timezone));
}

/**
 * The short heading over why a warning was flagged ("The agent admitted a
 * mistake."), with the warning's own reason ("the agent replied "you're
 * right" after your message (08:55)") in a tooltip on it rather than a line
 * of its own. The heading is a tab stop, so the tooltip opens on keyboard
 * focus as on hover; the reason is also kept as hidden text the heading is
 * described by, so a screen reader reads it with or without the tooltip, and
 * the Dismiss button described by this block still names it.
 */
function FlaggedHeading({ warning }: { warning: SessionWarningItem }) {
  const reason = useId();
  const theme = useSurfaceTheme();
  return (
    <>
      <Tooltip.Root disableHoverablePopup>
        <Tooltip.Trigger
          render={<h3 className="xt-warning-modal-heading" data-testid="warning-heading" />}
          tabIndex={0}
          aria-describedby={reason}
          delay={0}
        >
          {SESSION_WARNINGS_TEXT.flagged_headings[warning.kind]}
        </Tooltip.Trigger>
        <Tooltip.Portal data-theme={theme}>
          <Tooltip.Positioner
            className="xt-rule-positioner"
            positionMethod="fixed"
            side="bottom"
            align="start"
            sideOffset={6}
            collisionPadding={8}
          >
            {/* Hidden from a screen reader: the heading's description says it. */}
            <Tooltip.Popup
              aria-hidden="true"
              className="xt-rule-popover xt-rule-definition xt-warning-modal-reason-tip"
              data-testid="warning-reason-tip"
            >
              {warning.reason}
            </Tooltip.Popup>
          </Tooltip.Positioner>
        </Tooltip.Portal>
      </Tooltip.Root>
      <span id={reason} className="sr-only" data-testid="warning-reason-text">
        {warning.reason}
      </span>
    </>
  );
}

/**
 * A correction's whole message, under "you wrote", with the words that made
 * it a correction marked (the native marks, `correctionQuote`). Nothing until
 * the message is read, when it cannot be or marks nothing, and for every
 * other kind: the heading's tooltip carries the reason then.
 */
function WarningQuote({ read }: { read: WarningContextRead }) {
  // The native side marks only a correction whose phrase the marks agree
  // with: marks present are enough.
  const quote = correctionQuote(read);
  if (!quote) return null;
  return (
    <div className="xt-warning-modal-reason" data-testid="warning-reason" data-quote="">
      <span className="xt-warning-modal-wrote">{SESSION_WARNINGS_TEXT.you_wrote}</span>
      <blockquote className="xt-warning-quote">
        {quote.map((parts, line) => (
          <Fragment key={line}>
            {line > 0 && '\n'}
            {parts.map((part, index) =>
              part.marked ? (
                <mark key={index} className="xt-warning-mark">
                  {part.text}
                </mark>
              ) : (
                <Fragment key={index}>{part.text}</Fragment>
              ),
            )}
          </Fragment>
        ))}
      </blockquote>
    </div>
  );
}
