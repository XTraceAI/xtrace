import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { after, before, beforeEach, describe, test } from 'node:test';
import { escapeHtml, formatLeverage, LOGO_MARK, ordinal, PAGE_FONT_PATH } from '../src/page.js';
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
      rows: { rank: number; nickname: string; leverage_tenths: number; country: string | null }[];
    }
  ).rows;
const page = async () => {
  const response = await h.app.page.GET(request('GET', '/'));
  assert.equal(response.status, 200);
  return { response, html: await response.text() };
};
const standing = async (key: string) =>
  (await readJson(await me(h.app, key))) as { listed: boolean; rank: number | null };
/** Each member row on the page: rank label, color, rocket width (or ''), nickname, number. */
const pageRows = (html: string) =>
  [
    ...html.matchAll(
      /<tr data-rank="\d+" class="(\w+)"><td class="rank">([^<]*)<\/td><td class="ship">(?:<svg class="rocket" width="(\d+)")?[\s\S]*?<span class="nick">([^<]*)<\/span>[\s\S]*?<td class="lev">([^<]*)<\/td><\/tr>/g,
    ),
  ].map((m) => ({ tone: m[1], label: m[2], rocket: m[3] ?? '', nick: m[4], lev: m[5] }));
const pageNames = (html: string) => pageRows(html).map((row) => [row.nick, row.lev]);
/** The rank labels of the open places shown after the members. */
const openPlaces = (html: string) =>
  [...html.matchAll(/<tr data-empty aria-hidden="true"><td class="rank">([^<]*)<\/td>/g)].map(
    (m) => m[1],
  );
/** The top line's labels and their text (the 1UP name sits in its own spans). */
const hud = (html: string) =>
  Object.fromEntries(
    [...html.matchAll(/<div><dt>([^<]*)<\/dt><dd>(.*?)<\/dd><\/div>/g)].map((m) => [
      m[1],
      m[2]!.replace(/<[^>]*>/g, ''),
    ]),
  );
const openLine = (html: string) => /<p class="open">([^<]*)<\/p>/.exec(html)?.[1];

