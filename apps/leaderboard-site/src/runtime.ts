// The app instance the api/ files use. On Vercel it talks to Neon through
// DATABASE_URL; the local server installs a PGlite-backed app instead.
import { neon } from '@neondatabase/serverless';
import { createApp, type App } from './app.js';
import type { Query, Row } from './db.js';
import { resendMailer, unavailableMailer } from './mail.js';

class ConfigError extends Error {
  override name = 'ConfigError';
}

export function neonQuery(databaseUrl: string | undefined): Query {
  if (!databaseUrl)
    return async () => {
      throw new ConfigError('DATABASE_URL is not set');
    };
  const sql = neon(databaseUrl);
  return async (text, params = []) => (await sql.query(text, params)) as Row[];
}

let current: App | undefined;

/** Replaces the app used by api/ (local server only). */
export function installApp(app: App): void {
  current = app;
}

/** Reads DATABASE_URL, RESEND_API_KEY, LEADERBOARD_FROM_EMAIL and IP_HASH_SECRET. */
export type Environment = Record<string, string | undefined>;

/** The production app for these environment variables. `query` is replaced only in tests. */
export function appFromEnvironment(env: Environment, query = neonQuery(env.DATABASE_URL)): App {
  return createApp({
    query,
    mailer: env.RESEND_API_KEY
      ? resendMailer({ apiKey: env.RESEND_API_KEY, from: env.LEADERBOARD_FROM_EMAIL })
      : unavailableMailer,
    ipHashSecret: env.IP_HASH_SECRET,
  });
}

export function app(): App {
  current ??= appFromEnvironment(process.env);
  return current;
}
