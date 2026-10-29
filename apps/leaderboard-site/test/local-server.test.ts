// The local server routes to the real api/ files, the way Vercel does.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import type { Server } from 'node:http';
import { createApp } from '../src/app.js';
import { createHmac } from 'node:crypto';
import { applySchema, type Query } from '../src/db.js';
import {
  LOCAL_IP_HASH_SECRET,
  openLocalDatabase,
  readSchema,
  startLocalServer,
} from '../src/local.js';
import { consoleMailer } from '../src/mail.js';
import { installApp } from '../src/runtime.js';
import { existsSync, readFileSync } from 'node:fs';

let server: Server;
let origin: string;
let close: () => Promise<void>;
let localQuery: Query;
/** What the local server's mailer printed. */
const printed: string[] = [];

before(async () => {
  const { db, query } = await openLocalDatabase();
  localQuery = query;
  installApp(
    createApp({
      query,
      mailer: consoleMailer((line) => printed.push(line)),
      ipHashSecret: LOCAL_IP_HASH_SECRET,
    }),
  );
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
  assert.equal(await applySchema(query, readSchema()), 7);
  const indexes = (await query(`select indexname from pg_indexes where schemaname = 'public'`)).map(
    (row) => String(row.indexname),
  );
  assert.ok(indexes.includes('rate_counters_window_start'));
  assert.ok(indexes.includes('pending_joins_expires_at'));
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
      'country',
    ],
  );
  for (const country of ['us', 'USA', 'U', 'ÉS', '', 'XX'])
    await assert.rejects(
      query(
        `insert into members (nickname, nickname_key, email, key_hash, country) values ('c','c','c',decode('03','hex'),$1)`,
        [country],
      ),
      /check constraint/,
      country,
    );
  await query(
    `insert into members (nickname, nickname_key, email, key_hash, country) values ('d','d','d',decode('04','hex'),'NZ')`,
  );
  // Running it again keeps the column, its check and the stored country.
  assert.equal(await applySchema(query, readSchema()), 7);
  assert.deepEqual(await query(`select country from members where nickname = 'd'`), [
    { country: 'NZ' },
  ]);
  await assert.rejects(
    query(
      `insert into members (nickname, nickname_key, email, key_hash, country) values ('e','e','e',decode('05','hex'),'nz')`,
    ),
    /check constraint/,
  );
  await assert.rejects(
    query(
      `insert into members (nickname, nickname_key, email, key_hash, leverage_tenths) values ('a','a','a',decode('01','hex'),1000000001)`,
    ),
    /check constraint/,
  );
  await query(
    `insert into members (nickname, nickname_key, email, key_hash, leverage_tenths) values ('b','b','b',decode('02','hex'),1000000000)`,
  );
  await db.close();
});

/** The check constraints on members that mention the country. */
const countryChecks = async (query: Query) =>
  (
    await query(
      `select conname, pg_get_constraintdef(oid) as definition from pg_constraint
       where conrelid = 'members'::regclass and contype = 'c'
         and pg_get_constraintdef(oid) like '%country%'`,
    )
  ).map((row) => String(row.conname));

test('rerunning schema.sql on a table with rows keeps them and one country check', async () => {
  const { db, query } = await openLocalDatabase();
  await query(
    `insert into members (nickname, nickname_key, email, key_hash, country, leverage_tenths)
     values ('kept','kept','kept@example.com',decode('10','hex'),'SE', 42),
            ('blank','blank','blank@example.com',decode('11','hex'),null, 7)`,
  );
  for (let run = 0; run < 3; run += 1) await applySchema(query, readSchema());
  assert.deepEqual(await countryChecks(query), ['members_country_check']);
  assert.deepEqual(
    await query('select nickname, country, leverage_tenths from members order by nickname'),
    [
      { nickname: 'blank', country: null, leverage_tenths: 7 },
      { nickname: 'kept', country: 'SE', leverage_tenths: 42 },
    ],
  );
  // XX means no country, so the database refuses it as a stored value.
  await assert.rejects(
    query(`update members set country = 'XX' where nickname = 'kept'`),
    /check constraint/,
  );
  await db.close();
});

