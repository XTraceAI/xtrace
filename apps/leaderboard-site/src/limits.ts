// The one rate-limit rule: fixed windows counted in Postgres, so every Vercel
// instance shares them. Each limit's numbers live only in LIMITS below.
import { createHash, createHmac } from 'node:crypto';
import type { Query } from './db.js';

export type Limit = { name: string; max: number; windowSeconds: number };

const HOUR = 3600;
const DAY = 86400;

export const LIMITS = {
  /** Every /api/* request except GET /api/leaderboard (and the public page). */
  api: { name: 'api:ip', max: 600, windowSeconds: HOUR },
  joinPerIp: { name: 'join:ip', max: 5, windowSeconds: HOUR },
  joinPerEmail: { name: 'join:email', max: 3, windowSeconds: HOUR },
  verifyPerIp: { name: 'verify:ip', max: 20, windowSeconds: HOUR },
  scorePerMember: { name: 'score:member', max: 12, windowSeconds: HOUR },
  renamePerMember: { name: 'rename:member', max: 10, windowSeconds: DAY },
} satisfies Record<string, Limit>;

/** Counter rows (and expired pending joins) older than this are removed. */
export const CLEANUP_AFTER_SECONDS = DAY;

/**
 * Counts one request against `limit` for `subject`. Returns null when it is
 * allowed, or the whole seconds until the window ends when it is over the limit.
 */
export async function hitLimit(
  query: Query,
  now: Date,
  limit: Limit,
  subject: string,
): Promise<number | null> {
  const windowMs = limit.windowSeconds * 1000;
  const start = Math.floor(now.getTime() / windowMs) * windowMs;
  const rows = await query(
    `insert into rate_counters (bucket, window_start, count) values ($1, $2::timestamptz, 1)
     on conflict (bucket, window_start) do update set count = rate_counters.count + 1
     returning count`,
    [`${limit.name}:${subject}`, new Date(start).toISOString()],
  );
  if (Number(rows[0]?.count) <= limit.max) return null;
  return Math.max(1, Math.ceil((start + windowMs - now.getTime()) / 1000));
}

/**
 * The client's IP: `x-real-ip` (set by Vercel; the local server sets it from the
 * socket), else the first `x-forwarded-for` entry.
 */
export function clientIp(request: Request): string {
  const real = request.headers.get('x-real-ip')?.trim();
  if (real) return real;
  const forwarded = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim();
  return forwarded || 'unknown';
}

/** The IP is only ever stored as this keyed hash. */
export function ipSubject(request: Request, secret: string): string {
  return createHmac('sha256', secret).update(clientIp(request), 'utf8').digest('hex');
}

/** Email buckets use the SHA-256 of the lowercased email, never the email. */
export function emailSubject(email: string): string {
  return createHash('sha256').update(email.toLowerCase(), 'utf8').digest('hex');
}
