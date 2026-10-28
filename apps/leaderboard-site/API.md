# Leaderboard API contract (v1)

Base URL: `https://leaderboard.xtrace.ai` (the app's default; override with env `XTRACE_LEADERBOARD_URL`).
All request and response bodies are JSON (UTF-8). Request bodies over 4 KB are refused with 413.
Errors are `{"error": "<code>"}` with the status shown.

## Visibility (one rule, on the server)

A member is _listed_ when `leverage_tenths IS NOT NULL` and `score_updated_at` is within the last 48 hours.
The public page, `GET /api/leaderboard` and the `rank`/`listed` fields of `GET /api/me` all use this one rule.
Order: `leverage_tenths` descending, then nickname case-insensitively ascending.
Rank: standard competition ranking over listed members (equal leverage shares a rank: 1, 2, 2, 4).

## Nickname and email (validated by the server only; the app shows the server's answer)

- nickname: trimmed; 3–20 characters, each `A–Z a–z 0–9 _ -`. Unique case-insensitively. Stored as typed (after trim).
- email: trimmed; at most 254 characters; exactly one `@` with something before it, and a domain after it containing a dot
  that neither starts nor ends the domain; no spaces. Stored lowercased. ~~Not unique.~~ Never returned by any endpoint or shown.
  _v2 makes `members.email` unique; v2.2 also requires a plain address (see the end of this document)._

## Key

On join the server makes 32 random bytes, returns them base64url without padding (43 characters) as `key`, and stores
only the SHA-256 of the key. Requests about "me" send `Authorization: Bearer <key>`. Missing/unknown key → 401 `unknown_key`.

## Endpoints

### POST /api/join

Body `{"nickname": string, "email": string}`.

- 201 `{"key": string, "nickname": string}`
- 400 `invalid_request` (not JSON / missing or non-string fields), `invalid_nickname`, `invalid_email`
- 409 `nickname_taken`

### PUT /api/score (bearer)

Body `{"leverage_tenths": integer | null}` — the 7-day leverage times ten, as the app writes it to one decimal
(e.g. 12.3× → 123). `null` means the app has no 7-day number; the member is then not listed.
Integer 0…1000000000 inclusive (changed 2026-10-27 from 100000: a few seconds of your own time against long agent runs can exceed 10,000×), else 400 `invalid_leverage`. Sets `score_updated_at = now()`.

- 204 (no body)
- 401 `unknown_key`

### GET /api/me (bearer)

- 200 `{"nickname": string, "leverage_tenths": integer | null, "listed": boolean, "rank": integer | null, "score_updated_at": string | null}`
  (`rank` is null when not listed; `score_updated_at` is ISO 8601 UTC)
- 401 `unknown_key`

### PATCH /api/me (bearer)

Body `{"nickname": string}`.

- 200 `{"nickname": string}`
- 400 `invalid_request` / `invalid_nickname`, 409 `nickname_taken` (renaming to your own nickname in another case is allowed), 401 `unknown_key`

### DELETE /api/me (bearer)

Deletes the member's row (nickname, email, score) completely.

- 204
- 401 `unknown_key`

### GET /api/leaderboard

- 200 `{"rows": [{"rank": integer, "nickname": string, "leverage_tenths": integer}]}` — listed members only, in order.

### GET /

Public HTML page. Title "XTrace Leaderboard". One line under the title:
"Leverage over the last 7 days: agent hours divided by your own hours. Shared by people who opted in from the XTrace app."
A table with exactly two columns: **Nickname**, **Leverage (7 days)**. Leverage written `12.3×` (tenths ÷ 10, always one
decimal). Rows in the order above. Empty state: "No one is listed yet." Nicknames HTML-escaped. Works at phone width,
light and dark. Cache header `public, s-maxage=60, stale-while-revalidate=60`.

## Database (Postgres; Neon in production, set by Vercel Marketplace as `DATABASE_URL`)

```sql
create table if not exists members (
  id bigserial primary key,
  nickname text not null,
  nickname_key text not null unique,      -- lower(nickname)
  email text not null,
  key_hash bytea not null unique,         -- sha256(key)
  leverage_tenths integer check (leverage_tenths between 0 and 1000000000),
  score_updated_at timestamptz,
  created_at timestamptz not null default now()
);
```

---

# v2 changes (2026-10-27): email code check and rate limits

Tristan asked for both. Everything above still holds unless replaced here.

## Joining now takes two steps

### POST /api/join (replaces the v1 response)

Body `{"nickname": string, "email": string}` (same validation as v1).

- If the email already belongs to a member, the nickname is validated but otherwise ignored (see "Signing back in").
- ~~Otherwise, if the nickname belongs to another member: 409 `nickname_taken` (nicknames are public, so this reveals nothing private).~~
  _Replaced by v2.1: join never checks the nickname._
- Creates a _pending join_: random id (16 bytes, base64url, no padding), nickname, email (lowercased), SHA-256 of a
  6-digit code (000000–999999, from a CSPRNG), `expires_at = now() + 15 minutes`, `attempts = 0`.
  ~~Any earlier pending join for the same email is deleted (only the newest code works).~~
  _Replaced by v2.2: earlier live codes stay valid._
- Emails the code (see "Email"). If sending fails or email is not configured: delete the pending join, 503 `email_unavailable`.
- 202 `{"pending_id": string}`
- The response never says whether the email is already a member.

### POST /api/join/verify

Body `{"pending_id": string, "code": string}` (v2.1 adds an optional `"nickname"`) (code: exactly 6 ASCII digits, else 400 `invalid_code` without counting an attempt).

- Unknown pending id, expired, or already 5 wrong attempts: 410 `code_expired` (and the pending join is deleted).
- Wrong code: attempts += 1; 400 `invalid_code`. The 5th wrong attempt deletes the pending join.
- Right code: delete the pending join (v2.1: put back unchanged on 409), then
  - email already a member → _signing back in_: give that member a new key (old key stops working), keep their nickname
    and score. 201 `{"key", "nickname", "rejoined": true}`.
  - otherwise create the member (as v1 join did). 201 `{"key", "nickname", "rejoined": false}`.
    ~~If the nickname was taken meanwhile: 409 `nickname_taken` (pending join deleted; the person starts again).~~
    _Replaced by v2.1/v2.2: 409 `nickname_taken`, and the pending join is kept with the nickname just tried._
- Codes are compared by hash; only the hash is stored.

### Signing back in

Because the email is now proven, one email = one member: `members.email` is unique. Joining again with the same email
(for example on a new Mac) signs you back in to your existing entry instead of creating a second one.

## Email

Sent with Resend's HTTP API (`POST https://api.resend.com/emails`, `Authorization: Bearer $RESEND_API_KEY`) using plain
`fetch` — no SDK. From `$LEADERBOARD_FROM_EMAIL`, default `XTrace Leaderboard <noreply@emails.xtrace.ai>`
(the team already sends from emails.xtrace.ai through Resend). Subject: `Your XTrace leaderboard code: 123456`.
Plain text body:
"Your code is 123456. It expires in 15 minutes.
Enter it in the XTrace app to join the leaderboard.
If you did not ask to join the XTrace leaderboard, you can ignore this email."
Production with no RESEND_API_KEY → 503 `email_unavailable`. Codes and emails are never logged.
The local dev server never sends real email: it prints `code for <email>: <code>` to its own console.

## Rate limits (server side, stored in Postgres so they work across Vercel instances)

429 `{"error": "rate_limited", "retry_after_seconds": N}` with a `Retry-After: N` header.
Fixed windows; the one rule lives in one module.

- ~~POST /api/join: 5 per hour per IP, and 3 per hour per email.~~
  _Replaced by v2.2: 5 per hour per IP; 3 per hour per (email, IP); 10 per day per email._
- POST /api/join/verify: 20 per hour per IP (on top of the 5-attempts-per-code rule).
- PUT /api/score: 12 per hour per member.
- PATCH /api/me: 10 per day per member.
- Every /api/* request except GET /api/leaderboard: 600 per hour per IP.
  Client IP: ~~on Vercel the `x-real-ip` header (Vercel sets it), else the first `x-forwarded-for` entry, else the socket
  address (local server).~~ _As built (after review): `x-vercel-forwarded-for`, else `x-real-ip`, else the first
  `x-forwarded-for` entry; see the notes at the end._ The IP is never stored in clear: only HMAC-SHA256(IP, `$IP_HASH_SECRET`). Production with no
  IP_HASH_SECRET → 503 `server_misconfigured` on rate-limited endpoints (fail closed). Local server uses a fixed dev secret.
  Counter rows and expired pending joins older than a day are deleted opportunistically.

## Schema additions

```sql
-- members.email becomes unique (nothing is deployed yet, so the table definition is simply changed)
create table if not exists pending_joins (
  id text primary key,
  nickname text not null,
  nickname_key text not null,
  email text not null,
  code_hash bytea not null,
  attempts integer not null default 0,
  expires_at timestamptz not null,
  created_at timestamptz not null default now()
);
create table if not exists rate_counters (
  bucket text not null,          -- e.g. 'join:ip:<hmac hex>' / 'join:email:<hmac hex>' / 'score:member:<id>'
  window_start timestamptz not null,
  count integer not null,
  primary key (bucket, window_start)
);
```

~~(Email buckets use SHA-256 of the lowercased email, not the email itself.)~~
_Replaced by v2.2: email buckets use HMAC(lowercased email, IP_HASH_SECRET)._

## README (deploy) additions

Env vars RESEND_API_KEY, LEADERBOARD_FROM_EMAIL (optional), IP_HASH_SECRET (random 32+ bytes).
Also recommend a Vercel Firewall rate-limit rule on `/api/join` as an outer layer, and a spend limit.

## v2.1 correction (2026-10-27): join must not reveal whether an email is a member

The v2 rule "409 nickname_taken at POST /api/join when the email is not a member" let anyone test whether an email
belongs to a member (send a known-taken nickname with someone's email: 202 = member, 409 = not). Replaced by:

- POST /api/join never checks nickname availability: valid input always gets 202 (rate limits and 503 still apply).
- POST /api/join/verify body becomes `{"pending_id", "code", "nickname"?}`. The optional `nickname` replaces the pending
  join's nickname (validated: 400 invalid_nickname, without counting an attempt).
- Right code, email not a member, nickname taken by another member → 409 `nickname_taken`, and the pending join is KEPT
  (same code, same expiry) so the person can send verify again with the same code and a different `nickname`.
  This attempt does not count toward the 5 wrong attempts (only wrong codes count).
- Right code for a member's email → signs back in as before (nickname ignored).

## v2.2 (2026-10-27, after review): stop one IP locking out or flooding someone else's email

- Email must be a plain address: local part and domain made only of letters, digits and `.!#$%&'*+/=?^_`{|}~-`(local) / letters, digits,`-`and`.` (domain); no display names, quotes, angle brackets, commas, semicolons, spaces
  or control characters. Else 400 invalid_email.
- A new join no longer deletes earlier live codes for the same email: every pending join keeps working until it
  expires or uses its 5 wrong attempts. (Replaces "only the newest code works".)
- Join limits: 5 per hour per IP (unchanged); 3 per hour per (email, IP) pair; 10 per day per email (inbox protection).
- IPv6 addresses are limited by their /64 prefix. Email counter buckets use HMAC(email, IP_HASH_SECRET), not plain SHA-256.
- A nickname given at verify that gets 409 is saved on the kept pending join (so a later verify without `nickname` uses it).

## Details the contract above leaves open (as built)

- A body over 4 KB means more than 4096 bytes. The 413 body is `{"error": "body_too_large"}`.
- Order of checks in a rate-limited endpoint: the per-IP API limit (600 per hour), then the body
  (413 / 400), then the key (401), then the endpoint's own limits, then the work.
  `POST /api/join` checks its limits in this order: per IP, per (email, IP), per email per day. A request
  stopped by one limit is not counted by the later ones.
  - `POST /api/join`: the join limits count only requests whose body passed the checks. Invalid bodies
    use up only the 600-per-hour API limit.
  - `POST /api/join/verify`: the 20-per-hour limit counts only requests whose code is 6 digits and whose
    optional `nickname` is valid.
  - `PUT /api/score` and `PATCH /api/me` count against the member only when the key is known.
- A refused (429) request still counts. The window resets on its fixed boundary: the hour, or the UTC
  day for `PATCH /api/me`. `retry_after_seconds` is the whole seconds until then (at least 1).
- The 600-per-hour API limit covers `POST /api/join`, `POST /api/join/verify`, `PUT /api/score` and
  `GET`/`PATCH`/`DELETE /api/me`. `GET /api/leaderboard` and the public page (`GET`/`HEAD /`, served by
  `/api/page`) are not limited.
- Client IP: the first entry of the first header present among `x-vercel-forwarded-for`, `x-real-ip` and
  `x-forwarded-for`, trimmed. If that value is not a valid IPv4 or IPv6 address, or no header is present, the
  request is counted under the IP `unknown` (a bad header does not fall through to the next one). IPv6 addresses are
  grouped by their first 64 bits (written `a:b:c:d::/64`); IPv4-mapped IPv6 addresses (`::ffff:1.2.3.4`)
  count as the IPv4 address. Bucket names: `<limit>:<hmac>` for IPs and emails, `<limit>:<member id>` for
  members, and `join:email-ip:<email hmac>:<ip hmac>` for the email-and-IP pair. The local server
  sets `x-vercel-forwarded-for` from the socket address and drops any client-sent `x-real-ip` /
  `x-forwarded-for`, as Vercel's proxy does.
- The two email limits (per (email, IP) and per day) count the _inbox_ address, not the exact one: everything
  from the first `+` in the part before `@` is dropped for every domain, and for `gmail.com` and
  `googlemail.com` the dots before `@` are dropped too and both count as `gmail.com`. So
  `victim+1@gmail.com` and `v.ictim@googlemail.com` share `victim@gmail.com`'s limits. The stored email,
  the code email and the member lookup still use the exact (lowercased) address.
- Known limit, accepted: anyone who knows an email can use up its 10 joins for the day (from several IPs at once, or from one IP over about 4 hours),
  so that inbox cannot start a new join until the next UTC midnight. A code that was already sent and is
  still live keeps working, and an existing member's entry is not affected.
- Emails are also refused (400 `invalid_email`) when the part before `@` starts or ends with a dot or has
  two dots in a row, or when the domain has fewer than two labels, an empty label (`..`), or a label that
  starts or ends with `-`. Plus addressing, subdomains and punycode (`xn--…`) domains are accepted.
- `schema.sql` also indexes `rate_counters(window_start)` and `pending_joins(expires_at)` for the cleanup.
- An empty `IP_HASH_SECRET` counts as missing.
- `POST /api/join/verify` with `pending_id` or `code` missing or not a string, or a `nickname` that is not a
  string, gets 400 `invalid_request`. A `nickname` of `null` is the same as leaving it out. A bad code is
  reported (400 `invalid_code`) before a bad nickname (400 `invalid_nickname`).
- A code is expired from `expires_at` on: exactly 15 minutes after the join it no longer works.
- Only wrong codes count as attempts. The check "code is wrong" and the count happen in one database
  statement, so concurrent guesses cannot count past 5 or get a guess through after the 5th wrong one.
- A right code claims the pending join by deleting it, so only one of several identical requests goes on;
  the others get 410 `code_expired`. On 409 `nickname_taken` the pending join is put back unchanged (same
  id, code, attempts and expiry), with the nickname just tried. While it is being put back, a second
  identical request can get 410; that 410 deletes a pending join only if it is expired or used up, so it
  never removes one that was just put back.
- Opportunistic cleanup runs on any rate-limited endpoint when it is an IP's first request of the hour:
  counter rows whose window started more than a day ago, and pending joins that expired more than a day
  ago, are deleted.
- `PUT /api/score`: a body that is not a JSON object gets 400 `invalid_request`. A JSON object whose
  `leverage_tenths` is missing, not an integer or out of range gets 400 `invalid_leverage`.
- "Within the last 48 hours" includes exactly 48 hours: a score written 48 h ago is still listed; 48 h and
  1 ms ago is not.
- The Resend response body is cancelled without being read.
- An unexpected server or database failure gives 500 `{"error": "server_error"}`. Logs never contain
  request bodies, emails, keys or codes. A failed email send logs only `email not sent (<error kind>)`.
- `/api/*` responses send `Cache-Control: no-store`.
- `HEAD /` returns the same status and headers as `GET /`, with no body.
- The page sends `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'`.
- The local development server answers an unknown path with 404 `not_found` and an unsupported method
  with 405 `method_not_allowed`. On Vercel those two answers come from Vercel itself.
