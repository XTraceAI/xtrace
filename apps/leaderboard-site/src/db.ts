// The only database surface the site uses. Production passes Neon; tests and the
// local server pass PGlite. Both run the same SQL text.
export type Row = Record<string, unknown>;
export type Query = (text: string, params?: unknown[]) => Promise<Row[]>;
export type Clock = () => Date;

/** Runs each statement of schema.sql in order. Statements end with `;` at a line end. */
export async function applySchema(query: Query, schemaSql: string): Promise<number> {
  const statements = schemaSql
    .split(/;\s*(?:\n|$)/)
    .map((statement) =>
      statement
        .split('\n')
        .filter((line) => !line.trim().startsWith('--'))
        .join('\n')
        .trim(),
    )
    .filter(Boolean);
  for (const statement of statements) await query(statement);
  return statements.length;
}

/** Postgres error code for a unique-constraint violation, from Neon or PGlite. */
export function isUniqueViolation(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === '23505';
}
