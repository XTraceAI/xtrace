-- Applied by `pnpm db:migrate`. Every statement must be safe to run again.
create table if not exists members (
  id bigserial primary key,
  nickname text not null,
  nickname_key text not null unique,      -- lower(nickname)
  email text not null unique,             -- lowercased; proven by an emailed code
  key_hash bytea not null unique,         -- sha256(key)
  leverage_tenths integer check (leverage_tenths between 0 and 1000000000),
  score_updated_at timestamptz,
  created_at timestamptz not null default now()
);

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
  bucket text not null,          -- e.g. 'join:ip:<hmac hex>' / 'join:email:<sha256 hex>' / 'score:member:<id>'
  window_start timestamptz not null,
  count integer not null,
  primary key (bucket, window_start)
);

-- The hourly cleanup deletes by these columns.
create index if not exists rate_counters_window_start on rate_counters (window_start);
create index if not exists pending_joins_expires_at on pending_joins (expires_at);

-- Added 2026-10-29: the country Vercel reports for the member's IP address (never the IP itself).
-- Null when it was never known. Kept when a later request's country is unknown.
alter table members add column if not exists country text;
-- The same rule as isCountryCode in src/country.ts: two uppercase ASCII letters, never XX.
-- Dropped and added again on every run, so each run leaves exactly this one check (this
-- name is also the one Postgres gave the first version's inline check, which it replaces).
-- One statement, so it is all or nothing: if a stored row breaks the new check, the add
-- fails and the old check stays in place.
alter table members
  drop constraint if exists members_country_check,
  add constraint members_country_check
    check (octet_length(country) = 2 and country ~ '^[A-Z]{2}$' and country <> 'XX');
