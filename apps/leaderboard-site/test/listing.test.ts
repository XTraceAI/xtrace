import assert from 'node:assert/strict';
import { after, before, beforeEach, describe, test } from 'node:test';
import { escapeHtml, formatLeverage } from '../src/page.js';
import {
  HOUR,
  harness,
  join,
  me,
  readJson,
  request,
  score,
  START,
  type Harness,
} from './support.js';

let h: Harness;
before(async () => {
  h = await harness();
});
beforeEach(async () => {
  await h.reset();
});
after(async () => {
  await h.db.close();
});

const board = async () =>
  (
    (await readJson(await h.app.leaderboard.GET(request('GET', '/api/leaderboard')))) as {
      rows: { rank: number; nickname: string; leverage_tenths: number }[];
    }
  ).rows;
const page = async () => {
  const response = await h.app.page.GET(request('GET', '/'));
  assert.equal(response.status, 200);
  return { response, html: await response.text() };
};
const standing = async (key: string) =>
  (await readJson(await me(h.app, key))) as { listed: boolean; rank: number | null };
const pageNames = (html: string) =>
  [...html.matchAll(/<tr><td>([^<]*)<\/td><td>([^<]*)<\/td><\/tr>/g)].map((m) => [m[1], m[2]]);

describe('the 48-hour rule', () => {
  test('listed through exactly 48 hours, hidden after; page, list and me agree', async () => {
    const key = await join(h.app, 'clocked');
    await score(h.app, key, 20);

    h.advance(48 * HOUR);
    assert.deepEqual(await board(), [{ rank: 1, nickname: 'clocked', leverage_tenths: 20 }]);
    assert.deepEqual(await standing(key), { ...(await standing(key)), listed: true, rank: 1 });
    assert.deepEqual(pageNames((await page()).html), [['clocked', '2.0×']]);

    h.advance(1);
    assert.deepEqual(await board(), []);
    const after = (await readJson(await me(h.app, key))) as Record<string, unknown>;
    assert.equal(after.listed, false);
    assert.equal(after.rank, null);
    assert.equal(after.leverage_tenths, 20);
    assert.equal(after.score_updated_at, START.toISOString());
    assert.match((await page()).html, /No one is listed yet\./);

    // A new score brings them back.
    await score(h.app, key, 21);
    assert.deepEqual(await board(), [{ rank: 1, nickname: 'clocked', leverage_tenths: 21 }]);
  });

  test('members with no number are never listed, however fresh', async () => {
    const none = await join(h.app, 'nothing');
    await score(h.app, none, null);
    await join(h.app, 'never');
    assert.deepEqual(await board(), []);
    assert.equal((await standing(none)).listed, false);
  });

  test('a stale member does not take up a rank', async () => {
    const old = await join(h.app, 'old');
    await score(h.app, old, 999);
    h.advance(49 * HOUR);
    const fresh = await join(h.app, 'fresh');
    await score(h.app, fresh, 5);
    assert.deepEqual(await board(), [{ rank: 1, nickname: 'fresh', leverage_tenths: 5 }]);
    assert.equal((await standing(fresh)).rank, 1);
  });
});

describe('ranking', () => {
  test('competition ranking with ties, ties ordered by nickname ignoring case', async () => {
    const entries: [string, number][] = [
      ['Zed', 50],
      ['Bob', 120],
      ['alice', 120],
      ['carl', 120],
      ['dave', 300],
      ['_under', 50],
      ['Eve', 0],
    ];
    const keys = new Map<string, string>();
    for (const [nickname, tenths] of entries) {
      const key = await join(h.app, nickname);
      keys.set(nickname, key);
      await score(h.app, key, tenths);
    }
    const expected = [
      { rank: 1, nickname: 'dave', leverage_tenths: 300 },
      { rank: 2, nickname: 'alice', leverage_tenths: 120 },
      { rank: 2, nickname: 'Bob', leverage_tenths: 120 },
      { rank: 2, nickname: 'carl', leverage_tenths: 120 },
      { rank: 5, nickname: '_under', leverage_tenths: 50 },
      { rank: 5, nickname: 'Zed', leverage_tenths: 50 },
      { rank: 7, nickname: 'Eve', leverage_tenths: 0 },
    ];
    assert.deepEqual(await board(), expected);
    for (const row of expected)
      assert.equal((await standing(keys.get(row.nickname)!)).rank, row.rank);
    assert.deepEqual(
      pageNames((await page()).html),
      expected.map((row) => [row.nickname, formatLeverage(row.leverage_tenths)]),
    );
  });

  test('1, 2, 2, 4', async () => {
    for (const [nickname, tenths] of [
      ['aaa', 40],
      ['bbb', 30],
      ['ccc', 30],
      ['ddd', 10],
    ] as const)
      await score(h.app, await join(h.app, nickname), tenths);
    assert.deepEqual(
      (await board()).map((row) => row.rank),
      [1, 2, 2, 4],
    );
  });
});

