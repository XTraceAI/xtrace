// Development and tests only (never imported by api/): a PGlite database and a
// plain HTTP server that routes requests the way Vercel does for this folder.
import { PGlite } from '@electric-sql/pglite';
import { existsSync, readFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { Readable } from 'node:stream';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { applySchema, type Query } from './db.js';
import { fail } from './http.js';

const root = fileURLToPath(new URL('..', import.meta.url));

export function readSchema(): string {
  return readFileSync(new URL('../schema.sql', import.meta.url), 'utf8');
}

/** An in-memory database, or a file database when `dataDir` is given, with the schema applied. */
export async function openLocalDatabase(dataDir?: string): Promise<{ db: PGlite; query: Query }> {
  const db = dataDir ? new PGlite(dataDir) : new PGlite();
  const query: Query = async (text, params = []) =>
    (await db.query<Record<string, unknown>>(text, params)).rows;
  await applySchema(query, readSchema());
  return { db, query };
}

type Module = Record<string, unknown>;
const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];

function rewrites(): Map<string, string> {
  const config = JSON.parse(readFileSync(new URL('../vercel.json', import.meta.url), 'utf8')) as {
    rewrites?: { source: string; destination: string }[];
  };
  return new Map((config.rewrites ?? []).map((rule) => [rule.source, rule.destination]));
}

/** The api/ module for a path: `/api/<name>` maps to `api/<name>.ts`, as on Vercel. */
async function routeModule(pathname: string): Promise<Module | null> {
  const target = rewrites().get(pathname) ?? pathname;
  const match = /^\/api\/([a-z][a-z0-9-]*)$/.exec(target);
  if (!match) return null;
  const file = `${root}api/${match[1]}.ts`;
  if (!existsSync(file)) return null;
  return (await import(pathToFileURL(file).href)) as Module;
}

function toRequest(incoming: IncomingMessage, origin: string): Request {
  const headers = new Headers();
  for (const [name, value] of Object.entries(incoming.headers)) {
    if (Array.isArray(value)) for (const item of value) headers.append(name, item);
    else if (value !== undefined) headers.set(name, value);
  }
  const method = incoming.method ?? 'GET';
  const hasBody = method !== 'GET' && method !== 'HEAD';
  return new Request(new URL(incoming.url ?? '/', origin), {
    method,
    headers,
    ...(hasBody
      ? { body: Readable.toWeb(incoming) as ReadableStream<Uint8Array>, duplex: 'half' }
      : {}),
  } as RequestInit);
}

async function send(response: Response, outgoing: ServerResponse): Promise<void> {
  outgoing.statusCode = response.status;
  response.headers.forEach((value, name) => outgoing.setHeader(name, value));
  outgoing.end(Buffer.from(await response.arrayBuffer()));
}

async function handle(incoming: IncomingMessage, origin: string): Promise<Response> {
  const pathname = new URL(incoming.url ?? '/', origin).pathname;
  const module = await routeModule(pathname);
  if (!module) return fail(404, 'not_found');
  const method = incoming.method ?? 'GET';
  const handler = METHODS.includes(method) ? module[method] : undefined;
  if (typeof handler !== 'function') {
    const response = fail(405, 'method_not_allowed');
    response.headers.set(
      'allow',
      METHODS.filter((name) => typeof module[name] === 'function').join(', '),
    );
    return response;
  }
  return (await handler(toRequest(incoming, origin))) as Response;
}

/** Serves the api/ files on 127.0.0.1. Port 0 picks a free port. */
export async function startLocalServer(port: number): Promise<{ server: Server; origin: string }> {
  const server = createServer((incoming, outgoing) => {
    const origin = `http://${incoming.headers.host ?? '127.0.0.1'}`;
    handle(incoming, origin)
      .then((response) => send(response, outgoing))
      .catch(() => {
        outgoing.statusCode = 500;
        outgoing.end();
      });
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => resolve());
  });
  const address = server.address();
  const actual = typeof address === 'object' && address ? address.port : port;
  return { server, origin: `http://127.0.0.1:${actual}` };
}
