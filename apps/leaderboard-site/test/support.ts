import type { PGlite } from '@electric-sql/pglite';
import { createApp, type App } from '../src/app.js';
import type { Query } from '../src/db.js';
import { openLocalDatabase } from '../src/local.js';
import type { Mailer } from '../src/mail.js';

export const ORIGIN = 'https://leaderboard.test';
export const START = new Date('2026-10-27T12:00:00.000Z');
export const HOUR = 60 * 60 * 1000;
export const MINUTE = 60 * 1000;
export const TEST_IP_SECRET = 'test-ip-hash-secret';
/** The IP `request()` sends unless a test names another. */
export const DEFAULT_IP = '198.51.100.1';

export type SentCode = { to: string; code: string };

/** Records every code instead of sending it. `failNext` makes the next send throw. */
export class FakeMailer implements Mailer {
  sent: SentCode[] = [];
  failNext = false;
  async sendCode(to: string, code: string): Promise<void> {
    if (this.failNext) {
      this.failNext = false;
      throw new Error('send failed');
    }
    this.sent.push({ to, code });
  }
  lastCodeFor(email: string): string {
    const found = this.sent.filter((item) => item.to === email).at(-1);
    if (!found) throw new Error('no code was sent');
    return found.code;
  }
}

export type Harness = {
  app: App;
  db: PGlite;
  query: Query;
  logs: string[];
  mailer: FakeMailer;
  now: () => Date;
  setNow: (date: Date) => void;
  advance: (ms: number) => void;
  reset: () => Promise<void>;
};

/** One PGlite database per test file; `reset` empties it and restores the clock. */
export async function harness(): Promise<Harness> {
  const { db, query } = await openLocalDatabase();
  let now = START;
  const logs: string[] = [];
  const mailer = new FakeMailer();
  const app = createApp({
    query,
    clock: () => now,
    log: (message) => logs.push(message),
    mailer,
    ipHashSecret: TEST_IP_SECRET,
  });
  return {
    app,
    db,
    query,
    logs,
    mailer,
    now: () => now,
    setNow: (date) => {
      now = date;
    },
    advance: (ms) => {
      now = new Date(now.getTime() + ms);
    },
    reset: async () => {
      await query('truncate members, pending_joins, rate_counters restart identity');
      now = START;
      logs.length = 0;
      mailer.sent = [];
      mailer.failNext = false;
    },
  };
}

export function request(
  method: string,
  path: string,
  options: {
    body?: unknown;
    raw?: string | Uint8Array;
    key?: string;
    ip?: string;
    headers?: Record<string, string>;
  } = {},
): Request {
  const headers = new Headers(options.headers);
  if (!['x-vercel-forwarded-for', 'x-real-ip', 'x-forwarded-for'].some((name) => headers.has(name)))
    headers.set('x-vercel-forwarded-for', options.ip ?? DEFAULT_IP);
  if (options.key !== undefined) headers.set('authorization', `Bearer ${options.key}`);
  let body: string | Uint8Array | undefined = options.raw;
  if (options.body !== undefined) {
    body = JSON.stringify(options.body);
    headers.set('content-type', 'application/json');
  }
  return new Request(`${ORIGIN}${path}`, {
    method,
    headers,
    ...(body === undefined ? {} : { body }),
  });
}

export async function readJson(response: Response): Promise<unknown> {
  return JSON.parse(await response.text());
}

let joinIp = 0;
/** A different IP for each helper join, so the join limits do not interfere. */
export function nextIp(): string {
  joinIp += 1;
  return `203.0.${Math.floor(joinIp / 250) % 250}.${(joinIp % 250) + 1}`;
}

export async function startJoin(h: Harness, nickname: string, email: string, ip = nextIp()) {
  const response = await h.app.join.POST(
    request('POST', '/api/join', { body: { nickname, email }, ip }),
  );
  if (response.status !== 202) throw new Error(`join failed: ${response.status}`);
  return ((await readJson(response)) as { pending_id: string }).pending_id;
}

export async function verify(h: Harness, pendingId: string, code: string, ip = nextIp()) {
  return h.app.verify.POST(
    request('POST', '/api/join/verify', { body: { pending_id: pendingId, code }, ip }),
  );
}

/** Joins in both steps and returns the new key. */
export async function join(h: Harness, nickname: string, email = `${nickname}@example.com`) {
  const pendingId = await startJoin(h, nickname, email);
  const response = await verify(h, pendingId, h.mailer.lastCodeFor(email.trim().toLowerCase()));
  if (response.status !== 201) throw new Error(`verify failed: ${response.status}`);
  return ((await readJson(response)) as { key: string }).key;
}

export async function score(app: App, key: string, leverage_tenths: number | null) {
  const response = await app.score.PUT(
    request('PUT', '/api/score', { key, body: { leverage_tenths } }),
  );
  if (response.status !== 204) throw new Error(`score failed: ${response.status}`);
}

export async function me(app: App, key: string) {
  return app.me.GET(request('GET', '/api/me', { key }));
}
