// Joining in two steps: POST /api/join emails a 6-digit code, and
// POST /api/join/verify turns the right code into a member key.
import type { Clock, Query } from './db.js';
import { fail, json } from './http.js';
import { hashCode, newCode, newKey, newPendingId } from './keys.js';
import type { Mailer } from './mail.js';
import type { Nickname } from './validate.js';

export const CODE_LIFETIME_MINUTES = 15;
export const MAX_CODE_ATTEMPTS = 5;
export const CODE_FORMAT = /^[0-9]{6}$/;

export type JoinContext = {
  query: Query;
  clock: Clock;
  mailer: Mailer;
  log: (message: string) => void;
};

/** Step 1. The body has been validated and the join limits counted. */
export async function startJoin(
  { query, clock, mailer, log }: JoinContext,
  nickname: Nickname,
  email: string,
): Promise<Response> {
  const now = clock();
  // Nickname availability is not checked here: the answer must be the same
  // whether or not the email already belongs to a member. Verify checks it.
  const id = newPendingId();
  const code = newCode();
  const expires = new Date(now.getTime() + CODE_LIFETIME_MINUTES * 60_000);
  // Earlier codes for this email stay valid until they expire or use up their
  // attempts, so someone else's joins cannot cancel the owner's code.
  await query(
    `insert into pending_joins (id, nickname, nickname_key, email, code_hash, expires_at, created_at)
     values ($1, $2, $3, $4, decode($5, 'hex'), $6::timestamptz, $7::timestamptz)`,
    [
      id,
      nickname.nickname,
      nickname.nicknameKey,
      email,
      hashCode(code).toString('hex'),
      expires.toISOString(),
      now.toISOString(),
    ],
  );
  try {
    await mailer.sendCode(email, code);
  } catch (error) {
    await query('delete from pending_joins where id = $1', [id]);
    log(`leaderboard: email not sent (${error instanceof Error ? error.name : 'unknown error'})`);
    return fail(503, 'email_unavailable');
  }
  return json(202, { pending_id: id });
}

/** Step 2. The body has been validated (code is 6 digits) and the limit counted. */
export async function verifyJoin(
  { query, clock }: JoinContext,
  pendingId: string,
  code: string,
  nicknameOverride: Nickname | null,
): Promise<Response> {
  const now = clock().toISOString();
  const codeHash = hashCode(code).toString('hex');
  const live = `id = $1 and expires_at > $2::timestamptz and attempts < ${MAX_CODE_ATTEMPTS}`;

  // Only wrong codes count. The comparison and the count are one statement, so
  // concurrent guesses can never count past the limit or slip a guess in after it.
  const wrong = await query(
    `update pending_joins set attempts = attempts + 1
     where ${live} and code_hash <> decode($3, 'hex')
     returning attempts`,
    [pendingId, now, codeHash],
  );
  if (wrong[0]) {
    if (Number(wrong[0].attempts) >= MAX_CODE_ATTEMPTS)
      await query('delete from pending_joins where id = $1', [pendingId]);
    return fail(400, 'invalid_code');
  }

  // The right code: claim the pending join. Only one request can delete it.
  const claimed = await query(
    `delete from pending_joins where ${live} and code_hash = decode($3, 'hex')
     returning id, nickname, nickname_key, email, encode(code_hash, 'hex') as code_hash,
               attempts, expires_at::text as expires_at, created_at::text as created_at`,
    [pendingId, now, codeHash],
  );
  const pending = claimed[0];
  if (!pending) {
    // Unknown, expired or used up. Remove it only if it really is dead: a live
    // row here was just put back by a racing 409 and must stay.
    await query(
      `delete from pending_joins
       where id = $1 and (expires_at <= $2::timestamptz or attempts >= ${MAX_CODE_ATTEMPTS})`,
      [pendingId, now],
    );
    return fail(410, 'code_expired');
  }

  const email = String(pending.email);
  const nickname = nicknameOverride ?? {
    nickname: String(pending.nickname),
    nicknameKey: String(pending.nickname_key),
  };
  const { key, keyHashHex } = newKey();
  const signBackIn = async () => {
    const rows = await query(
      `update members set key_hash = decode($1, 'hex') where email = $2 returning nickname`,
      [keyHashHex, email],
    );
    return rows[0] ? json(201, { key, nickname: rows[0].nickname, rejoined: true }) : null;
  };
  // A member's email signs them back in; the nickname is ignored.
  const rejoined = await signBackIn();
  if (rejoined) return rejoined;
  const created = await query(
    `insert into members (nickname, nickname_key, email, key_hash)
     values ($1, $2, $3, decode($4, 'hex'))
     on conflict do nothing
     returning nickname`,
    [nickname.nickname, nickname.nicknameKey, email, keyHashHex],
  );
  if (created[0]) return json(201, { key, nickname: created[0].nickname, rejoined: false });
  // This email may have joined in the meantime.
  const late = await signBackIn();
  if (late) return late;
  // The nickname is taken. Put the pending join back with the same code, expiry
  // and attempts, and with the nickname just tried, so a later verify without
  // `nickname` uses it.
  await query(
    `insert into pending_joins (id, nickname, nickname_key, email, code_hash, attempts, expires_at, created_at)
     values ($1, $2, $3, $4, decode($5, 'hex'), $6, $7::timestamptz, $8::timestamptz)
     on conflict (id) do nothing`,
    [
      pending.id,
      nickname.nickname,
      nickname.nicknameKey,
      email,
      pending.code_hash,
      pending.attempts,
      pending.expires_at,
      pending.created_at,
    ],
  );
  return fail(409, 'nickname_taken');
}
