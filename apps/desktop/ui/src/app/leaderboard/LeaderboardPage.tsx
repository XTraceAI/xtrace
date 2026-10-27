import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useRef, useState, type FormEvent, type RefObject } from 'react';
import { useData } from '../../data/DataProvider';
import type { LeaderboardControls } from '../../data/DataSource';
import type { LeaderboardStatus } from '../../data/generated/LeaderboardStatus';
import { queryKeys } from '../../data/query-client';
import { Button } from '../../kit/Button';
import { clock } from '../../kit/clock';
import { Modal, ModalClose } from '../../kit/Modal';
import { SectionCard } from '../../kit/SectionCard';
import {
  JOIN_TEXT,
  LEAVE_BACKUPS,
  LEAVE_QUESTION,
  LEAVE_TEXT,
  leaderboardNumber,
  refusalField,
  refusalText,
  standingText,
} from './leaderboard';
import '../../styles/settings.css';
import '../../styles/leaderboard.css';

export function LeaderboardPage() {
  const { source } = useData();
  // A finished change moves focus here: the form or button that started it
  // is gone or replaced.
  const heading = useRef<HTMLHeadingElement>(null);
  return (
    <section className="xt-placeholder xt-settings xt-leaderboard">
      <h1 ref={heading} tabIndex={-1}>
        Leaderboard
      </h1>
      {source.leaderboard ? (
        <Leaderboard controls={source.leaderboard} heading={heading} />
      ) : (
        <SectionCard title="Public leaderboard">
          <p className="xt-settings-note">{JOIN_TEXT}</p>
          <p className="xt-settings-note">
            Joining needs the desktop app. This preview sends nothing.
          </p>
        </SectionCard>
      )}
    </section>
  );
}

/**
 * The status and its changes. Every change shows only the status the app read
 * back afterwards; a refused one shows the leaderboard's answer and re-reads.
 */
function useLeaderboard(
  controls: LeaderboardControls,
  heading: RefObject<HTMLHeadingElement | null>,
) {
  const client = useQueryClient();
  const status = useQuery({ queryKey: queryKeys.leaderboard, queryFn: () => controls.read() });
  const saved = (next: LeaderboardStatus) => {
    client.setQueryData(queryKeys.leaderboard, next);
    // After the new state renders.
    requestAnimationFrame(() => heading.current?.focus());
  };
  const reread = () => void client.invalidateQueries({ queryKey: queryKeys.leaderboard });
  const change = <T,>(run: (input: T) => Promise<LeaderboardStatus>) => ({
    mutationFn: run,
    onSuccess: saved,
    onError: reread,
  });
  const join = useMutation(
    change(({ nickname, email }: { nickname: string; email: string }) =>
      controls.join(nickname, email),
    ),
  );
  const rename = useMutation(change((nickname: string) => controls.rename(nickname)));
  const leave = useMutation(change(() => controls.leave()));
  return { status, join, rename, leave };
}

function Leaderboard({
  controls,
  heading,
}: {
  controls: LeaderboardControls;
  heading: RefObject<HTMLHeadingElement | null>;
}) {
  const { status, join, rename, leave } = useLeaderboard(controls, heading);
  const current = status.data;
  if (!current)
    return (
      <SectionCard title="Public leaderboard">
        {status.isError ? (
          <p role="alert">The leaderboard status could not be read.</p>
        ) : (
          <p className="xt-settings-note">Reading…</p>
        )}
      </SectionCard>
    );
  if (!current.available)
    return (
      <SectionCard title="Public leaderboard">
        <p className="xt-settings-note">{JOIN_TEXT}</p>
        <p className="xt-settings-note" data-testid="leaderboard-unavailable">
          {current.unavailable_reason}
        </p>
      </SectionCard>
    );
  return current.joined ? (
    <Membership
      status={current}
      controls={controls}
      rename={rename}
      leave={leave}
      heading={heading}
    />
  ) : (
    <JoinForm status={current} join={join} />
  );
}

type Changes = ReturnType<typeof useLeaderboard>;

function JoinForm({ status, join }: { status: LeaderboardStatus; join: Changes['join'] }) {
  const [nickname, setNickname] = useState('');
  const [email, setEmail] = useState('');
  const nicknameId = useId();
  const emailId = useId();
  const errorId = useId();
  const failed = join.isError ? refusalText(join.error) : null;
  const field = join.isError ? refusalField(join.error) : null;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!join.isPending) join.mutate({ nickname, email });
  };
  return (
    <SectionCard title="Join the public leaderboard">
      <p className="xt-settings-note">{JOIN_TEXT}</p>
      {status.notice && (
        <p role="status" data-testid="leaderboard-notice">
          {status.notice}
        </p>
      )}
      {/* The leaderboard checks the nickname and email; the app does not. */}
      <form className="xt-leaderboard-form" onSubmit={submit} noValidate>
        <label htmlFor={nicknameId}>Nickname</label>
        <input
          id={nicknameId}
          type="text"
          autoComplete="nickname"
          spellCheck={false}
          value={nickname}
          aria-invalid={field === 'nickname' || undefined}
          aria-describedby={failed ? errorId : undefined}
          onChange={(event) => setNickname(event.target.value)}
        />
        <label htmlFor={emailId}>Email</label>
        <input
          id={emailId}
          type="email"
          autoComplete="email"
          spellCheck={false}
          value={email}
          aria-invalid={field === 'email' || undefined}
          aria-describedby={failed ? errorId : undefined}
          onChange={(event) => setEmail(event.target.value)}
        />
        <div className="xt-leaderboard-actions">
          <Button type="submit" variant="primary" height={28} disabled={join.isPending}>
            {join.isPending ? 'Joining…' : 'Join'}
          </Button>
        </div>
      </form>
      {failed && (
        <p id={errorId} role="alert">
          {failed}
        </p>
      )}
    </SectionCard>
  );
}