test("schema.sql replaces the first version's country check rather than adding a second", async () => {
  const { db, query } = await openLocalDatabase();
  // The table as the first version of this column left it: an inline check, no XX rule.
  await query('alter table members drop column country');
  await query(
    `alter table members add column country text
       check (octet_length(country) = 2 and country ~ '^[A-Z]{2}$')`,
  );
  assert.deepEqual(await countryChecks(query), ['members_country_check']);
  await query(
    `insert into members (nickname, nickname_key, email, key_hash, country)
     values ('old','old','old@example.com',decode('12','hex'),'NL')`,
  );
  await applySchema(query, readSchema());
  assert.deepEqual(await countryChecks(query), ['members_country_check']);
  assert.deepEqual(await query('select country from members'), [{ country: 'NL' }]);
  await assert.rejects(query(`update members set country = 'XX'`), /check constraint/);
  await db.close();
});

test('a country check that cannot be added again leaves the previous one in place', async () => {
  const { db, query } = await openLocalDatabase();
  // The first version's inline check, which allowed XX, and a stored XX row.
  await query('alter table members drop column country');
  await query(
    `alter table members add column country text
       check (octet_length(country) = 2 and country ~ '^[A-Z]{2}$')`,
  );
  await query(
    `insert into members (nickname, nickname_key, email, key_hash, country)
     values ('xx','xx','xx@example.com',decode('13','hex'),'XX')`,
  );
  // The new check refuses the stored XX, so the migration fails...
  await assert.rejects(applySchema(query, readSchema()), /check constraint/);
  // ...and the drop is undone with it: the old check is still there and still refuses.
  assert.deepEqual(await countryChecks(query), ['members_country_check']);
  await assert.rejects(
    query(
      `insert into members (nickname, nickname_key, email, key_hash, country)
       values ('bad','bad','bad@example.com',decode('14','hex'),'zz!')`,
    ),
    /check constraint/,
  );
  assert.deepEqual(await query('select country from members'), [{ country: 'XX' }]);
  await db.close();
});

test('join, verify, score, me, leaderboard and page over HTTP', async () => {
  const joined = await fetch(`${origin}/api/join`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ nickname: 'Http_User', email: 'HTTP@example.com' }),
  });
  assert.equal(joined.status, 202);
  const { pending_id } = (await joined.json()) as { pending_id: string };
  const line = printed.at(-1) ?? '';
  const match = /^code for http@example\.com: ([0-9]{6})$/.exec(line);
  assert.ok(match, 'the local server prints the code');
  // A client cannot claim a country: the local server drops the header, as no
  // proxy here has looked the IP up.
  const claimed = { 'x-vercel-ip-country': 'FR' };
  const verified = await fetch(`${origin}/api/join/verify`, {
    method: 'POST',
    headers: { ...claimed, 'content-type': 'application/json' },
    body: JSON.stringify({ pending_id, code: match[1] }),
  });
  assert.equal(verified.status, 201);
  const { key, rejoined } = (await verified.json()) as { key: string; rejoined: boolean };
  assert.equal(rejoined, false);
  const auth = { authorization: `Bearer ${key}` };

  const scored = await fetch(`${origin}/api/score`, {
    method: 'PUT',
    headers: { ...auth, ...claimed, 'content-type': 'application/json' },
    body: JSON.stringify({ leverage_tenths: 45 }),
  });
  assert.equal(scored.status, 204);

  const mine = (await (await fetch(`${origin}/api/me`, { headers: auth })).json()) as Record<
    string,
    unknown
  >;
  assert.equal(mine.rank, 1);
  assert.equal(mine.country, null);
  assert.deepEqual(await (await fetch(`${origin}/api/leaderboard`)).json(), {
    rows: [{ rank: 1, nickname: 'Http_User', leverage_tenths: 45, country: null }],
  });
  const page = await fetch(`${origin}/`);
  assert.equal(page.headers.get('cache-control'), 'public, s-maxage=60, stale-while-revalidate=60');
  assert.match(
    await page.text(),
    /<span class="nick">Http_User<\/span>[\s\S]*?<td class="lev">4\.5X<\/td>/,
  );

  const big = await fetch(`${origin}/api/score`, {
    method: 'PUT',
    headers: { ...auth, 'content-type': 'application/json' },
    body: JSON.stringify({ leverage_tenths: 1, pad: 'x'.repeat(5000) }),
  });
  assert.equal(big.status, 413);

  // fetch sends a string body as text/plain, as a form on another site could.
  const plain = await fetch(`${origin}/api/join`, {
    method: 'POST',
    body: JSON.stringify({ nickname: 'Plain_User', email: 'plain@example.com' }),
  });
  assert.equal(plain.status, 415);
  assert.ok(!printed.some((item) => item.startsWith('code for plain@')));

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

