import { useNavigate, useSearchParams } from 'react-router';
import { useSelectedRange } from './range';
import { listAddress, listState } from '../session-search';
import { WarningContext } from './WarningContext';
import { useEffect, useId, useImperativeHandle, useRef, useState, type RefObject } from 'react';
import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import { SESSION_WARNINGS_TEXT } from '../../data/generated/SessionWarningsText';
import { Modal } from '../../kit/Modal';
import { DetachedDiagnosis, DiagnoseButton, DiagnosisPanel } from '../session-diagnosis';
import { useDiagnose } from '../useSessionDiagnosis';
import {
  WarningLine,
  WarningRow,
  useSessionWarning,
  useWarningDismissals,
  warningHref,
} from '../session-warnings';

export type SelectedWarning = {
  warning: SessionWarningItem;
  name: string;
  sessionId: string;
  timezone: string;
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
  const card = useRef<HTMLDivElement>(null);
  const rowProps = useWarningDismissals(selected.sessionId, read, card);
  const diagnose = useDiagnose(selected.sessionId, selected.timezone, read?.warnings);
  const running = diagnose?.stage?.kind === 'running';
  const [closing, setClosing] = useState(false);
  const close = () => {
    if (!running) {
      diagnose?.notNow();
      onClose();
    } else if (!closing) {
      setClosing(true);
      void diagnose.stop().then((accepted) => {
        if (!accepted) setClosing(false);
      });
    }
  };
  useImperativeHandle(requestClose, () => close);
  // Stop's acknowledgement is not completion. Keep the modal and Stop until
  // the diagnosis promise settles, even when cancellation was accepted.
  useEffect(() => {
    if (closing && !running) onClose();
  }, [closing, running, onClose]);
  return (
    <>
      <header className="xt-dash-modal-header">
        <h2 id={title}>
          <span className="xt-warning-modal-dot" aria-hidden="true" />
          {(warning ?? selected.warning).label}
        </h2>
        <button
          type="button"
          className="xt-button"
          data-variant="outline"
          tabIndex={0}
          aria-disabled={closing || undefined}
          onClick={close}
        >
          Close
        </button>
      </header>
      <p id={session} className="xt-warning-modal-session">
        {selected.name}
      </p>
      <p className="xt-warning-modal-day">
        {(warning ?? selected.warning).day}
        {dismissed ? ' · Dismissed' : ''}
      </p>
      <div ref={card}>
        {warning ? (
          <ol className="xt-warnings-list" data-dismissed={dismissed ? '' : undefined}>
            <WarningRow
              warning={warning}
              undo={Boolean(dismissed)}
              highlighted={false}
              summary={<p className="xt-warning-modal-reason">{warning.reason}</p>}
              context={
                <WarningContext
                  sessionId={selected.sessionId}
                  warning={warning}
                  timezone={selected.timezone}
                  fullSession={{
                    href: warningHref(selected.sessionId, warning.id, carry),
                    disabled: Boolean(running || closing),
                    open: () => {
                      if (running || closing) return;
                      diagnose?.notNow();
                      onClose();
                      navigate(warningHref(selected.sessionId, warning.id, carry));
                    },
                  }}
                />
              }
              {...rowProps(warning, Boolean(dismissed))}
              actions={
                active && diagnose ? (
                  <DiagnoseButton diagnose={diagnose} warning={warning} />
                ) : undefined
              }
              below={
                diagnose ? <DiagnosisPanel diagnose={diagnose} warning={warning} /> : undefined
              }
            />
          </ol>
        ) : (
          <>
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
          </>
        )}
        {!warning && diagnose && <DetachedDiagnosis diagnose={diagnose} warnings={[]} />}
      </div>
    </>
  );
}
