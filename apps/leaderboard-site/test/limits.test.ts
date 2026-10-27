import assert from 'node:assert/strict';
import { after, before, beforeEach, describe, test } from 'node:test';
import { createHash } from 'node:crypto';
import { clientIp, ipLimitKey } from '../src/limits.js';
import {
  HOUR,
  harness,
  join,
  MINUTE,
  readJson,
  request,
  score,
  startJoin,
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

/** 429 with the same whole seconds in the body and in Retry-After. */
async function expectLimited(response: Response, seconds: number) {
  assert.equal(response.status, 429);
  assert.equal(response.headers.get('retry-after'), String(seconds));
  assert.deepEqual(await readJson(response), {
    error: 'rate_limited',
    retry_after_seconds: seconds,
  });
}

const postJoin = (nickname: string, email: string, ip: string) =>
  h.app.join.POST(request('POST', '/api/join', { body: { nickname, email }, ip }));

describe('POST /api/join', () => {
  test('5 per hour per IP; the 6th waits for the next hour', async () => {
    for (let i = 1; i <= 5; i += 1)
      assert.equal((await postJoin(`ip${i}`, `ip${i}@example.com`, '192.0.2.9')).status, 202);
    h.advance(10 * MINUTE);
    await expectLimited(await postJoin('ip6', 'ip6@example.com', '192.0.2.9'), 50 * 60);
    // Another IP is not affected.
    assert.equal((await postJoin('ip7', 'ip7@example.com', '192.0.2.10')).status, 202);
    // The window resets on the hour (START is 12:00:00).
    h.advance(50 * MINUTE - 1);
    await expectLimited(await postJoin('ip8', 'ip8@example.com', '192.0.2.9'), 1);
    h.advance(1);
    assert.equal((await postJoin('ip9', 'ip9@example.com', '192.0.2.9')).status, 202);
  });

  test('3 per hour per email from one IP; other IPs can still join with that email', async () => {
    for (let i = 1; i <= 3; i += 1)
      assert.equal((await postJoin('mailer', 'Same@example.com', '192.0.2.1')).status, 202);
    await expectLimited(await postJoin('mailer', 'same@EXAMPLE.com', '192.0.2.1'), 3600);
    assert.equal((await postJoin('mailer', 'same@example.com', '192.0.2.2')).status, 202);
    assert.equal((await postJoin('mailer', 'other@example.com', '192.0.2.1')).status, 202);
    h.advance(HOUR);
    assert.equal((await postJoin('mailer', 'same@example.com', '192.0.2.1')).status, 202);
  });

  test('10 per day per email, from any IPs (UTC days)', async () => {
    for (let i = 1; i <= 10; i += 1)
      assert.equal((await postJoin('daily', 'daily@example.com', `192.0.2.${i}`)).status, 202);
    // START is 12:00 UTC, so the day window ends in 12 hours.
    await expectLimited(await postJoin('daily', 'daily@example.com', '192.0.2.11'), 12 * 3600);
    assert.equal(h.mailer.sent.filter((item) => item.to === 'daily@example.com').length, 10);
    h.advance(12 * HOUR);
    assert.equal((await postJoin('daily', 'daily@example.com', '192.0.2.12')).status, 202);
  });

  test("an attacker cannot 429 someone else's email before the daily cap", async () => {
    // One attacker IP gets 3 per hour for this email, then 429.
    for (let i = 1; i <= 3; i += 1)
      assert.equal((await postJoin('attacker', 'victim@example.com', '198.51.100.66')).status, 202);
    await expectLimited(await postJoin('attacker', 'victim@example.com', '198.51.100.66'), 3600);
    // Even spread over several IPs, the victim (on their own IP) still gets
    // through until the email's daily total of 10 is reached.
    for (const ip of ['198.51.100.67', '198.51.100.68'])
      for (let i = 1; i <= 3; i += 1)
        assert.equal((await postJoin('attacker', 'victim@example.com', ip)).status, 202);
    assert.equal((await postJoin('Victim', 'victim@example.com', '192.0.2.200')).status, 202);
    await expectLimited(await postJoin('Victim', 'victim@example.com', '192.0.2.200'), 12 * 3600);
  });

  test('invalid bodies do not use up the join limit', async () => {
    for (let i = 0; i < 10; i += 1)
      assert.equal((await postJoin('x', 'bad', '192.0.2.20')).status, 400);
    assert.equal((await postJoin('valid', 'valid@example.com', '192.0.2.20')).status, 202);
  });
});

describe('POST /api/join/verify', () => {
  test('20 per hour per IP', async () => {
    const attempt = (ip: string) =>
      h.app.verify.POST(
        request('POST', '/api/join/verify', {
          body: { pending_id: 'AAAAAAAAAAAAAAAAAAAAAA', code: '123456' },
          ip,
        }),
      );
    for (let i = 1; i <= 20; i += 1) assert.equal((await attempt('192.0.2.30')).status, 410);
    await expectLimited(await attempt('192.0.2.30'), 3600);
    assert.equal((await attempt('192.0.2.31')).status, 410);
  });
});

describe('PUT /api/score', () => {
  test('12 per hour per member', async () => {
    const key = await join(h, 'scorer');
    const other = await join(h, 'other');
    const put = (k: string) =>
      h.app.score.PUT(request('PUT', '/api/score', { key: k, body: { leverage_tenths: 5 } }));
    for (let i = 1; i <= 12; i += 1) assert.equal((await put(key)).status, 204);
    await expectLimited(await put(key), 3600);
    assert.equal((await put(other)).status, 204);
    h.advance(HOUR);
    assert.equal((await put(key)).status, 204);
  });
});

describe('PATCH /api/me', () => {
  test('10 per day per member (UTC days)', async () => {
    const key = await join(h, 'renamer');
    const patch = (nickname: string) =>
      h.app.me.PATCH(request('PATCH', '/api/me', { key, body: { nickname } }));
    for (let i = 1; i <= 10; i += 1) assert.equal((await patch(`name${i}`)).status, 200);
    // START is 12:00 UTC, so the day window ends in 12 hours.
    await expectLimited(await patch('name11'), 12 * 3600);
    h.advance(12 * HOUR);
    assert.equal((await patch('name12')).status, 200);
  });
});

describe('every /api/* request except GET /api/leaderboard', () => {
  test('600 per hour per IP', async () => {
    const get = (ip: string) => h.app.me.GET(request('GET', '/api/me', { ip }));
    for (let i = 1; i <= 600; i += 1) assert.equal((await get('192.0.2.40')).status, 401);
    await expectLimited(await get('192.0.2.40'), 3600);
    // Every limited endpoint shares the same per-IP count.
    await expectLimited(
      await h.app.join.POST(
        request('POST', '/api/join', { body: { nickname: 'n', email: 'e' }, ip: '192.0.2.40' }),
      ),
      3600,
    );
    assert.equal((await get('192.0.2.41')).status, 401);
    // The leaderboard and the page are never limited.
    for (let i = 0; i < 5; i += 1) {
      const ip = { ip: '192.0.2.40' };
      assert.equal(
        (await h.app.leaderboard.GET(request('GET', '/api/leaderboard', ip))).status,
        200,
      );
      assert.equal((await h.app.page.GET(request('GET', '/', ip))).status, 200);
    }
  });
});

describe('client IP', () => {
  test('x-real-ip, else the first x-forwarded-for entry', () => {
    const at = (headers: Record<string, string>) =>
      clientIp(new Request('https://leaderboard.test/', { headers }));
    assert.equal(at({ 'x-real-ip': '192.0.2.1', 'x-forwarded-for': '192.0.2.2' }), '192.0.2.1');
    assert.equal(at({ 'x-forwarded-for': ' 192.0.2.2 , 10.0.0.1' }), '192.0.2.2');
  });

  test('IPv6 addresses are limited by their /64; IPv4-mapped ones as IPv4', () => {
    const cases: [string, string][] = [
      ['192.0.2.1', '192.0.2.1'],
      ['2001:db8:1:2:3:4:5:6', '2001:db8:1:2::/64'],
      ['2001:db8:1:2::9', '2001:db8:1:2::/64'],
      ['2001:DB8:0001:0002:ffff::', '2001:db8:1:2::/64'],
      ['[2001:db8::1]', '2001:db8:0:0::/64'],
      ['fe80::1%eth0', 'fe80:0:0:0::/64'],
      ['::', '0:0:0:0::/64'],
      ['2001:db8:1:2:3:4:192.0.2.1', '2001:db8:1:2::/64'],
      ['::ffff:192.0.2.7', '192.0.2.7'],
      ['::ffff:c000:207', '192.0.2.7'],
      ['unknown', 'unknown'],
    ];
    for (const [ip, key] of cases) assert.equal(ipLimitKey(ip), key, ip);
  });

  test('two addresses in one /64 share the join limit; another /64 does not', async () => {
    for (let i = 1; i <= 5; i += 1)
      assert.equal(
        (await postJoin(`v6_${i}`, `v6_${i}@example.com`, `2001:db8:a:b::${i}`)).status,
        202,
      );
    await expectLimited(
      await postJoin('v6_6', 'v6_6@example.com', '2001:db8:a:b:ffff:1:2:3'),
      3600,
    );
    assert.equal((await postJoin('v6_7', 'v6_7@example.com', '2001:db8:a:c::1')).status, 202);
  });

  test('the IP is stored only as an HMAC, and the email only as a hash', async () => {
    await postJoin('hashed', 'hashed.person@example.com', '203.0.113.77');
    await h.app.me.GET(request('GET', '/api/me', { ip: '203.0.113.77' }));
    const dump = JSON.stringify(
      await h.query(
        `select (select json_agg(r)::text from rate_counters r) as counters,
                (select json_agg(p)::text from pending_joins p) as pending,
                (select json_agg(m)::text from members m) as members`,
      ),
    );
    assert.ok(!dump.includes('203.0.113.77'));
    const buckets = (await h.query('select bucket from rate_counters')).map((row) =>
      String(row.bucket),
    );
    assert.ok(buckets.length >= 3);
    for (const bucket of buckets) {
      assert.match(bucket, /^[a-z]+:[a-z-]+:[0-9a-f]{64}(?::[0-9a-f]{64})?$/);
      assert.ok(!bucket.includes('hashed.person'));
      // Keyed: the plain SHA-256 of the email is not used either.
      assert.ok(
        !bucket.includes(createHash('sha256').update('hashed.person@example.com').digest('hex')),
      );
    }
  });
});

describe('cleanup', () => {
  test('counters and expired pending joins older than a day are removed by later joins', async () => {
    await startJoin(h, 'stale', 'stale@example.com');
    h.advance(24 * HOUR + 16 * MINUTE);
    await startJoin(h, 'fresh', 'fresh@example.com');
    const pending = await h.query('select email from pending_joins');
    assert.deepEqual(pending, [{ email: 'fresh@example.com' }]);
    const old = await h.query(
      `select count(*)::int as n from rate_counters where window_start < $1::timestamptz`,
      [new Date(h.now().getTime() - 24 * HOUR).toISOString()],
    );
    assert.equal(old[0]!.n, 0);
  });

  test("any limited endpoint cleans up on an IP's first request of the hour", async () => {
    await startJoin(h, 'stale2', 'stale2@example.com');
    h.advance(24 * HOUR + 16 * MINUTE);
    await h.app.me.GET(request('GET', '/api/me', { ip: '192.0.2.99' }));
    assert.deepEqual(await h.query('select email from pending_joins'), []);
    const old = await h.query(
      `select count(*)::int as n from rate_counters where window_start < $1::timestamptz`,
      [new Date(h.now().getTime() - 24 * HOUR).toISOString()],
    );
    assert.equal(old[0]!.n, 0);
  });

  test('a recent score still works after cleanup', async () => {
    const key = await join(h, 'kept');
    await score(h.app, key, 3);
    assert.equal((await h.query('select count(*)::int as n from members'))[0]!.n, 1);
  });
});
