// Applies schema.sql to the database in DATABASE_URL. Safe to run more than once.
// Usage: DATABASE_URL=... pnpm db:migrate
import { readFileSync } from 'node:fs';
import { applySchema } from '../src/db.js';
import { neonQuery } from '../src/runtime.js';

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  console.error('Set DATABASE_URL to the Neon connection string first.');
  process.exit(2);
}
try {
  const schema = readFileSync(new URL('../schema.sql', import.meta.url), 'utf8');
  const count = await applySchema(neonQuery(databaseUrl), schema);
  console.log(`Schema applied: ${count} statement(s).`);
} catch (error) {
  // Error details can include the connection string; print only the kind.
  const code =
    typeof error === 'object' && error !== null && 'code' in error
      ? ` (code ${String(error.code)})`
      : '';
  console.error(
    `Migration failed: ${error instanceof Error ? error.name : 'unknown error'}${code}.`,
  );
  process.exitCode = 1;
}