test('the client IP is the socket address, whatever the client claims', async () => {
  await fetch(`${origin}/api/me`, {
    headers: {
      'x-vercel-forwarded-for': '192.0.2.40',
      'x-real-ip': '192.0.2.50',
      'x-forwarded-for': '192.0.2.60',
    },
  });
  const hmac = (ip: string) => createHmac('sha256', LOCAL_IP_HASH_SECRET).update(ip).digest('hex');
  const buckets = (await localQuery('select bucket from rate_counters')).map((row) =>
    String(row.bucket),
  );
  assert.ok(buckets.includes(`api:ip:${hmac('127.0.0.1')}`));
  for (const spoofed of ['192.0.2.40', '192.0.2.50', '192.0.2.60'])
    assert.ok(!buckets.includes(`api:ip:${hmac(spoofed)}`));
});

test('files in public/ are served like Vercel serves them, and nothing outside it', async () => {
  const page = await (await fetch(`${origin}/`)).text();
  const fontPath = /url\("([^"]*)"\)/.exec(page)?.[1];
  assert.equal(fontPath, '/fonts/PressStart2P-Regular.ttf');
  const font = await fetch(`${origin}${fontPath}`);
  assert.equal(font.status, 200);
  assert.equal(font.headers.get('content-type'), 'font/ttf');
  const onDisk = readFileSync(new URL(`../public${fontPath}`, import.meta.url));
  assert.deepEqual(Buffer.from(await font.arrayBuffer()), onDisk);
  assert.equal(font.headers.get('content-length'), String(onDisk.length));

  const head = await fetch(`${origin}${fontPath}`, { method: 'HEAD' });
  assert.equal(head.status, 200);
  assert.equal(head.headers.get('content-type'), 'font/ttf');
  assert.equal(await head.text(), '');

  const license = await fetch(`${origin}/fonts/PressStart2P-OFL.txt`);
  assert.equal(license.status, 200);
  assert.equal(license.headers.get('content-type'), 'text/plain; charset=utf-8');
  assert.match(await license.text(), /SIL OPEN FONT LICENSE/);
  const robots = await fetch(`${origin}/robots.txt`);
  assert.equal(robots.status, 200);
  assert.equal(robots.headers.get('content-type'), 'text/plain; charset=utf-8');

  // Files of this site outside public/, reached with encoded `..` and slashes. The .txt
  // fixture has a type the server serves, so only the inside-public/ check stops it.
  const outside = '../test/fixtures/outside-public.txt';
  assert.ok(existsSync(new URL(`../public/${outside}`, import.meta.url)));
  for (const path of [
    `/${outside.replaceAll('/', '%2f')}`,
    `/fonts/..%2F${outside.replaceAll('/', '%2F')}`,
    `/${outside.replaceAll('.', '%2e').replaceAll('/', '%2f')}`,
    '/fonts/..%2f..%2ftest%2ffixtures%2foutside-public.txt',
    '/fonts/..%2f..%2fpackage.json',
    '/fonts/..%2F..%2Fsrc%2Fpage.ts',
    '/%2e%2e/package.json',
    '/fonts/%2e%2e%2f%2e%2e%2fschema.sql',
    '/fonts/',
    '/fonts',
    '/fonts/missing.ttf',
    '/fonts/bad%zz.ttf',
  ]) {
    const response = await fetch(`${origin}${path}`);
    assert.equal(response.status, 404, path);
    assert.deepEqual(await response.json(), { error: 'not_found' });
  }
  assert.equal((await fetch(`${origin}/robots.txt`, { method: 'POST', body: '' })).status, 404);
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