describe('the 48-hour rule', () => {
  test('listed through exactly 48 hours, hidden after; page, list and me agree', async () => {
    const key = await join(h, 'clocked');
    await score(h.app, key, 20);

    h.advance(48 * HOUR);
    assert.deepEqual(await board(), [
      { rank: 1, nickname: 'clocked', leverage_tenths: 20, country: null },
    ]);
    assert.deepEqual(await standing(key), { ...(await standing(key)), listed: true, rank: 1 });
    assert.deepEqual(pageNames((await page()).html), [['clocked', '2.0X']]);

    h.advance(1);
    assert.deepEqual(await board(), []);
    const after = (await readJson(await me(h.app, key))) as Record<string, unknown>;
    assert.equal(after.listed, false);
    assert.equal(after.rank, null);
    assert.equal(after.leverage_tenths, 20);
    assert.equal(after.score_updated_at, START.toISOString());
    assert.deepEqual(pageNames((await page()).html), []);
    assert.equal(
      openLine((await page()).html),
      '1ST&nbsp;PLACE IS OPEN&nbsp;· JOIN FROM THE&nbsp;APP',
    );

    // A new score brings them back.
    await score(h.app, key, 21);
    assert.deepEqual(await board(), [
      { rank: 1, nickname: 'clocked', leverage_tenths: 21, country: null },
    ]);
  });

  test('members with no number are never listed, however fresh', async () => {
    const none = await join(h, 'nothing');
    await score(h.app, none, null);
    await join(h, 'never');
    assert.deepEqual(await board(), []);
    assert.equal((await standing(none)).listed, false);
  });

  test('a stale member does not take up a rank', async () => {
    const old = await join(h, 'old');
    await score(h.app, old, 999);
    h.advance(49 * HOUR);
    const fresh = await join(h, 'fresh');
    await score(h.app, fresh, 5);
    assert.deepEqual(await board(), [
      { rank: 1, nickname: 'fresh', leverage_tenths: 5, country: null },
    ]);
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
      const key = await join(h, nickname);
      keys.set(nickname, key);
      await score(h.app, key, tenths);
    }
    const expected = [
      { rank: 1, nickname: 'dave', leverage_tenths: 300, country: null },
      { rank: 2, nickname: 'alice', leverage_tenths: 120, country: null },
      { rank: 2, nickname: 'Bob', leverage_tenths: 120, country: null },
      { rank: 2, nickname: 'carl', leverage_tenths: 120, country: null },
      { rank: 5, nickname: '_under', leverage_tenths: 50, country: null },
      { rank: 5, nickname: 'Zed', leverage_tenths: 50, country: null },
      { rank: 7, nickname: 'Eve', leverage_tenths: 0, country: null },
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
      await score(h.app, await join(h, nickname), tenths);
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

  test('cached like the page, which shows the same rows', async () => {
    const response = await h.app.leaderboard.GET(request('GET', '/api/leaderboard'));
    assert.equal(
      response.headers.get('cache-control'),
      'public, s-maxage=60, stale-while-revalidate=60',
    );
    assert.equal(response.headers.get('x-content-type-options'), 'nosniff');
  });
});

describe('GET /', () => {
  test('title, line, four columns, cache header', async () => {
    await score(h.app, await join(h, 'Ada'), 123);
    const { response, html } = await page();
    assert.equal(response.headers.get('content-type'), 'text/html; charset=utf-8');
    assert.equal(
      response.headers.get('cache-control'),
      'public, s-maxage=60, stale-while-revalidate=60',
    );
    assert.match(html, /<title>XTrace Leaderboard<\/title>/);
    assert.match(html, /<meta name="description" content="[^"]+">/);
    assert.match(html, /<h1>HIGH AGENT LEVERAGE<\/h1>/);
    assert.ok(
      html.includes(
        '<p class="sub">AGENT HOURS&nbsp;÷ YOUR HOURS&nbsp;· LAST&nbsp;7&nbsp;DAYS</p>',
      ),
    );
    const headers = [...html.matchAll(/<th scope="col"[^>]*>([^<]*)<\/th>/g)].map((m) => m[1]);
    assert.deepEqual(headers, ['RANK', '', 'NAME', 'LEVER']);
    assert.deepEqual(pageNames(html), [['Ada', '12.3X']]);
    assert.match(html, /name="viewport" content="width=device-width, initial-scale=1"/);
    assert.match(html, /color-scheme: dark;/);
    assert.ok(!html.includes('prefers-color-scheme'), 'always dark');
    assert.ok(html.includes('href="https://github.com/XTraceAI/xtrace">OPEN SOURCE</a>'));
    assert.ok(html.includes('<span class="free">FREE</span>&nbsp;· <a'));
  });

  test('nicknames are never cut off: they wrap instead', async () => {
    const { html } = await page();
    assert.ok(!html.includes('ellipsis'));
    assert.match(html, /\.nick \{[^}]*overflow-wrap: anywhere;/);
    assert.match(html, /\.hud dd \{[^}]*overflow-wrap: anywhere;/);
    // The top number and the player count never break inside the number.
    assert.match(html, /\.hud > div \+ div dd \{[^}]*white-space: nowrap;/);
    assert.match(html, /\.dots \{[^}]*min-width: 0;/);
  });

  test('no scripts and nothing loaded from other sites', async () => {
    await score(h.app, await join(h, 'Quiet'), 30);
    const { html } = await page();
    assert.ok(!/<script/i.test(html));
    assert.ok(!/\son[a-z]+=/i.test(html), 'no inline event handlers');
    assert.ok(!/\ssrc=/i.test(html), 'no src attributes');
    // The only things the page loads are its own icons and font.
    assert.deepEqual(
      [...html.matchAll(/url\("([^"]*)"\)/g)].map((m) => m[1]),
      [PAGE_FONT_PATH],
    );
    assert.deepEqual(
      [...html.matchAll(/<link [^>]*href="([^"]*)"/g)].map((m) => m[1]),
      ['/favicon.svg', '/favicon-32.png', '/apple-touch-icon.png', PAGE_FONT_PATH],
    );
    // Links people can follow are fine; nothing else points at another site.
    const elsewhere = [...html.matchAll(/https?:\/\/[^"\s)]+/g)].map((m) => m[0]);
    assert.deepEqual([...new Set(elsewhere)].sort(), [
      'https://github.com/XTraceAI/xtrace',
      'https://xtrace.ai',
    ]);
  });

  test('the font is served from public/ with its license beside it', async () => {
    const { html } = await page();
    assert.match(html, /@font-face \{[^}]*font-display: swap;/);
    assert.match(html, /--pixel: "Press Start 2P", [^;]*monospace;/);
    const font = new URL(`../public${PAGE_FONT_PATH}`, import.meta.url);
    assert.ok(existsSync(font));
    const license = readFileSync(
      new URL('../public/fonts/PressStart2P-OFL.txt', import.meta.url),
      'utf8',
    );
    assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/);
    assert.match(license, /Reserved Font Name "Press Start 2P"/);
  });

  test('the blinking line stops when the reader asks for less motion', async () => {
    const { html } = await page();
    assert.match(html, /\.open \{[^}]*animation: blink 1s steps\(1, end\) infinite;/);
    assert.match(
      html,
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.open \{ animation: none; \}/,
    );
  });

  test('strict content security policy', async () => {
    const { response } = await page();
    assert.equal(
      response.headers.get('content-security-policy'),
      "default-src 'none'; style-src 'unsafe-inline'; font-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'",
    );
  });

  test('HEAD gives the same status and headers as GET, with no body', async () => {
    await score(h.app, await join(h, 'Header'), 10);
    const { response: get } = await page();
    const head = await h.app.page.HEAD(request('HEAD', '/'));
    assert.equal(head.status, 200);
    assert.deepEqual([...head.headers], [...get.headers]);
    assert.equal(await head.text(), '');
  });

  test('the largest allowed number fits on the page', async () => {
    await score(h.app, await join(h, 'a_twenty_char_name_1'), 1000000000);
    assert.deepEqual(pageNames((await page()).html), [['a_twenty_char_name_1', '100000000.0X']]);
  });

  test('empty state: eight open places and the first one named', async () => {
    const { html } = await page();
    assert.deepEqual(pageRows(html), []);
    assert.ok(!html.includes('data-rank='));
    assert.deepEqual(openPlaces(html), ['1ST', '2ND', '3RD', '4TH', '5TH', '6TH', '7TH', '8TH']);
    assert.equal((html.match(/<span class="nick">---<\/span>/g) ?? []).length, 8);
    assert.equal((html.match(/<td class="lev">-\.-X<\/td>/g) ?? []).length, 8);
    assert.deepEqual(hud(html), { '1UP': '---', 'HI-LEVER': '-.-X', PLAYERS: '00' });
    assert.equal(openLine(html), '1ST&nbsp;PLACE IS OPEN&nbsp;· JOIN FROM THE&nbsp;APP');
  });

  test('ordinals, colors, rockets for the top three ranks, dimming and counts', async () => {
    await score(h.app, await join(h, 'First'), 400);
    await score(h.app, await join(h, 'Second'), 200);
    await score(h.app, await join(h, 'TiedA'), 100);
    await score(h.app, await join(h, 'TiedB'), 100);
    await score(h.app, await join(h, 'Fifth'), 10);
    await score(h.app, await join(h, 'Sixth'), 9);
    const { html } = await page();
    assert.deepEqual(pageRows(html), [
      { tone: 'gold', label: '1ST', rocket: '21', nick: 'First', lev: '40.0X' },
      { tone: 'orange', label: '2ND', rocket: '14', nick: 'Second', lev: '20.0X' },
      { tone: 'pink', label: '3RD', rocket: '14', nick: 'TiedA', lev: '10.0X' },
      { tone: 'pink', label: '3RD', rocket: '14', nick: 'TiedB', lev: '10.0X' },
      { tone: 'cyan', label: '5TH', rocket: '', nick: 'Fifth', lev: '1.0X' },
      { tone: 'dim', label: '6TH', rocket: '', nick: 'Sixth', lev: '0.9X' },
    ]);
    assert.deepEqual(openPlaces(html), ['7TH', '8TH']);
    assert.deepEqual(hud(html), { '1UP': 'First', 'HI-LEVER': '40.0X', PLAYERS: '06' });
    assert.equal(openLine(html), '7TH&nbsp;PLACE IS OPEN&nbsp;· JOIN FROM THE&nbsp;APP');
  });

  test('every listed member is shown, past eight, with no open places', async () => {
    for (let index = 0; index < 12; index += 1)
      await score(h.app, await join(h, `player_${index}`), 200 - index);
    const { html } = await page();
    const rows = pageRows(html);
    assert.equal(rows.length, 12);
    assert.deepEqual(
      rows.map((row) => row.label),
      ['1ST', '2ND', '3RD', '4TH', '5TH', '6TH', '7TH', '8TH', '9TH', '10TH', '11TH', '12TH'],
    );
    assert.deepEqual(openPlaces(html), []);
    assert.equal(hud(html).PLAYERS, '12');
    assert.equal(openLine(html), 'JOIN FROM THE XTRACE&nbsp;APP');
  });

  test('a full board of exactly eight says how to join', async () => {
    for (let index = 0; index < 8; index += 1)
      await score(h.app, await join(h, `eight_${index}`), 50 + index);
    const { html } = await page();
    assert.equal(pageRows(html).length, 8);
    assert.deepEqual(openPlaces(html), []);
    assert.equal(openLine(html), 'JOIN FROM THE XTRACE&nbsp;APP');
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
    const escaped = '&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;&#39;';
    // Once on the board and once as the leader in the top line.
    assert.equal(html.split(escaped).length - 1, 2, 'escaped nickname present');
  });

  test('ranks are written as ordinals', () => {
    const cases: [number, string][] = [
      [1, '1ST'],
      [2, '2ND'],
      [3, '3RD'],
      [4, '4TH'],
      [10, '10TH'],
      [11, '11TH'],
      [12, '12TH'],
      [13, '13TH'],
      [14, '14TH'],
      [21, '21ST'],
      [22, '22ND'],
      [23, '23RD'],
      [100, '100TH'],
      [101, '101ST'],
      [102, '102ND'],
      [111, '111TH'],
      [112, '112TH'],
      [113, '113TH'],
      [121, '121ST'],
      [1001, '1001ST'],
    ];
    for (const [rank, label] of cases) assert.equal(ordinal(rank), label, `rank ${rank}`);
  });

  test('leverage is always written with one decimal', () => {
    assert.equal(formatLeverage(0), '0.0X');
    assert.equal(formatLeverage(7), '0.7X');
    assert.equal(formatLeverage(10), '1.0X');
    assert.equal(formatLeverage(123), '12.3X');
    assert.equal(formatLeverage(100000), '10000.0X');
    assert.equal(formatLeverage(1234567890), '123456789.0X');
    assert.equal(formatLeverage(1000000000), '100000000.0X');
    assert.equal(escapeHtml(`a&b<c>d"e'f`), 'a&amp;b&lt;c&gt;d&quot;e&#39;f');
  });
});

test('the tab icon draws exactly the header logo', () => {
  const favicon = readFileSync(new URL('../public/favicon.svg', import.meta.url), 'utf8');
  assert.equal(
    favicon,
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 3 3">${LOGO_MARK}</svg>\n`,
  );
});
