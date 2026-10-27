-- Applied by `pnpm db:migrate`. Every statement must be safe to run again.
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
