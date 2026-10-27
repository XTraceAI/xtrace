// Joining in two steps: POST /api/join emails a 6-digit code, and
// POST /api/join/verify turns the right code into a member key.
import type { Clock, Query } from './db.js';
import { fail, json } from './http.js';
import { hashCode, newCode, newKey, newPendingId, sameHash } from './keys.js';
import { CLEANUP_AFTER_SECONDS } from './limits.js';
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

/** Removes counter rows and expired pending joins older than a day. */
async function cleanUp(query: Query, now: Date): Promise<void> {
  const cutoff = new Date(now.getTime() - CLEANUP_AFTER_SECONDS * 1000).toISOString();
  await query('delete from rate_counters where window_start < $1::timestamptz', [cutoff]);
  await query('delete from pending_joins where expires_at < $1::timestamptz', [cutoff]);
}

/** Step 1. The body has been validated and the join limits counted. */
export async function startJoin(
  { query, clock, mailer, log }: JoinContext,
  nickname: Nickname,
  email: string,
): Promise<Response> {
  const now = clock();
  await cleanUp(query, now);
  const member = await query('select 1 from members where email = $1', [email]);
  // A member's email signs them back in, so their typed nickname is ignored.
  if (member.length === 0) {
    const taken = await query('select 1 from members where nickname_key = $1', [
      nickname.nicknameKey,
    ]);
    if (taken.length > 0) return fail(409, 'nickname_taken');
  }
  const id = newPendingId();
  const code = newCode();
  const expires = new Date(now.getTime() + CODE_LIFETIME_MINUTES * 60_000);
  // Only the newest code for an email works.
  await query('delete from pending_joins where email = $1', [email]);
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
): Promise<Response> {
  const now = clock().toISOString();
  // Claiming an attempt first keeps the 5-attempt rule exact under concurrent requests.
  const claimed = await query(
    `update pending_joins set attempts = attempts + 1
     where id = $1 and expires_at > $2::timestamptz and attempts < $3
     returning nickname, nickname_key, email, encode(code_hash, 'hex') as code_hash, attempts`,
    [pendingId, now, MAX_CODE_ATTEMPTS],
  );
  const pending = claimed[0];
  if (!pending) {
    await query('delete from pending_joins where id = $1', [pendingId]);
    return fail(410, 'code_expired');
  }
  if (!sameHash(hashCode(code), Buffer.from(String(pending.code_hash), 'hex'))) {
    if (Number(pending.attempts) >= MAX_CODE_ATTEMPTS)
      await query('delete from pending_joins where id = $1', [pendingId]);
    return fail(400, 'invalid_code');
  }
  // Single use: only the request that deletes the row goes on.
  const used = await query('delete from pending_joins where id = $1 returning id', [pendingId]);
  if (used.length === 0) return fail(410, 'code_expired');

  const email = String(pending.email);
  const { key, keyHashHex } = newKey();
  const signBackIn = async () => {
    const rows = await query(
      `update members set key_hash = decode($1, 'hex') where email = $2 returning nickname`,
      [keyHashHex, email],
    );
    return rows[0] ? json(201, { key, nickname: rows[0].nickname, rejoined: true }) : null;
  };
  const rejoined = await signBackIn();
  if (rejoined) return rejoined;
  const created = await query(
    `insert into members (nickname, nickname_key, email, key_hash)
     values ($1, $2, $3, decode($4, 'hex'))
     on conflict do nothing
     returning nickname`,
    [pending.nickname, pending.nickname_key, email, keyHashHex],
  );
  if (created[0]) return json(201, { key, nickname: created[0].nickname, rejoined: false });
  // Someone else took the nickname, or this email joined, in the meantime.
  return (await signBackIn()) ?? fail(409, 'nickname_taken');
}
