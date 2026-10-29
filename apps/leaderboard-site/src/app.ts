// Every endpoint's behaviour. The files in api/ only forward to these handlers.
// Order inside a handler: the per-IP API limit, the body (its content type
// first), the key, then the endpoint's own limit. GET /api/leaderboard and the
// page are not limited.
import { requestCountry } from './country.js';
import { isUniqueViolation, type Clock, type Query } from './db.js';
import { fail, json, noContent, readJsonObject } from './http.js';
import { CODE_FORMAT, startJoin, verifyJoin } from './joining.js';
import { bearerKeyHash } from './keys.js';
import { cleanUp, emailSubject, hitLimit, ipSubject, LIMITS, type Limit } from './limits.js';
import { listedMembers, memberStanding } from './listing.js';
import { unavailableMailer, type Mailer } from './mail.js';
import { PAGE_CACHE_CONTROL, pageResponse } from './page.js';
import { parseEmail, parseLeverageTenths, parseNickname } from './validate.js';

export type Handler = (request: Request) => Promise<Response>;

export type App = {
  join: { POST: Handler };
  verify: { POST: Handler };
  score: { PUT: Handler };
  me: { GET: Handler; PATCH: Handler; DELETE: Handler };
  leaderboard: { GET: Handler };
  page: { GET: Handler; HEAD: Handler };
};

export type AppOptions = {
  query: Query;
  clock?: Clock;
  log?: (message: string) => void;
  mailer?: Mailer;
  /** Key for hashing client IPs. Missing → rate-limited endpoints answer 503. */
  ipHashSecret?: string | undefined;
};

/** 429 with the seconds until the window ends, in the body and in Retry-After. */
function rateLimited(retryAfterSeconds: number): Response {
  const response = json(429, { error: 'rate_limited', retry_after_seconds: retryAfterSeconds });
  response.headers.set('retry-after', String(retryAfterSeconds));
  return response;
}

/** Never logs request contents: emails and keys must not reach the logs. */
function guarded(handler: Handler, log: (message: string) => void): Handler {
  return async (request) => {
    try {
      return await handler(request);
    } catch (error) {
      const code =
        typeof error === 'object' &&
        error !== null &&
        'code' in error &&
        typeof error.code === 'string'
          ? ` (code ${error.code})`
          : '';
      const name = error instanceof Error ? error.name : 'unknown error';
      log(`leaderboard: request failed: ${name}${code}`);
      return fail(500, 'server_error');
    }
  };
}

