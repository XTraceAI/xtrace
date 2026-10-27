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
