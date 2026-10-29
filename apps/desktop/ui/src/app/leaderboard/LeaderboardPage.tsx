import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { useData } from '../../data/DataProvider';
import type { LeaderboardControls } from '../../data/DataSource';
import type { LeaderboardRow } from '../../data/generated/LeaderboardRow';
import type { LeaderboardStatus } from '../../data/generated/LeaderboardStatus';
import { queryKeys } from '../../data/query-client';
import { Button } from '../../kit/Button';
import { clock } from '../../kit/clock';
import { Modal, ModalClose } from '../../kit/Modal';
import { SectionCard } from '../../kit/SectionCard';
import {
  EMAIL_PRIVATE,
  JOIN_TEXT,
  JOIN_TITLE,
  LEAVE_BACKUPS,
  SAMPLE_ROWS,
  SHARED_FACTS,
  rankMedal,
  SHARED_SUMMARY,
  TABLE_EMPTY,
  TABLE_READING,
  TABLE_TITLE,
  TABLE_UNREADABLE,
  TABLE_UPDATES,
  YOU_LABEL,
  codeSentText,
  countryFlag,
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
        <Locked>
          <SectionCard title={JOIN_TITLE}>
            <p className="xt-settings-note">{JOIN_TEXT}</p>
            <p className="xt-settings-note">{EMAIL_PRIVATE}</p>
            <p className="xt-settings-note">
              Joining needs the desktop app. This preview sends nothing.
            </p>
            <SharedDetails />
          </SectionCard>
        </Locked>
      )}
    </section>
  );
}

/**
 * The page before joining: the list it unlocks drawn blurred behind the card,
 * from made-up rows. The drawing is decoration only: hidden from assistive
 * technology, out of the tab order, and not clickable or selectable.
 */
function Locked({ children }: { children: ReactNode }) {
  return (
    <div className="xt-leaderboard-locked">
      <div
        className="xt-leaderboard-backdrop"
        aria-hidden="true"
        inert
        data-testid="leaderboard-sample"
      >
        <div className="xt-leaderboard-sample">
          <div className="xt-leaderboard-sample-title">{TABLE_TITLE}</div>
          <div className="xt-leaderboard-sample-panel">
            <PublicTable rows={SAMPLE_ROWS} />
          </div>
        </div>
      </div>
      <div className="xt-leaderboard-gate">{children}</div>
    </div>
  );
}

/** The rest of what joining shares, folded under the join card's one line. */
function SharedDetails() {
  return (
    <details className="xt-leaderboard-shared">
      <summary>{SHARED_SUMMARY}</summary>
      <ul>
        {SHARED_FACTS.map((fact) => (
          <li key={fact}>{fact}</li>
        ))}
      </ul>
    </details>
  );
}

/** The rank, with a medal before the top three; the medal is decoration. */
function RankCell({ rank }: { rank: number }) {
  const medal = rankMedal(rank);
  return (
    <td>
      {medal && (
        <span className="xt-leaderboard-medal" aria-hidden="true">
          {medal}
        </span>
      )}
      {rank}
    </td>
  );
}

/** The member's flag before their nickname, named by its code; nothing without a country. */
function CountryFlag({ country }: { country: string | null }) {
  const flag = countryFlag(country);
  if (flag === null || country === null) return null;
  return (
    <span className="xt-leaderboard-flag" role="img" aria-label={country} title={country}>
      {flag}
    </span>
  );
}