export function createApp({
  query,
  clock = () => new Date(),
  log = (message) => console.error(message),
  mailer = unavailableMailer,
  ipHashSecret,
}: AppOptions): App {
  const joinContext = { query, clock, mailer, log };
  /** Seconds to wait when over `rule`, else null. */
  const limit = async (rule: Limit, subject?: string) =>
    (await hitLimit(query, clock(), rule, subject)).retryAfterSeconds;

  /**
   * The per-IP API limit, failing closed without a secret. Runs before anything
   * else. An IP's first request in each hour also removes old counters and
   * expired pending joins, so cleanup happens now and then on every endpoint.
   */
  const limitedByIp =
    (handler: (request: Request, ip: string, secret: string) => Promise<Response>): Handler =>
    async (request) => {
      if (!ipHashSecret) return fail(503, 'server_misconfigured');
      const ip = ipSubject(request, ipHashSecret);
      const hit = await hitLimit(query, clock(), LIMITS.api, ip);
      if (hit.retryAfterSeconds !== null) return rateLimited(hit.retryAfterSeconds);
      if (hit.count === 1) await cleanUp(query, clock());
      return handler(request, ip, ipHashSecret);
    };

  /** The member id for a key, or null. */
  const memberId = async (keyHash: string): Promise<string | null> => {
    const rows = await query(
      `select id::text as id from members where key_hash = decode($1, 'hex')`,
      [keyHash],
    );
    return rows[0] ? String(rows[0].id) : null;
  };

  const join = limitedByIp(async (request, ip, secret) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    const { nickname: rawNickname, email: rawEmail } = body.value;
    if (typeof rawNickname !== 'string' || typeof rawEmail !== 'string')
      return fail(400, 'invalid_request');
    const nickname = parseNickname(rawNickname);
    if (!nickname) return fail(400, 'invalid_nickname');
    const email = parseEmail(rawEmail);
    if (email === null) return fail(400, 'invalid_email');
    const ipWait = await limit(LIMITS.joinPerIp, ip);
    if (ipWait !== null) return rateLimited(ipWait);
    const emailKey = emailSubject(email, secret);
    const pairWait = await limit(LIMITS.joinPerEmailAndIp, `${emailKey}:${ip}`);
    if (pairWait !== null) return rateLimited(pairWait);
    const emailWait = await limit(LIMITS.joinPerEmail, emailKey);
    if (emailWait !== null) return rateLimited(emailWait);
    // Last, so requests refused by the limits above do not use up the shared cap.
    const globalWait = await limit(LIMITS.joinGlobal);
    if (globalWait !== null) return rateLimited(globalWait);
    return startJoin(joinContext, nickname, email);
  });

  const verify = limitedByIp(async (request, ip) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    const { pending_id: pendingId, code, nickname: rawNickname } = body.value;
    if (typeof pendingId !== 'string' || typeof code !== 'string')
      return fail(400, 'invalid_request');
    // `nickname` is optional: absent or null means "keep the one given at join".
    const nicknameGiven = rawNickname !== undefined && rawNickname !== null;
    if (nicknameGiven && typeof rawNickname !== 'string') return fail(400, 'invalid_request');
    if (!CODE_FORMAT.test(code)) return fail(400, 'invalid_code');
    const nickname = nicknameGiven ? parseNickname(String(rawNickname)) : null;
    if (nicknameGiven && !nickname) return fail(400, 'invalid_nickname');
    const wait = await limit(LIMITS.verifyPerIp, ip);
    if (wait !== null) return rateLimited(wait);
    return verifyJoin(joinContext, pendingId, code, nickname, requestCountry(request));
  });

  const score = limitedByIp(async (request) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    const leverage = parseLeverageTenths(body.value.leverage_tenths);
    if (leverage === undefined) return fail(400, 'invalid_leverage');
    const keyHash = bearerKeyHash(request);
    const id = keyHash && (await memberId(keyHash));
    if (!id) return fail(401, 'unknown_key');
    const wait = await limit(LIMITS.scorePerMember, id);
    if (wait !== null) return rateLimited(wait);
    // An unknown country keeps the stored one.
    const rows = await query(
      `update members set leverage_tenths = $1, score_updated_at = $2::timestamptz,
         country = coalesce($4, country)
       where id = $3::bigint returning id`,
      [leverage, clock().toISOString(), id, requestCountry(request)],
    );
    return rows.length === 0 ? fail(401, 'unknown_key') : noContent();
  });

  const getMe = limitedByIp(async (request) => {
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    const standing = await memberStanding(query, clock(), keyHash);
    return standing ? json(200, standing) : fail(401, 'unknown_key');
  });

  const renameMe = limitedByIp(async (request) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    if (typeof body.value.nickname !== 'string') return fail(400, 'invalid_request');
    const nickname = parseNickname(body.value.nickname);
    if (!nickname) return fail(400, 'invalid_nickname');
    const keyHash = bearerKeyHash(request);
    const id = keyHash && (await memberId(keyHash));
    if (!id) return fail(401, 'unknown_key');
    const wait = await limit(LIMITS.renamePerMember, id);
    if (wait !== null) return rateLimited(wait);
    let rows;
    try {
      // Renaming to your own nickname in another case keeps the same
      // nickname_key on the same row, so it never conflicts.
      rows = await query(
        `update members set nickname = $1, nickname_key = $2
         where id = $3::bigint returning nickname`,
        [nickname.nickname, nickname.nicknameKey, id],
      );
    } catch (error) {
      if (isUniqueViolation(error)) return fail(409, 'nickname_taken');
      throw error;
    }
    return rows.length === 0
      ? fail(401, 'unknown_key')
      : json(200, { nickname: nickname.nickname });
  });

  const deleteMe = limitedByIp(async (request) => {
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    const rows = await query(
      `delete from members where key_hash = decode($1, 'hex') returning id`,
      [keyHash],
    );
    return rows.length === 0 ? fail(401, 'unknown_key') : noContent();
  });

  // Cached like the page, which shows the same rows.
  const leaderboard: Handler = async () => {
    const response = json(200, { rows: await listedMembers(query, clock()) });
    response.headers.set('cache-control', PAGE_CACHE_CONTROL);
    return response;
  };

  const page: Handler = async () => pageResponse(await listedMembers(query, clock()));

  // Same status and headers as GET, without the body (for uptime checks).
  const pageHead: Handler = async (request) => {
    const response = await guarded(page, log)(request);
    return new Response(null, { status: response.status, headers: response.headers });
  };

  return {
    join: { POST: guarded(join, log) },
    verify: { POST: guarded(verify, log) },
    score: { PUT: guarded(score, log) },
    me: { GET: guarded(getMe, log), PATCH: guarded(renameMe, log), DELETE: guarded(deleteMe, log) },
    leaderboard: { GET: guarded(leaderboard, log) },
    page: { GET: guarded(page, log), HEAD: pageHead },
  };
}