describe('GET /api/leaderboard', () => {
  test('empty list', async () => {
    const response = await h.app.leaderboard.GET(request('GET', '/api/leaderboard'));
    assert.equal(response.status, 200);
    assert.match(response.headers.get('content-type') ?? '', /^application\/json/);
    assert.deepEqual(await readJson(response), { rows: [] });
  });
});

describe('GET /', () => {
  test('title, line, two columns, cache header', async () => {
    await score(h.app, await join(h.app, 'Ada'), 123);
    const { response, html } = await page();
    assert.equal(response.headers.get('content-type'), 'text/html; charset=utf-8');
    assert.equal(
      response.headers.get('cache-control'),
      'public, s-maxage=60, stale-while-revalidate=60',
    );
    assert.match(html, /<title>XTrace Leaderboard<\/title>/);
    assert.match(html, /<h1>XTrace Leaderboard<\/h1>/);
    assert.ok(
      html.includes(
        'Leverage over the last 7 days: agent hours divided by your own hours. Shared by people who opted in from the XTrace app.',
      ),
    );
    const headers = [...html.matchAll(/<th[^>]*>([^<]*)<\/th>/g)].map((m) => m[1]);
    assert.deepEqual(headers, ['Nickname', 'Leverage (7 days)']);
    assert.deepEqual(pageNames(html), [['Ada', '12.3×']]);
    assert.match(html, /name="viewport" content="width=device-width, initial-scale=1"/);
    assert.match(html, /prefers-color-scheme: dark/);
    assert.ok(!html.includes('No one is listed yet.'));
  });

  test('strict content security policy', async () => {
    const { response } = await page();
    assert.equal(
      response.headers.get('content-security-policy'),
      "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'",
    );
  });

  test('HEAD gives the same status and headers as GET, with no body', async () => {
    await score(h.app, await join(h.app, 'Header'), 10);
    const { response: get } = await page();
    const head = await h.app.page.HEAD(request('HEAD', '/'));
    assert.equal(head.status, 200);
    assert.deepEqual([...head.headers], [...get.headers]);
    assert.equal(await head.text(), '');
  });

  test('empty state', async () => {
    const { html } = await page();
    assert.match(html, /No one is listed yet\./);
    assert.ok(!html.includes('<table>'));
  });

  test('nicknames are HTML-escaped', async () => {
    // Validation never allows these characters, so write the row directly.
    await h.query(
      `insert into members (nickname, nickname_key, email, key_hash, leverage_tenths, score_updated_at)
       values ($1, $2, 'x@y.z', decode('00', 'hex'), 10, $3::timestamptz)`,
      [`<script>alert("x")</script>&'`, 'evil', START.toISOString()],
    );
    const { html } = await page();
    assert.ok(!html.includes('<script>'));
    assert.ok(
      html.includes('&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;&#39;'),
      'escaped nickname present',
    );
  });

  test('leverage is always written with one decimal', () => {
    assert.equal(formatLeverage(0), '0.0×');
    assert.equal(formatLeverage(7), '0.7×');
    assert.equal(formatLeverage(10), '1.0×');
    assert.equal(formatLeverage(123), '12.3×');
    assert.equal(formatLeverage(100000), '10000.0×');
    assert.equal(escapeHtml(`a&b<c>d"e'f`), 'a&amp;b&lt;c&gt;d&quot;e&#39;f');
  });
});
