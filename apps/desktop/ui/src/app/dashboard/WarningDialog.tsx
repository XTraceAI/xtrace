import { useNavigate, useSearchParams } from 'react-router';
import { useSelectedRange } from './range';
import { listAddress, listState } from '../session-search';
import { WarningContext } from './WarningContext';
import { useId, useImperativeHandle, useRef, type RefObject } from 'react';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { Modal } from '../../kit/Modal';
import {
  DetachedDiagnosis,
  DiagnoseButton,
  DiagnosisPanel,
  usePanelShown,
} from '../session-diagnosis';
import { useDiagnose } from '../useSessionDiagnosis';
import {
  DismissButton,
  WarningLine,
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
  const read =
    !query.isFetching && !query.isError && query.data?.state === 'read' ? query.data : null;
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
  const elsewhere =
    stage?.kind === 'running' &&
    (stage.sessionId !== selected.sessionId || stage.warningId !== shown.id);
  const close = () => {
    diagnose?.notNow();
    onClose();
  };
  useImperativeHandle(requestClose, () => close);
  const press = warning ? rowProps(warning, Boolean(dismissed)) : null;
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
            <WarningContext
              sessionId={selected.sessionId}
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
                    <p className="xt-warning-modal-label">{SESSION_WARNINGS_TEXT.flagged_label}</p>
                    <h3>{SESSION_WARNINGS_TEXT.flagged_headings[warning.kind]}</h3>
                    <p className="xt-warning-modal-reason">{warning.reason}</p>
                    <p className="xt-warning-modal-next">{SESSION_WARNINGS_TEXT.flagged_next}</p>
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
                  {active && elsewhere && (
                    <p className="xt-warning-modal-busy" data-testid="diagnosis-busy">
                      {SESSION_WARNINGS_TEXT.diagnose.busy}
                    </p>
                  )}
                </>
              )}
              {warning.returned && (
                <p className="xt-warning-returned" data-testid="warning-returned">
                  {warning.returned}
                </p>
              )}
              {press?.refused && (
                <p className="xt-warning-refused" role="alert" data-testid="warning-refused">
                  {dismissed
                    ? SESSION_WARNINGS_TEXT.restore_failed
                    : SESSION_WARNINGS_TEXT.dismiss_failed}
                </p>
              )}
            </section>
          </div>
        ) : (
          <section className="xt-warning-modal-side" data-alone="">
            <WarningLine warning={selected.warning} />
            <p className="xt-session-detail-notice" role="status" data-testid="warnings-state">
              {query.isError
                ? SESSION_WARNINGS_TEXT.session_unread
                : query.isFetching || query.isPending
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