function Membership({
  status,
  controls,
  rename,
  leave,
  heading,
}: {
  status: LeaderboardStatus;
  controls: LeaderboardControls;
  rename: Changes['rename'];
  leave: Changes['leave'];
  heading: RefObject<HTMLHeadingElement | null>;
}) {
  const [renaming, setRenaming] = useState(false);
  const [openFailed, setOpenFailed] = useState(false);
  const host = new URL(status.page_url).host;
  return (
    <SectionCard
      title="You are on the leaderboard"
      right={
        <Button
          variant="outline"
          height={28}
          onClick={() => {
            setOpenFailed(false);
            controls.openPage().catch(() => setOpenFailed(true));
          }}
        >
          Open {host}
        </Button>
      }
    >
      <dl className="xt-database-summary" data-testid="leaderboard-summary">
        <dt>Nickname</dt>
        <dd>{status.nickname}</dd>
        <dt>Leverage, 7 days</dt>
        <dd>{status.last_sent ? leaderboardNumber(status.last_sent.leverage_tenths) : '—'}</dd>
        <dt>Rank</dt>
        <dd>{status.standing ? standingText(status.standing) : '—'}</dd>
        <dt>Last sent</dt>
        <dd>
          {status.last_sent ? clock(status.last_sent.at_ms, { date: 'day' }) : 'Not sent yet'}
        </dd>
      </dl>
      <p className="xt-settings-note">
        The app sends your number when it starts and every hour while it runs.
      </p>
      {status.error && <p role="alert">{status.error}</p>}
      {openFailed && <p role="alert">The leaderboard page could not be opened in your browser.</p>}
      {renaming ? (
        <RenameForm
          current={status.nickname ?? ''}
          rename={rename}
          onDone={() => setRenaming(false)}
        />
      ) : (
        <div className="xt-leaderboard-actions">
          <Button
            variant="outline"
            height={28}
            onClick={() => {
              rename.reset();
              setRenaming(true);
            }}
          >
            Change nickname
          </Button>
          <LeaveButton leave={leave} heading={heading} />
        </div>
      )}
    </SectionCard>
  );
}

function RenameForm({
  current,
  rename,
  onDone,
}: {
  current: string;
  rename: Changes['rename'];
  onDone: () => void;
}) {
  const [nickname, setNickname] = useState(current);
  const inputId = useId();
  const errorId = useId();
  const failed = rename.isError ? refusalText(rename.error) : null;
  const invalid = rename.isError && refusalField(rename.error) === 'nickname';
  return (
    <>
      <form
        className="xt-leaderboard-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          if (!rename.isPending) rename.mutate(nickname, { onSuccess: onDone });
        }}
      >
        <label htmlFor={inputId}>New nickname</label>
        <input
          id={inputId}
          type="text"
          autoComplete="nickname"
          spellCheck={false}
          value={nickname}
          // Opened by its own button, so the field takes the focus.
          autoFocus
          aria-invalid={invalid || undefined}
          aria-describedby={failed ? errorId : undefined}
          onChange={(event) => setNickname(event.target.value)}
        />
        <div className="xt-leaderboard-actions">
          <Button type="submit" variant="primary" height={28} disabled={rename.isPending}>
            {rename.isPending ? 'Saving…' : 'Save nickname'}
          </Button>
          <Button variant="outline" height={28} disabled={rename.isPending} onClick={onDone}>
            Cancel
          </Button>
        </div>
      </form>
      {failed && (
        <p id={errorId} role="alert">
          {failed}
        </p>
      )}
    </>
  );
}

function LeaveButton({
  leave,
  heading,
}: {
  leave: Changes['leave'];
  heading: RefObject<HTMLHeadingElement | null>;
}) {
  const [open, setOpen] = useState(false);
  // Cancel returns to Leave; leaving replaces it, so focus goes to the heading.
  const [left, setLeft] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const title = useId();
  return (
    <>
      <Button
        ref={trigger}
        variant="outline"
        height={28}
        onClick={() => {
          leave.reset();
          setOpen(true);
        }}
      >
        Leave leaderboard
      </Button>
      <Modal
        open={open}
        onOpenChange={(next) => {
          if (!leave.isPending) setOpen(next);
        }}
        returnFocusRef={left ? heading : trigger}
        aria-labelledby={title}
        className="xt-settings-modal"
      >
        <h2 id={title}>{LEAVE_QUESTION}</h2>
        <p>{LEAVE_TEXT}</p>
        <p>{LEAVE_BACKUPS}</p>
        {leave.isError && <p role="alert">{refusalText(leave.error)}</p>}
        <div className="xt-settings-actions">
          <ModalClose
            className="xt-button"
            data-variant="outline"
            style={{ height: 28 }}
            disabled={leave.isPending}
          >
            Cancel
          </ModalClose>
          <Button
            variant="primary"
            height={28}
            disabled={leave.isPending}
            onClick={() =>
              leave.mutate(undefined, {
                onSuccess: () => {
                  setLeft(true);
                  setOpen(false);
                },
              })
            }
          >
            {leave.isPending ? 'Leaving…' : 'Leave leaderboard'}
          </Button>
        </div>
      </Modal>
    </>
  );
}
