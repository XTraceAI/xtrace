// The local server routes to the real api/ files, the way Vercel does.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import type { Server } from 'node:http';
import { createApp } from '../src/app.js';
import { applySchema } from '../src/db.js';
import { openLocalDatabase, readSchema, startLocalServer } from '../src/local.js';
import { installApp } from '../src/runtime.js';
import { readFileSync } from 'node:fs';

let server: Server;
let origin: string;
let close: () => Promise<void>;

before(async () => {
  const { db, query } = await openLocalDatabase();
  installApp(createApp({ query }));
  ({ server, origin } = await startLocalServer(0));
  close = async () => {
    await new Promise((resolve) => server.close(resolve));
    await db.close();
  };
});
after(async () => {
  await close();
});

test('schema.sql can be applied again without changes', async () => {
  const { db, query } = await openLocalDatabase();
  assert.equal(await applySchema(query, readSchema()), 1);
  const columns = await query(
    `select column_name from information_schema.columns where table_name = 'members' order by ordinal_position`,
  );
  assert.deepEqual(
    columns.map((row) => row.column_name),
    [
      'id',
      'nickname',
      'nickname_key',
      'email',
      'key_hash',
      'leverage_tenths',
      'score_updated_at',
      'created_at',
    ],
  );
  await assert.rejects(
    query(
      `insert into members (nickname, nickname_key, email, key_hash, leverage_tenths) values ('a','a','a',decode('01','hex'),100001)`,
    ),
  );
  await db.close();
});

test('join, score, me, leaderboard and page over HTTP', async () => {
  const joined = await fetch(`${origin}/api/join`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ nickname: 'Http_User', email: 'http@example.com' }),
  });
  assert.equal(joined.status, 201);
  const { key } = (await joined.json()) as { key: string };
  const auth = { authorization: `Bearer ${key}` };

  const scored = await fetch(`${origin}/api/score`, {
    method: 'PUT',
    headers: { ...auth, 'content-type': 'application/json' },
    body: JSON.stringify({ leverage_tenths: 45 }),
  });
  assert.equal(scored.status, 204);

  const mine = (await (await fetch(`${origin}/api/me`, { headers: auth })).json()) as Record<
    string,
    unknown
  >;
  assert.equal(mine.rank, 1);
  assert.deepEqual(await (await fetch(`${origin}/api/leaderboard`)).json(), {
    rows: [{ rank: 1, nickname: 'Http_User', leverage_tenths: 45 }],
  });
  const page = await fetch(`${origin}/`);
  assert.equal(page.headers.get('cache-control'), 'public, s-maxage=60, stale-while-revalidate=60');
  assert.match(await page.text(), /<td>Http_User<\/td><td>4\.5×<\/td>/);

  const big = await fetch(`${origin}/api/score`, {
    method: 'PUT',
    headers: auth,
    body: JSON.stringify({ leverage_tenths: 1, pad: 'x'.repeat(5000) }),
  });
  assert.equal(big.status, 413);

  assert.equal((await fetch(`${origin}/api/me`, { method: 'DELETE', headers: auth })).status, 204);
  assert.equal((await fetch(`${origin}/api/me`, { headers: auth })).status, 401);
});

test('HEAD / answers like GET without a body', async () => {
  const head = await fetch(`${origin}/`, { method: 'HEAD' });
  assert.equal(head.status, 200);
  assert.equal(head.headers.get('content-type'), 'text/html; charset=utf-8');
  assert.equal(head.headers.get('cache-control'), 'public, s-maxage=60, stale-while-revalidate=60');
  assert.equal(await head.text(), '');
});

test('Vercel installs this package with the pinned pnpm and no strict Node check', () => {
  const vercel = JSON.parse(readFileSync(new URL('../vercel.json', import.meta.url), 'utf8')) as {
    installCommand: string;
  };
  const site = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as {
    name: string;
  };
  const root = JSON.parse(
    readFileSync(new URL('../../../package.json', import.meta.url), 'utf8'),
  ) as {
    packageManager: string;
  };
  const [, version] = root.packageManager.split('@');
  assert.equal(
    vercel.installCommand,
    `npx --yes pnpm@${version} install --frozen-lockfile --config.engine-strict=false --filter ${site.name}...`,
  );
});

test('unknown paths are 404 and unsupported methods are 405', async () => {
  const missing = await fetch(`${origin}/api/nothing`);
  assert.equal(missing.status, 404);
  assert.deepEqual(await missing.json(), { error: 'not_found' });
  assert.equal((await fetch(`${origin}/elsewhere`)).status, 404);
  const wrong = await fetch(`${origin}/api/leaderboard`, { method: 'POST', body: '{}' });
  assert.equal(wrong.status, 405);
  assert.equal(wrong.headers.get('allow'), 'GET');
  assert.deepEqual(await wrong.json(), { error: 'method_not_allowed' });
});
