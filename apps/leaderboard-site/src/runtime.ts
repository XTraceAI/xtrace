// The app instance the api/ files use. On Vercel it talks to Neon through
// DATABASE_URL; the local server installs a PGlite-backed app instead.
import { neon } from '@neondatabase/serverless';
import { createApp, type App } from './app.js';
import type { Query, Row } from './db.js';

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

export function app(): App {
  current ??= createApp({ query: neonQuery(process.env.DATABASE_URL) });
  return current;
}