/** Rank, flag and nickname, and 7-day leverage, with the member's own row marked. */
function PublicTable({
  rows,
  label,
  note,
}: {
  rows: readonly LeaderboardRow[];
  label?: string;
  /** Said in a row of its own while there are no rows yet. */
  note?: string;
}) {
  return (
    <table className="xt-leaderboard-table" aria-label={label} aria-busy={note ? true : undefined}>
      <thead>
        <tr>
          <th scope="col">Rank</th>
          <th scope="col">Nickname</th>
          <th scope="col">Leverage, 7 days</th>
        </tr>
      </thead>
      <tbody>
        {note && (
          <tr>
            <td colSpan={3} className="xt-leaderboard-table-note">
              {note}
            </td>
          </tr>
        )}
        {rows.map((row) => (
          <tr key={`${row.rank}-${row.nickname}`} data-you={row.is_you || undefined}>
            <RankCell rank={row.rank} />
            <td>
              <CountryFlag country={row.country} />
              {row.nickname}
              {row.is_you && (
                <>
                  <span className="xt-leaderboard-you" aria-hidden="true">
                    You
                  </span>
                  <span className="sr-only"> {YOU_LABEL}</span>
                </>
              )}
            </td>
            <td>{leaderboardNumber(row.leverage_tenths)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * The public list while joined, in its own card so a failed read never hides
 * the membership controls above it, with the public page's link in its
 * header. It is read again with the status.
 */
function PublicBoard({ controls, pageUrl }: { controls: LeaderboardControls; pageUrl: string }) {
  const [openFailed, setOpenFailed] = useState(false);
  const table = useQuery({
    queryKey: queryKeys.leaderboardTable,
    queryFn: () => controls.table(),
  });
  const data = table.data;
  // The leaderboard may still serve a list from before the last change: the
  // native side says when it surely shows it, and the list is read once more
  // then. Each answer sets at most one timer.
  const refreshAfter = data?.refresh_after_seconds ?? null;
  const { refetch, dataUpdatedAt } = table;
  useEffect(() => {
    if (refreshAfter === null) return;
    const timer = setTimeout(() => void refetch(), refreshAfter * 1000);
    return () => clearTimeout(timer);
  }, [refreshAfter, dataUpdatedAt, refetch]);
  return (
    <SectionCard
      title={TABLE_TITLE}
      right={
        <Button
          variant="outline"
          height={28}
          onClick={() => {
            setOpenFailed(false);
            controls.openPage().catch(() => setOpenFailed(true));
          }}
        >
          Open {new URL(pageUrl).host}
        </Button>
      }
    >
      {openFailed && <p role="alert">The leaderboard page could not be opened in your browser.</p>}
      {table.isError && <p role="alert">{TABLE_UNREADABLE}</p>}
      {data === undefined ? (
        !table.isError && <PublicTable rows={[]} label={TABLE_TITLE} note={TABLE_READING} />
      ) : data.state === 'error' ? (
        <p role="alert">{data.reason}</p>
      ) : data.state === 'unavailable' ? (
        <p className="xt-settings-note">{data.reason}</p>
      ) : data.rows.length === 0 ? (
        <p className="xt-settings-note">{TABLE_EMPTY}</p>
      ) : (
        <PublicTable rows={data.rows} label={TABLE_TITLE} />
      )}
      {/* Shown while reading too, so the card keeps its shape when the rows arrive. */}
      {(data === undefined ? !table.isError : data.state === 'ok') && (
        <p className="xt-settings-note">{TABLE_UPDATES}</p>
      )}
    </SectionCard>
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
  // The public list is read again with every status the app reads back.
  const rereadTable = () => void client.invalidateQueries({ queryKey: queryKeys.leaderboardTable });
  const saved = (next: LeaderboardStatus, focusHeading: boolean) => {
    client.setQueryData(queryKeys.leaderboard, next);
    if (next.joined) rereadTable();
    // After the new state renders. The code step and the form focus their
    // own first field instead.
    if (focusHeading) requestAnimationFrame(() => heading.current?.focus());
  };
  const reread = () => {
    void client.invalidateQueries({ queryKey: queryKeys.leaderboard });
    rereadTable();
  };
  const change = <T,>(run: (input: T) => Promise<LeaderboardStatus>, focusHeading = true) => ({
    mutationFn: run,
    onSuccess: (next: LeaderboardStatus) => saved(next, focusHeading),
    onError: reread,
  });
  const join = useMutation(
    change(
      ({ nickname, email }: { nickname: string; email: string }) => controls.join(nickname, email),
      false,
    ),
  );
  const confirm = useMutation(
    change(({ code, nickname }: { code: string; nickname?: string }) =>
      controls.confirm(code, nickname),
    ),
  );
  const cancelCode = useMutation(change(() => controls.cancelCode(), false));
  const rename = useMutation(change((nickname: string) => controls.rename(nickname)));
  const leave = useMutation(change(() => controls.leave()));
  // Not joined (left, or the leaderboard no longer has the membership): no
  // earlier answer about the public list may show after the next join.
  const joined = status.data?.joined;
  useEffect(() => {
    if (joined === false) client.removeQueries({ queryKey: queryKeys.leaderboardTable });
  }, [joined, client]);
  return { status, join, confirm, cancelCode, rename, leave };
}

function Leaderboard({
  controls,
  heading,
}: {
  controls: LeaderboardControls;
  heading: RefObject<HTMLHeadingElement | null>;
}) {
  const { status, join, confirm, cancelCode, rename, leave } = useLeaderboard(controls, heading);
  const current = status.data;
  // Nothing is drawn while the status is read, so the page appears once in
  // its final shape instead of swapping a placeholder for it.
  if (!current)
    return status.isError ? <p role="alert">The leaderboard status could not be read.</p> : null;
  if (!current.available)
    return (
      <Locked>
        <SectionCard title={JOIN_TITLE}>
          <p className="xt-settings-note">{JOIN_TEXT}</p>
          <p className="xt-settings-note" data-testid="leaderboard-unavailable">
            {current.unavailable_reason}
          </p>
          <SharedDetails />
        </SectionCard>
      </Locked>
    );
  return current.joined ? (
    <>
      <Membership
        status={current}
        // Only the code's own answer carries the welcome; later status reads
        // leave it on the page.
        welcome={confirm.data?.welcome ?? null}
        rename={rename}
        leave={leave}
        heading={heading}
      />
      <PublicBoard controls={controls} pageUrl={current.page_url} />
    </>
  ) : (
    <Locked>
      <NotJoined status={current} join={join} confirm={confirm} cancelCode={cancelCode} />
    </Locked>
  );
}

type Changes = ReturnType<typeof useLeaderboard>;

/**
 * Joining in two steps: the form, then the emailed code. What was typed stays
 * here across both, so "Use a different email" comes back with the nickname;
 * after a restart of the page it comes from the code the app still holds.
 */
function NotJoined({
  status,
  join,
  confirm,
  cancelCode,
}: {
  status: LeaderboardStatus;
  join: Changes['join'];
  confirm: Changes['confirm'];
  cancelCode: Changes['cancelCode'];
}) {
  const [nickname, setNickname] = useState(status.code_sent?.nickname ?? '');
  const [email, setEmail] = useState(status.code_sent?.email ?? '');
  const sent = status.code_sent;
  if (sent) return <CodeStep sent={sent} join={join} confirm={confirm} cancelCode={cancelCode} />;
  // A refusal of the code step that ended it (the nickname was taken
  // meanwhile) is shown on the form it returns to.
  const refused = join.isError ? join.error : confirm.isError ? confirm.error : null;
  return (
    <JoinForm
      status={status}
      join={join}
      onJoin={() => {
        confirm.reset();
        join.mutate({ nickname, email });
      }}
      refused={refused}
      nickname={nickname}
      email={email}
      setNickname={setNickname}
      setEmail={setEmail}
      // Back from the code step: the email is the field to change.
      focusEmail={cancelCode.isSuccess}
    />
  );
}

function JoinForm({
  status,
  join,
  onJoin,
  refused,
  nickname,
  email,
  setNickname,
  setEmail,
  focusEmail,
}: {
  status: LeaderboardStatus;
  join: Changes['join'];
  onJoin: () => void;
  refused: unknown;
  nickname: string;
  email: string;
  setNickname: (value: string) => void;
  setEmail: (value: string) => void;
  focusEmail: boolean;
}) {
  const nicknameId = useId();
  const emailId = useId();
  const errorId = useId();
  const failed = refused ? refusalText(refused) : null;
  const field = refused ? refusalField(refused) : null;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!join.isPending) onJoin();
  };
  return (
    <SectionCard title={JOIN_TITLE}>
      <p className="xt-settings-note">{JOIN_TEXT}</p>
      <p className="xt-settings-note">{EMAIL_PRIVATE}</p>
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
          autoFocus={focusEmail}
          aria-invalid={field === 'email' || undefined}
          aria-describedby={failed ? errorId : undefined}
          onChange={(event) => setEmail(event.target.value)}
        />
        <div className="xt-leaderboard-actions">
          <Button type="submit" variant="primary" height={28} disabled={join.isPending}>
            {join.isPending ? 'Sending a code…' : 'Join'}
          </Button>
        </div>
      </form>
      {failed && (
        <p id={errorId} role="alert">
          {failed}
        </p>
      )}
      <SharedDetails />
    </SectionCard>
  );
}

function CodeStep({
  sent,
  join,
  confirm,
  cancelCode,
}: {
  sent: NonNullable<LeaderboardStatus['code_sent']>;
  join: Changes['join'];
  confirm: Changes['confirm'];
  cancelCode: Changes['cancelCode'];
}) {
  const [code, setCode] = useState('');
  const [nickname, setNickname] = useState(sent.nickname);
  const [resent, setResent] = useState(false);
  const codeId = useId();
  const nicknameId = useId();
  const sentId = useId();
  const errorId = useId();
  const refused = confirm.isError ? confirm.error : join.isError ? join.error : null;
  const failed = refused ? refusalText(refused) : null;
  const field = refused ? refusalField(refused) : null;
  // The code was right but the nickname is taken: the same code is
  // confirmed again with another nickname.
  const chooseNickname = sent.choose_nickname || field === 'nickname';
  const busy = confirm.isPending || join.isPending || cancelCode.isPending;
  return (
    <SectionCard title="Check your email">
      <p id={sentId} data-testid="leaderboard-code-sent">
        {codeSentText(sent.email)}
      </p>
      <form
        className="xt-leaderboard-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          if (busy) return;
          setResent(false);
          join.reset();
          confirm.mutate({ code, nickname: chooseNickname ? nickname : undefined });
        }}
      >
        <label htmlFor={codeId}>Code</label>
        <input
          id={codeId}
          type="text"
          inputMode="numeric"
          autoComplete="one-time-code"
          spellCheck={false}
          value={code}
          // The step replaced the form, so its one field takes the focus.
          autoFocus
          aria-invalid={field === 'code' || undefined}
          // Where the code went, and the refusal when there is one.
          aria-describedby={failed ? `${sentId} ${errorId}` : sentId}
          onChange={(event) => setCode(event.target.value)}
        />
        {chooseNickname && (
          <>
            <label htmlFor={nicknameId}>Nickname</label>
            <input
              id={nicknameId}
              type="text"
              autoComplete="nickname"
              spellCheck={false}
              value={nickname}
              // Shown because the nickname must change: it takes the focus.
              autoFocus
              aria-invalid={field === 'nickname' || (sent.choose_nickname && !refused) || undefined}
              aria-describedby={failed ? errorId : undefined}
              onChange={(event) => setNickname(event.target.value)}
            />
          </>
        )}
        <div className="xt-leaderboard-actions">
          <Button type="submit" variant="primary" height={28} disabled={busy}>
            {confirm.isPending ? 'Checking…' : 'Confirm'}
          </Button>
          <Button
            variant="outline"
            height={28}
            disabled={busy}
            onClick={() => {
              setResent(false);
              confirm.reset();
              join.mutate(
                // While choosing a nickname, the one typed now.
                { nickname: chooseNickname ? nickname : sent.nickname, email: sent.email },
                { onSuccess: () => setResent(true) },
              );
            }}
          >
            {join.isPending ? 'Sending…' : 'Send a new code'}
          </Button>
          <Button
            variant="outline"
            height={28}
            disabled={busy}
            onClick={() => {
              confirm.reset();
              join.reset();
              cancelCode.mutate();
            }}
          >
            Use a different email
          </Button>
        </div>
      </form>
      {resent && !failed && <p role="status">A new code was sent. Only the newest code works.</p>}
      {failed && (
        <p id={errorId} role="alert">
          {failed}
        </p>
      )}
    </SectionCard>
  );
}

/**
 * The member's standing in one strip above the list: medal, rank, nickname,
 * number and when it was sent, with the changes beside them.
 */
function Membership({
  status,
  welcome,
  rename,
  leave,
  heading,
}: {
  status: LeaderboardStatus;
  welcome: string | null;
  rename: Changes['rename'];
  leave: Changes['leave'];
  heading: RefObject<HTMLHeadingElement | null>;
}) {
  const [renaming, setRenaming] = useState(false);
  const titleId = useId();
  const medal = status.standing?.state === 'ranked' ? rankMedal(status.standing.rank) : null;
  return (
    <section className="xt-leaderboard-standing" aria-labelledby={titleId}>
      <h2 id={titleId} className="sr-only">
        You are on the leaderboard
      </h2>
      <div className="xt-leaderboard-standing-row">
        {medal && (
          <span className="xt-leaderboard-standing-medal" aria-hidden="true">
            {medal}
          </span>
        )}
        <dl className="xt-leaderboard-standing-facts" data-testid="leaderboard-summary">
          <div>
            <dt>Rank</dt>
            <dd>{status.standing ? standingText(status.standing) : '—'}</dd>
          </div>
          <div>
            <dt>Nickname</dt>
            <dd>{status.nickname}</dd>
          </div>
          <div>
            <dt>Leverage, 7 days</dt>
            <dd>{status.last_sent ? leaderboardNumber(status.last_sent.leverage_tenths) : '—'}</dd>
          </div>
          <div>
            <dt>Last sent</dt>
            <dd>
              {status.last_sent ? clock(status.last_sent.at_ms, { date: 'day' }) : 'Not sent yet'}
            </dd>
          </div>
        </dl>
        {!renaming && (
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
      </div>
      {renaming && (
        <RenameForm
          current={status.nickname ?? ''}
          rename={rename}
          onDone={() => setRenaming(false)}
        />
      )}
      {welcome && (
        <p role="status" data-testid="leaderboard-welcome">
          {welcome}
        </p>
      )}
      {status.error && <p role="alert">{status.error}</p>}
    </section>
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
