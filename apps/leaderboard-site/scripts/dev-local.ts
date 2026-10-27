// Local server: the same handlers as production, on 127.0.0.1, backed by PGlite.
// Usage: pnpm dev:local -- [--port 8787] [--data <folder>]
// Without --data the database lives in memory and is gone when the server stops.
import { createApp } from '../src/app.js';
import { openLocalDatabase, startLocalServer } from '../src/local.js';
import { installApp } from '../src/runtime.js';

function option(name: string): string | undefined {
  const args = process.argv.slice(2).filter((arg) => arg !== '--');
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
}

const port = Number(option('--port') ?? '8787');
if (!Number.isInteger(port) || port < 0 || port > 65535) {
  console.error('--port must be a whole number from 0 to 65535.');
  process.exit(2);
}
const dataDir = option('--data');
const { query } = await openLocalDatabase(dataDir);
installApp(createApp({ query }));
const { origin } = await startLocalServer(port);
console.log(
  `Leaderboard local server: ${origin} (database: ${dataDir ? 'file' : 'in memory'}). Stop with Ctrl-C.`,
);
