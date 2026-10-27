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
  that neither starts nor ends the domain; no spaces. Stored lowercased. Not unique. Never returned by any endpoint or shown.

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
Integer 0…100000 inclusive, else 400 `invalid_leverage`. Sets `score_updated_at = now()`.

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
  leverage_tenths integer check (leverage_tenths between 0 and 100000),
  score_updated_at timestamptz,
  created_at timestamptz not null default now()
);
```

## Details the contract above leaves open (as built)

- A body over 4 KB means more than 4096 bytes. The 413 body is `{"error": "body_too_large"}`.
- The body is checked before the key. A bad body gets its 400 or 413 even when the key is unknown.
- `PUT /api/score`: a body that is not a JSON object gets 400 `invalid_request`. A JSON object whose
  `leverage_tenths` is missing, not an integer or out of range gets 400 `invalid_leverage`.
- "Within the last 48 hours" includes exactly 48 hours: a score written 48 h ago is still listed; 48 h and
  1 ms ago is not.
- An unexpected server or database failure gives 500 `{"error": "server_error"}`. Logs never contain
  request bodies, emails or keys.
- `/api/*` responses send `Cache-Control: no-store`.
- `HEAD /` returns the same status and headers as `GET /`, with no body.
- The page sends `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'`.
- The local development server answers an unknown path with 404 `not_found` and an unsupported method
  with 405 `method_not_allowed`. On Vercel those two answers come from Vercel itself.
