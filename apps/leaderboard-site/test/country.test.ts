// The member's country: read only from Vercel's x-vercel-ip-country header,
// saved at verify and on every score, kept when unknown, shown as a flag.
import assert from 'node:assert/strict';
import { after, before, beforeEach, describe, test } from 'node:test';
import { COUNTRY_HEADER, countryFlag, requestCountry } from '../src/country.js';
import { flagHtml, renderPage } from '../src/page.js';
import { harness, me, nextIp, readJson, request, startJoin, type Harness } from './support.js';

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

const withCountry = (country: string | undefined): Record<string, string> =>
  country === undefined ? {} : { [COUNTRY_HEADER]: country };

/** Joins in both steps, sending `country` (if any) on the verify request. */
async function joinFrom(nickname: string, country?: string, email = `${nickname}@example.com`) {
  const pendingId = await startJoin(h, nickname, email);
  const response = await h.app.verify.POST(
    request('POST', '/api/join/verify', {
      body: { pending_id: pendingId, code: h.mailer.lastCodeFor(email.toLowerCase()) },
      ip: nextIp(),
      headers: withCountry(country),
    }),
  );
  assert.equal(response.status, 201);
  return ((await readJson(response)) as { key: string }).key;
}

async function scoreFrom(key: string, leverage_tenths: number, country?: string) {
  const response = await h.app.score.PUT(
    request('PUT', '/api/score', { key, body: { leverage_tenths }, headers: withCountry(country) }),
  );
  assert.equal(response.status, 204);
}

const storedCountry = async (nickname: string) =>
  (await h.query('select country from members where nickname = $1', [nickname]))[0]?.country;

const board = async () =>
  (
    (await readJson(await h.app.leaderboard.GET(request('GET', '/api/leaderboard')))) as {
      rows: { nickname: string; country: string | null }[];
    }
  ).rows;

describe('the country header rule', () => {
  const countryOf = (value?: string) =>
    requestCountry(new Request('https://leaderboard.test/', { headers: withCountry(value) }));

  test('two uppercase ASCII letters are the country', () => {
    assert.equal(countryOf('US'), 'US');
    assert.equal(countryOf('DE'), 'DE');
    assert.equal(countryOf('NZ'), 'NZ');
  });

  test('a missing header is unknown', () => {
    assert.equal(countryOf(undefined), null);
  });

  test('anything else is unknown', () => {
    for (const value of ['', 'XX', 'us', 'Us', 'USA', 'U', 'U S', 'U1', 'ÜS', 'US,DE'])
      assert.equal(countryOf(value), null, JSON.stringify(value));
  });

  test('surrounding spaces are removed by the Headers API before the rule sees the value', () => {
    assert.equal(countryOf(' US '), 'US');
  });

  test('no other header gives a country', () => {
    const request = new Request('https://leaderboard.test/', {
      headers: { 'cf-ipcountry': 'US', 'x-country': 'US', 'x-vercel-ip-country-region': 'CA' },
    });
    assert.equal(requestCountry(request), null);
  });
});

describe('saving the country', () => {
  test('a new member gets the country of the verify request', async () => {
    await joinFrom('Newbie', 'SE');
    assert.equal(await storedCountry('Newbie'), 'SE');
  });

  test('a new member with no known country has none', async () => {
    await joinFrom('Nowhere');
    await joinFrom('Unknown', 'XX');
    assert.equal(await storedCountry('Nowhere'), null);
    assert.equal(await storedCountry('Unknown'), null);
  });

  test('signing back in updates the country, or keeps it when unknown', async () => {
    await joinFrom('Mover', 'FR', 'mover@example.com');
    await joinFrom('ignored', undefined, 'mover@example.com');
    assert.equal(await storedCountry('Mover'), 'FR');
    await joinFrom('ignored', 'it', 'mover@example.com');
    assert.equal(await storedCountry('Mover'), 'FR');
    await joinFrom('ignored', 'IT', 'mover@example.com');
    assert.equal(await storedCountry('Mover'), 'IT');
  });

  test('every score updates the country, and an unknown one keeps it', async () => {
    const key = await joinFrom('Scorer');
    assert.equal(await storedCountry('Scorer'), null);
    await scoreFrom(key, 10, 'JP');
    assert.equal(await storedCountry('Scorer'), 'JP');
    for (const unknown of [undefined, 'XX', 'jp', 'JPN', '']) await scoreFrom(key, 11, unknown);
    assert.equal(await storedCountry('Scorer'), 'JP');
    await scoreFrom(key, 12, 'KR');
    assert.equal(await storedCountry('Scorer'), 'KR');
  });

  test('a refused score does not change the country', async () => {
    const key = await joinFrom('Refused', 'CA');
    const bad = await h.app.score.PUT(
      request('PUT', '/api/score', {
        key,
        body: { leverage_tenths: -1 },
        headers: withCountry('MX'),
      }),
    );
    assert.equal(bad.status, 400);
    assert.equal(await storedCountry('Refused'), 'CA');
  });

  test('the IP is not stored with the country', async () => {
    const key = await joinFrom('Private', 'BR');
    await scoreFrom(key, 5, 'BR');
    const row = (await h.query('select * from members'))[0] ?? {};
    const text = JSON.stringify(row);
    assert.ok(!/\d+\.\d+\.\d+\.\d+/.test(text), 'no IPv4 address');
    assert.ok(!text.includes('198.51.100.1'));
  });
});

