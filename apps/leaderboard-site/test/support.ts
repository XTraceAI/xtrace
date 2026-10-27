import type { PGlite } from '@electric-sql/pglite';
import { createApp, type App } from '../src/app.js';
import type { Query } from '../src/db.js';
import { openLocalDatabase } from '../src/local.js';

export const ORIGIN = 'https://leaderboard.test';
export const START = new Date('2026-10-27T12:00:00.000Z');
export const HOUR = 60 * 60 * 1000;

export type Harness = {
  app: App;
  db: PGlite;
  query: Query;
  logs: string[];
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
  const app = createApp({ query, clock: () => now, log: (message) => logs.push(message) });
  return {
    app,
    db,
    query,
    logs,
    now: () => now,
    setNow: (date) => {
      now = date;
    },
    advance: (ms) => {
      now = new Date(now.getTime() + ms);
    },
    reset: async () => {
      await query('truncate members restart identity');
      now = START;
      logs.length = 0;
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
    headers?: Record<string, string>;
  } = {},
): Request {
  const headers = new Headers(options.headers);
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

/** Joins and returns the new key. */
export async function join(app: App, nickname: string, email = `${nickname}@example.com`) {
  const response = await app.join.POST(request('POST', '/api/join', { body: { nickname, email } }));
  if (response.status !== 201) throw new Error(`join failed: ${response.status}`);
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
