// The one visibility rule. The public page, GET /api/leaderboard and the
// `listed`/`rank` fields of GET /api/me all read it from here.
import type { Query } from './db.js';

/** A member is listed when they have a number updated within this many hours. */
export const LISTED_WITHIN_HOURS = 48;

// `$1` is the current time. Rank is standard competition ranking (1, 2, 2, 4).
const LISTED = `listed as (
  select id, nickname, leverage_tenths, country,
         (rank() over (order by leverage_tenths desc))::int as rank
  from members
  where leverage_tenths is not null
    and score_updated_at >= $1::timestamptz - interval '${LISTED_WITHIN_HOURS} hours'
)`;

// Nicknames are ASCII, so the "C" collation gives the same case-insensitive
// order on every Postgres, whatever the database's default collation is.
const ORDER = `order by leverage_tenths desc, lower(nickname) collate "C" asc`;

/** `country` is two uppercase letters, or null when it is not known (see country.ts). */
export type ListedRow = {
  rank: number;
  nickname: string;
  leverage_tenths: number;
  country: string | null;
};

function country(value: unknown): string | null {
  return value === null || value === undefined ? null : String(value);
}

export async function listedMembers(query: Query, now: Date): Promise<ListedRow[]> {
  const rows = await query(
    `with ${LISTED} select rank, nickname, leverage_tenths, country from listed ${ORDER}`,
    [now.toISOString()],
  );
  return rows.map((row) => ({
    rank: Number(row.rank),
    nickname: String(row.nickname),
    leverage_tenths: Number(row.leverage_tenths),
    country: country(row.country),
  }));
}

export type Standing = {
  nickname: string;
  leverage_tenths: number | null;
  listed: boolean;
  rank: number | null;
  score_updated_at: string | null;
  country: string | null;
};

/** The member with this key hash and where they stand, or null for an unknown key. */
export async function memberStanding(
  query: Query,
  now: Date,
  keyHashHex: string,
): Promise<Standing | null> {
  const rows = await query(
    `with ${LISTED}
     select m.nickname, m.leverage_tenths, l.rank, m.country,
            to_char(m.score_updated_at at time zone 'UTC',
                    'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') as score_updated_at
     from members m left join listed l on l.id = m.id
     where m.key_hash = decode($2, 'hex')`,
    [now.toISOString(), keyHashHex],
  );
  const row = rows[0];
  if (!row) return null;
  const rank = row.rank === null || row.rank === undefined ? null : Number(row.rank);
  return {
    nickname: String(row.nickname),
    leverage_tenths: row.leverage_tenths === null ? null : Number(row.leverage_tenths),
    listed: rank !== null,
    rank,
    score_updated_at: row.score_updated_at === null ? null : String(row.score_updated_at),
    country: country(row.country),
  };
}