describe('showing the country', () => {
  test('GET /api/leaderboard and GET /api/me carry the country or null', async () => {
    const known = await joinFrom('Known', 'AU');
    const blank = await joinFrom('Blank');
    await scoreFrom(known, 30);
    await scoreFrom(blank, 20);
    assert.deepEqual(await board(), [
      { rank: 1, nickname: 'Known', leverage_tenths: 30, country: 'AU' },
      { rank: 2, nickname: 'Blank', leverage_tenths: 20, country: null },
    ]);
    const mine = (await readJson(await me(h.app, known))) as Record<string, unknown>;
    assert.equal(mine.country, 'AU');
    const theirs = (await readJson(await me(h.app, blank))) as Record<string, unknown>;
    assert.equal(theirs.country, null);
  });

  test('a member who is not listed still sees their country in GET /api/me', async () => {
    const key = await joinFrom('Unlisted', 'NO');
    const mine = (await readJson(await me(h.app, key))) as Record<string, unknown>;
    assert.equal(mine.listed, false);
    assert.equal(mine.country, 'NO');
  });

  test('the page shows the flag before the nickname, on the board and in 1UP', async () => {
    const leader = await joinFrom('Leader', 'US');
    const plain = await joinFrom('Plain');
    await scoreFrom(leader, 50);
    await scoreFrom(plain, 40);
    const html = await (await h.app.page.GET(request('GET', '/'))).text();
    const flag = '<span class="flag" role="img" aria-label="US" title="US">🇺🇸</span>';
    assert.ok(
      html.includes(`<div class="line">${flag}<span class="nick">Leader</span>`),
      'flag right before the nickname on the board',
    );
    assert.ok(html.includes(`<dt>1UP</dt><dd>${flag}Leader</dd>`), 'flag before the 1UP name');
    // No flag, and nothing standing in for one, for a member with no country.
    assert.ok(html.includes('<div class="line"><span class="nick">Plain</span>'));
    assert.equal(html.split('class="flag"').length - 1, 2);
    assert.ok(!/<script/i.test(html));
  });

  test('a leader with no country has no flag in 1UP', async () => {
    const key = await joinFrom('Solo');
    await scoreFrom(key, 15);
    const html = await (await h.app.page.GET(request('GET', '/'))).text();
    assert.ok(html.includes('<dt>1UP</dt><dd>Solo</dd>'));
    assert.ok(!html.includes('class="flag"'));
  });

  test('the flag never wraps on its own and the nickname still wraps', async () => {
    const html = renderPage([]);
    assert.match(html, /\.flag \{[^}]*flex: none;/);
    assert.match(html, /\.nick \{[^}]*overflow-wrap: anywhere;/);
  });
});

describe('the flag helpers', () => {
  test('a code becomes its two regional-indicator letters', () => {
    assert.equal(countryFlag('US'), '\u{1F1FA}\u{1F1F8}');
    assert.equal(countryFlag('DE'), '\u{1F1E9}\u{1F1EA}');
    assert.equal(countryFlag('AZ'), '\u{1F1E6}\u{1F1FF}');
    assert.equal([...countryFlag('GB')].length, 2);
  });

  test('anything else gives no flag', () => {
    for (const value of ['', 'us', 'USA', 'U', '<b', '"X', 'U&'])
      assert.equal(countryFlag(value), '', JSON.stringify(value));
  });

  test('flagHtml is empty for null or a bad code, so nothing is injected', () => {
    assert.equal(flagHtml(null), '');
    assert.equal(flagHtml('"><script>'), '');
    assert.equal(flagHtml('a"'), '');
    assert.equal(
      flagHtml('FR'),
      '<span class="flag" role="img" aria-label="FR" title="FR">\u{1F1EB}\u{1F1F7}</span>',
    );
  });

  test('a stored country that bypassed the rule is never shown', () => {
    // The column's check refuses it, so the page is given the row directly.
    const html = renderPage([{ rank: 1, nickname: 'Odd', leverage_tenths: 10, country: '<i>' }]);
    assert.ok(!html.includes('<i>'));
    assert.ok(!html.includes('class="flag"'));
    assert.ok(html.includes('<dd>Odd</dd>'));
  });
});
