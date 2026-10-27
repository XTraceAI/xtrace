// Every endpoint's behaviour. The files in api/ only forward to these handlers.
// The request body is always checked before the key.
import { isUniqueViolation, type Clock, type Query } from './db.js';
import { fail, json, noContent, readJsonObject } from './http.js';
import { bearerKeyHash, newKey } from './keys.js';
import { listedMembers, memberStanding } from './listing.js';
import { pageResponse } from './page.js';
import { parseEmail, parseLeverageTenths, parseNickname } from './validate.js';

export type Handler = (request: Request) => Promise<Response>;

export type App = {
  join: { POST: Handler };
  score: { PUT: Handler };
  me: { GET: Handler; PATCH: Handler; DELETE: Handler };
  leaderboard: { GET: Handler };
  page: { GET: Handler; HEAD: Handler };
};

export type AppOptions = { query: Query; clock?: Clock; log?: (message: string) => void };

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
}: AppOptions): App {
  const join: Handler = async (request) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    const { nickname: rawNickname, email: rawEmail } = body.value;
    if (typeof rawNickname !== 'string' || typeof rawEmail !== 'string')
      return fail(400, 'invalid_request');
    const nickname = parseNickname(rawNickname);
    if (!nickname) return fail(400, 'invalid_nickname');
    const email = parseEmail(rawEmail);
    if (email === null) return fail(400, 'invalid_email');
    const { key, keyHashHex } = newKey();
    const rows = await query(
      `insert into members (nickname, nickname_key, email, key_hash)
       values ($1, $2, $3, decode($4, 'hex'))
       on conflict (nickname_key) do nothing
       returning nickname`,
      [nickname.nickname, nickname.nicknameKey, email, keyHashHex],
    );
    if (rows.length === 0) return fail(409, 'nickname_taken');
    return json(201, { key, nickname: nickname.nickname });
  };

  const score: Handler = async (request) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    const leverage = parseLeverageTenths(body.value.leverage_tenths);
    if (leverage === undefined) return fail(400, 'invalid_leverage');
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    const rows = await query(
      `update members set leverage_tenths = $1, score_updated_at = $2::timestamptz
       where key_hash = decode($3, 'hex') returning id`,
      [leverage, clock().toISOString(), keyHash],
    );
    return rows.length === 0 ? fail(401, 'unknown_key') : noContent();
  };

  const getMe: Handler = async (request) => {
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    const standing = await memberStanding(query, clock(), keyHash);
    return standing ? json(200, standing) : fail(401, 'unknown_key');
  };

  const renameMe: Handler = async (request) => {
    const body = await readJsonObject(request);
    if (!body.ok) return body.response;
    if (typeof body.value.nickname !== 'string') return fail(400, 'invalid_request');
    const nickname = parseNickname(body.value.nickname);
    if (!nickname) return fail(400, 'invalid_nickname');
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    let rows;
    try {
      // Renaming to your own nickname in another case keeps the same
      // nickname_key on the same row, so it never conflicts.
      rows = await query(
        `update members set nickname = $1, nickname_key = $2
         where key_hash = decode($3, 'hex') returning nickname`,
        [nickname.nickname, nickname.nicknameKey, keyHash],
      );
    } catch (error) {
      if (isUniqueViolation(error)) return fail(409, 'nickname_taken');
      throw error;
    }
    return rows.length === 0
      ? fail(401, 'unknown_key')
      : json(200, { nickname: nickname.nickname });
  };

  const deleteMe: Handler = async (request) => {
    const keyHash = bearerKeyHash(request);
    if (!keyHash) return fail(401, 'unknown_key');
    const rows = await query(
      `delete from members where key_hash = decode($1, 'hex') returning id`,
      [keyHash],
    );
    return rows.length === 0 ? fail(401, 'unknown_key') : noContent();
  };

  const leaderboard: Handler = async () => json(200, { rows: await listedMembers(query, clock()) });

  const page: Handler = async () => pageResponse(await listedMembers(query, clock()));

  // Same status and headers as GET, without the body (for uptime checks).
  const pageHead: Handler = async (request) => {
    const response = await guarded(page, log)(request);
    return new Response(null, { status: response.status, headers: response.headers });
  };

  return {
    join: { POST: guarded(join, log) },
    score: { PUT: guarded(score, log) },
    me: { GET: guarded(getMe, log), PATCH: guarded(renameMe, log), DELETE: guarded(deleteMe, log) },
    leaderboard: { GET: guarded(leaderboard, log) },
    page: { GET: guarded(page, log), HEAD: pageHead },
  };
}
