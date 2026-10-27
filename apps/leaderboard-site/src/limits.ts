// The one rate-limit rule: fixed windows counted in Postgres, so every Vercel
// instance shares them. Each limit's numbers live only in LIMITS below.
import { createHmac } from 'node:crypto';
import { isIPv4, isIPv6 } from 'node:net';
import type { Query } from './db.js';

export type Limit = { name: string; max: number; windowSeconds: number };

const HOUR = 3600;
const DAY = 86400;

export const LIMITS = {
  /** Every /api/* request except GET /api/leaderboard (and the public page). */
  api: { name: 'api:ip', max: 600, windowSeconds: HOUR },
  joinPerIp: { name: 'join:ip', max: 5, windowSeconds: HOUR },
  /** One IP cannot use up an email's joins for everyone else. */
  joinPerEmailAndIp: { name: 'join:email-ip', max: 3, windowSeconds: HOUR },
  /** Protects the inbox: at most this many code emails per address per day. */
  joinPerEmail: { name: 'join:email', max: 10, windowSeconds: DAY },
  verifyPerIp: { name: 'verify:ip', max: 20, windowSeconds: HOUR },
  scorePerMember: { name: 'score:member', max: 12, windowSeconds: HOUR },
  renamePerMember: { name: 'rename:member', max: 10, windowSeconds: DAY },
} satisfies Record<string, Limit>;

/** Counter rows (and expired pending joins) older than this are removed. */
export const CLEANUP_AFTER_SECONDS = DAY;

export type Hit = { count: number; retryAfterSeconds: number | null };

/**
 * Counts one request against `limit` for `subject`. `retryAfterSeconds` is null
 * when it is allowed, or the whole seconds until the window ends when it is not.
 */
export async function hitLimit(
  query: Query,
  now: Date,
  limit: Limit,
  subject: string,
): Promise<Hit> {
  const windowMs = limit.windowSeconds * 1000;
  const start = Math.floor(now.getTime() / windowMs) * windowMs;
  const rows = await query(
    `insert into rate_counters (bucket, window_start, count) values ($1, $2::timestamptz, 1)
     on conflict (bucket, window_start) do update set count = rate_counters.count + 1
     returning count`,
    [`${limit.name}:${subject}`, new Date(start).toISOString()],
  );
  const count = Number(rows[0]?.count);
  if (count <= limit.max) return { count, retryAfterSeconds: null };
  return {
    count,
    retryAfterSeconds: Math.max(1, Math.ceil((start + windowMs - now.getTime()) / 1000)),
  };
}

/** Removes counter rows and expired pending joins older than a day. */
export async function cleanUp(query: Query, now: Date): Promise<void> {
  const cutoff = new Date(now.getTime() - CLEANUP_AFTER_SECONDS * 1000).toISOString();
  await query('delete from rate_counters where window_start < $1::timestamptz', [cutoff]);
  await query('delete from pending_joins where expires_at < $1::timestamptz', [cutoff]);
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

/** Expands an IPv6 address to its 8 groups, or null when it is not one. */
function ipv6Groups(address: string): number[] | null {
  if (!isIPv6(address)) return null;
  let text = address.toLowerCase();
  // A trailing dotted IPv4 part becomes two groups.
  const dotted = /^(.*:)(\d+\.\d+\.\d+\.\d+)$/.exec(text);
  if (dotted) {
    const [a, b, c, d] = dotted[2]!.split('.').map(Number) as [number, number, number, number];
    text = `${dotted[1]}${((a << 8) | b).toString(16)}:${((c << 8) | d).toString(16)}`;
  }
  const [head = '', tail] = text.split('::');
  const left = head ? head.split(':') : [];
  const right = tail ? tail.split(':') : [];
  const missing = tail === undefined ? 0 : 8 - left.length - right.length;
  const groups = [...left, ...Array<string>(missing).fill('0'), ...right].map((group) =>
    parseInt(group, 16),
  );
  return groups.length === 8 && groups.every((group) => group >= 0 && group <= 0xffff)
    ? groups
    : null;
}

/**
 * What one client is limited as: an IPv4 address as itself, an IPv6 address by
 * its /64 prefix (one home or server usually owns a whole /64). IPv4-mapped IPv6
 * addresses count as their IPv4 address.
 */
export function ipLimitKey(ip: string): string {
  const address = ip.replace(/^\[|\]$/g, '').replace(/%.*$/, '');
  if (isIPv4(address)) return address;
  const groups = ipv6Groups(address);
  if (!groups) return address;
  if (groups.slice(0, 5).every((group) => group === 0) && groups[5] === 0xffff)
    return [groups[6]! >> 8, groups[6]! & 255, groups[7]! >> 8, groups[7]! & 255].join('.');
  return `${groups
    .slice(0, 4)
    .map((group) => group.toString(16))
    .join(':')}::/64`;
}

/** The IP is only ever stored as this keyed hash of its limit key. */
export function ipSubject(request: Request, secret: string): string {
  return createHmac('sha256', secret)
    .update(ipLimitKey(clientIp(request)), 'utf8')
    .digest('hex');
}

/** Email buckets use a keyed hash of the lowercased email, never the email. */
export function emailSubject(email: string, secret: string): string {
  return createHmac('sha256', secret).update(email.toLowerCase(), 'utf8').digest('hex');
}
