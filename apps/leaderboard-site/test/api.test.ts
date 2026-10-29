import assert from 'node:assert/strict';
import { after, before, beforeEach, describe, test } from 'node:test';
import { createApp } from '../src/app.js';
import { MAX_BODY_BYTES } from '../src/http.js';
import {
  harness,
  join,
  me,
  nextIp,
  readJson,
  request,
  score,
  START,
  TEST_IP_SECRET,
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

const post = (body: unknown) =>
  h.app.join.POST(request('POST', '/api/join', { body, ip: nextIp() }));

async function expectError(response: Response, status: number, code: string) {
  assert.equal(response.status, status);
  assert.match(response.headers.get('content-type') ?? '', /^application\/json/);
  assert.deepEqual(await readJson(response), { error: code });
}

describe('POST /api/join checks the body first', () => {
  test('refuses bodies that are not a JSON object with string fields', async () => {
    const raw = (text: string) => h.app.join.POST(request('POST', '/api/join', { raw: text }));
    await expectError(await raw('not json'), 400, 'invalid_request');
    await expectError(await raw(''), 400, 'invalid_request');
    await expectError(await raw('[]'), 400, 'invalid_request');
    await expectError(await raw('null'), 400, 'invalid_request');
    await expectError(await raw('"text"'), 400, 'invalid_request');
    await expectError(
      await h.app.join.POST(request('POST', '/api/join', { raw: new Uint8Array([0xff, 0xfe]) })),
      400,
      'invalid_request',
    );
    await expectError(await post({ nickname: 'alpha' }), 400, 'invalid_request');
    await expectError(await post({ email: 'a@b.co' }), 400, 'invalid_request');
    await expectError(await post({ nickname: 123, email: 'a@b.co' }), 400, 'invalid_request');
    await expectError(await post({ nickname: 'alpha', email: null }), 400, 'invalid_request');
  });

  test('checks nicknames: 3–20 of A–Z a–z 0–9 _ - after trimming', async () => {
    for (const nickname of [
      'ab',
      'a'.repeat(21),
      'has space',
      'émile',
      'dot.name',
      'a<b',
      '',
      '   ',
    ])
      await expectError(await post({ nickname, email: 'a@b.co' }), 400, 'invalid_nickname');
    for (const [index, nickname] of ['abc', 'a'.repeat(20), 'A-b_9', ' trimmed '].entries())
      assert.equal((await post({ nickname, email: `n${index}@b.co` })).status, 202, nickname);
  });

  test('checks emails', async () => {
    const bad = [
      'plain',
      '@example.com',
      'a@',
      'a@@b.co',
      'a@b@c.co',
      'a@localhost',
      'a@.b.co',
      'a@b.co.',
      'a b@c.co',
      'a@b .co',
      `${'a'.repeat(250)}@b.co`,
      // Only plain addresses: no display names, quotes, brackets, separators or controls.
      'x<attacker@evil.com>',
      'Name <a@b.co>',
      '<a@b.co>',
      '"quoted"@example.com',
      'a(b)@example.com',
      'a,b@example.com',
      'a;b@example.com',
      'a:b@example.com',
      'a[b]@example.com',
      'a\\b@example.com',
      'a\tb@example.com',
      'a\u0000b@example.com',
      'a\u007fb@example.com',
      'a@exa_mple.com',
      'a@exa,mple.com',
      '.a@example.com',
      'a.@example.com',
      'a..b@example.com',
      'a@-example.com',
      'a@example-.com',
      'a@example.com-',
      'a@sub.-example.com',
      'a@example..com',
      'a@com',
      'a@example.com>',
      'ä@example.com',
      'a@exämple.com',
    ];
    for (const [index, email] of bad.entries())
      await expectError(await post({ nickname: `bad${index}`, email }), 400, 'invalid_email');
    assert.deepEqual(h.mailer.sent, []);
    const good = [
      'a@b.co',
      'first.last+tag@sub.example.org',
      `${'a'.repeat(249)}@b.co`,
      "o'brien@example.com",
      "a!#$%&'*+/=?^_`{|}~-z@my-host.example.com",
      'first.m.last+news@mail.sub.example.co.uk',
      'someone@xn--bcher-kva.example',
      'a@b--c.example.com',
    ];
    for (const [index, email] of good.entries())
      assert.equal((await post({ nickname: `good${index}`, email })).status, 202, email);
  });

  test('nickname is checked before email', async () => {
    await expectError(await post({ nickname: 'x', email: 'bad' }), 400, 'invalid_nickname');
  });
});

describe('request body size', () => {
  const padded = (bytes: number) => {
    const base = JSON.stringify({ nickname: 'sizer', email: 'a@b.co', pad: '' });
    return JSON.stringify({
      nickname: 'sizer',
      email: 'a@b.co',
      pad: 'x'.repeat(bytes - base.length),
    });
  };

  test('accepts exactly 4096 bytes and refuses 4097 with 413', async () => {
    assert.equal(Buffer.byteLength(padded(MAX_BODY_BYTES)), 4096);
    assert.equal(
      (await h.app.join.POST(request('POST', '/api/join', { raw: padded(4096) }))).status,
      202,
    );
    await h.reset();
    await expectError(
      await h.app.join.POST(request('POST', '/api/join', { raw: padded(4097) })),
      413,
      'body_too_large',
    );
  });

  test('refuses a large streamed body that declares no length', async () => {
    const big = new TextEncoder().encode(padded(10_000));
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (let i = 0; i < big.length; i += 1000) controller.enqueue(big.slice(i, i + 1000));
        controller.close();
      },
    });
    const response = await h.app.join.POST(
      new Request('https://leaderboard.test/api/join', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: stream,
        duplex: 'half',
      } as RequestInit),
    );
    await expectError(response, 413, 'body_too_large');
  });

  test('refuses a declared length over the limit without reading', async () => {
    const response = await h.app.join.POST(
      request('POST', '/api/join', { raw: '{}', headers: { 'content-length': '5000' } }),
    );
    await expectError(response, 413, 'body_too_large');
  });

  test('applies to PUT /api/score and PATCH /api/me too', async () => {
    const key = await join(h, 'limits');
    const huge = JSON.stringify({ leverage_tenths: 1, pad: 'x'.repeat(5000) });
    await expectError(
      await h.app.score.PUT(request('PUT', '/api/score', { key, raw: huge })),
      413,
      'body_too_large',
    );
    await expectError(
      await h.app.me.PATCH(request('PATCH', '/api/me', { key, raw: huge })),
      413,
      'body_too_large',
    );
  });
});

describe('request content type', () => {
  const joinJson = JSON.stringify({ nickname: 'typed', email: 'typed@example.com' });
  const postAs = (contentType: string) =>
    h.app.join.POST(
      request('POST', '/api/join', {
        raw: joinJson,
        ip: nextIp(),
        headers: { 'content-type': contentType },
      }),
    );

  test('a join that is not sent as application/json gets 415 and sends no code', async () => {
    for (const contentType of [
      'text/plain',
      'text/plain;charset=UTF-8',
      'application/x-www-form-urlencoded',
      'multipart/form-data; boundary=x',
      'text/json',
      'application/json-seq',
      'application/jsonx',
    ])
      await expectError(await postAs(contentType), 415, 'unsupported_media_type');
    // No content type at all (a byte body gets none from Request).
    await expectError(
      await h.app.join.POST(
        new Request('https://leaderboard.test/api/join', {
          method: 'POST',
          headers: { 'x-vercel-forwarded-for': nextIp() },
          body: new TextEncoder().encode(joinJson),
        }),
      ),
      415,
      'unsupported_media_type',
    );
    assert.equal(h.mailer.sent.length, 0);
    assert.equal((await h.query('select count(*)::int as n from pending_joins'))[0]!.n, 0);
    // Only the per-IP API limit counted them; the join limits did not.
    const buckets = await h.query('select bucket from rate_counters');
    assert.ok(buckets.every((row) => String(row.bucket).startsWith('api:ip:')));
  });

  test('application/json with a charset, or in another case, is accepted', async () => {
    assert.equal((await postAs('application/json; charset=utf-8')).status, 202);
    assert.equal((await postAs('Application/JSON')).status, 202);
    assert.equal((await postAs('application/json ;charset=UTF-8')).status, 202);
    assert.equal(h.mailer.sent.length, 3);
  });

  test('verify, score and rename refuse it too; GET and DELETE need no content type', async () => {
    const key = await join(h, 'typer');
    const text = { 'content-type': 'text/plain' };
    await expectError(
      await h.app.verify.POST(
        request('POST', '/api/join/verify', {
          raw: JSON.stringify({ pending_id: 'AAAAAAAAAAAAAAAAAAAAAA', code: '123456' }),
          headers: text,
        }),
      ),
      415,
      'unsupported_media_type',
    );
    await expectError(
      await h.app.score.PUT(
        request('PUT', '/api/score', {
          key,
          raw: JSON.stringify({ leverage_tenths: 5 }),
          headers: text,
        }),
      ),
      415,
      'unsupported_media_type',
    );
    await expectError(
      await h.app.me.PATCH(
        request('PATCH', '/api/me', {
          key,
          raw: JSON.stringify({ nickname: 'renamed' }),
          headers: text,
        }),
      ),
      415,
      'unsupported_media_type',
    );
    const standing = (await readJson(await me(h.app, key))) as Record<string, unknown>;
    assert.equal(standing.nickname, 'typer');
    assert.equal(standing.leverage_tenths, null);
    // No body, so request() sends no content type.
    assert.equal((await h.app.me.DELETE(request('DELETE', '/api/me', { key }))).status, 204);
  });
});

describe('keys', () => {
  test('missing, malformed and unknown keys get 401 unknown_key on every "me" endpoint', async () => {
    await join(h, 'someone');
    const unknown = Buffer.alloc(32, 7).toString('base64url');
    const variants: Record<string, string>[] = [
      {},
      { authorization: 'Bearer' },
      { authorization: 'Bearer short' },
      { authorization: `Basic ${unknown}` },
      { authorization: `Bearer ${unknown}` },
      { authorization: `Bearer ${unknown}x` },
    ];
    for (const headers of variants) {
      await expectError(
        await h.app.me.GET(request('GET', '/api/me', { headers })),
        401,
        'unknown_key',
      );
      await expectError(
        await h.app.score.PUT(
          request('PUT', '/api/score', { headers, body: { leverage_tenths: 5 } }),
        ),
        401,
        'unknown_key',
      );
      await expectError(
        await h.app.me.PATCH(
          request('PATCH', '/api/me', { headers, body: { nickname: 'newname' } }),
        ),
        401,
        'unknown_key',
      );
      await expectError(
        await h.app.me.DELETE(request('DELETE', '/api/me', { headers })),
        401,
        'unknown_key',
      );
    }
    const rows = await h.query('select nickname, leverage_tenths from members');
    assert.deepEqual(rows, [{ nickname: 'someone', leverage_tenths: null }]);
  });

  test('the Bearer scheme name is case-insensitive', async () => {
    const key = await join(h, 'scheme');
    const response = await h.app.me.GET(
      request('GET', '/api/me', { headers: { authorization: `bearer ${key}` } }),
    );
    assert.equal(response.status, 200);
  });
});

describe('PUT /api/score', () => {
  test('stores the number and the time, and returns 204 with no body', async () => {
    const key = await join(h, 'scorer');
    const response = await h.app.score.PUT(
      request('PUT', '/api/score', { key, body: { leverage_tenths: 123 } }),
    );
    assert.equal(response.status, 204);
    assert.equal(await response.text(), '');
    const rows = await h.query(
      `select leverage_tenths, score_updated_at = $1::timestamptz as at_clock from members`,
      [START.toISOString()],
    );
    assert.deepEqual(rows, [{ leverage_tenths: 123, at_clock: true }]);
  });

  test('accepts 0, 1000000000 and null', async () => {
    const key = await join(h, 'bounds');
    for (const value of [0, 100000, 1000000000, null]) {
      await score(h.app, key, value);
      const rows = await h.query('select leverage_tenths from members');
      assert.equal(rows[0]!.leverage_tenths, value);
    }
  });

  test('refuses anything else with 400 invalid_leverage', async () => {
    const key = await join(h, 'invalid');
    for (const body of [
      { leverage_tenths: -1 },
      { leverage_tenths: 1000000001 },
      { leverage_tenths: 2 ** 31 },
      { leverage_tenths: Number.MAX_SAFE_INTEGER + 2 },
      { leverage_tenths: 12.3 },
      { leverage_tenths: '123' },
      { leverage_tenths: true },
      { leverage_tenths: [1] },
      {},
    ])
      await expectError(
        await h.app.score.PUT(request('PUT', '/api/score', { key, body })),
        400,
        'invalid_leverage',
      );
    await expectError(
      await h.app.score.PUT(request('PUT', '/api/score', { key, raw: 'nope' })),
      400,
      'invalid_request',
    );
  });
});

describe('GET /api/me', () => {
  test('before any score: no number, not listed, no rank, no time', async () => {
    const key = await join(h, 'Fresh', 'fresh@example.com');
    const response = await me(h.app, key);
    assert.equal(response.status, 200);
    const text = await response.text();
    assert.ok(!text.includes('fresh@example.com'));
    assert.deepEqual(JSON.parse(text), {
      nickname: 'Fresh',
      leverage_tenths: null,
      listed: false,
      rank: null,
      score_updated_at: null,
      country: null,
    });
  });

  test('after a score: listed with rank and an ISO 8601 UTC time', async () => {
    const key = await join(h, 'Scored');
    h.setNow(new Date('2026-10-27T08:09:10.123Z'));
    await score(h.app, key, 57);
    assert.deepEqual(await readJson(await me(h.app, key)), {
      nickname: 'Scored',
      leverage_tenths: 57,
      listed: true,
      rank: 1,
      score_updated_at: '2026-10-27T08:09:10.123Z',
      country: null,
    });
  });

  test('a null score keeps the time but is not listed', async () => {
    const key = await join(h, 'Nulled');
    await score(h.app, key, 40);
    await score(h.app, key, null);
    assert.deepEqual(await readJson(await me(h.app, key)), {
      nickname: 'Nulled',
      leverage_tenths: null,
      listed: false,
      rank: null,
      score_updated_at: START.toISOString(),
      country: null,
    });
  });
});

describe('PATCH /api/me', () => {
  const patch = (key: string, body: unknown) =>
    h.app.me.PATCH(request('PATCH', '/api/me', { key, body }));

  test('renames, trimming the new nickname', async () => {
    const key = await join(h, 'before');
    const response = await patch(key, { nickname: ' after_1 ' });
    assert.equal(response.status, 200);
    assert.deepEqual(await readJson(response), { nickname: 'after_1' });
    assert.deepEqual(await h.query('select nickname, nickname_key from members'), [
      { nickname: 'after_1', nickname_key: 'after_1' },
    ]);
    // The old nickname is free again.
    assert.equal(typeof (await join(h, 'Before')), 'string');
  });

  test('renaming to your own nickname in another case is allowed and stored as typed', async () => {
    const key = await join(h, 'casey');
    const response = await patch(key, { nickname: 'CaSeY' });
    assert.equal(response.status, 200);
    assert.deepEqual(await readJson(response), { nickname: 'CaSeY' });
    assert.equal(
      ((await readJson(await me(h.app, key))) as { nickname: string }).nickname,
      'CaSeY',
    );
    // Same nickname, same case, also fine.
    assert.equal((await patch(key, { nickname: 'CaSeY' })).status, 200);
  });

  test("someone else's nickname in any case is taken", async () => {
    await join(h, 'Owner');
    const key = await join(h, 'renamer');
    await expectError(await patch(key, { nickname: 'owner' }), 409, 'nickname_taken');
    await expectError(await patch(key, { nickname: 'OWNER' }), 409, 'nickname_taken');
    assert.equal(
      ((await readJson(await me(h.app, key))) as { nickname: string }).nickname,
      'renamer',
    );
  });

  test('bad requests and bad nicknames', async () => {
    const key = await join(h, 'patcher');
    await expectError(await patch(key, {}), 400, 'invalid_request');
    await expectError(await patch(key, { nickname: 5 }), 400, 'invalid_request');
    await expectError(
      await h.app.me.PATCH(request('PATCH', '/api/me', { key, raw: '{' })),
      400,
      'invalid_request',
    );
    await expectError(await patch(key, { nickname: 'no' }), 400, 'invalid_nickname');
    await expectError(await patch(key, { nickname: 'bad name' }), 400, 'invalid_nickname');
  });
});

describe('DELETE /api/me', () => {
  test('removes the whole row; the key stops working and the nickname is free', async () => {
    const key = await join(h, 'Leaver', 'leaver@example.com');
    const stay = await join(h, 'stayer');
    await score(h.app, key, 99);
    const response = await h.app.me.DELETE(request('DELETE', '/api/me', { key }));
    assert.equal(response.status, 204);
    assert.equal(await response.text(), '');
    const rows = await h.query(
      `select nickname from members where nickname_key = 'leaver' or email = 'leaver@example.com'`,
    );
    assert.deepEqual(rows, []);
    assert.equal((await h.query('select count(*)::int as n from members'))[0]!.n, 1);
    await expectError(await me(h.app, key), 401, 'unknown_key');
    await expectError(
      await h.app.me.DELETE(request('DELETE', '/api/me', { key })),
      401,
      'unknown_key',
    );
    assert.equal((await me(h.app, stay)).status, 200);
    assert.equal(typeof (await join(h, 'leaver')), 'string');
  });
});

describe('emails and keys stay private', () => {
  test('no endpoint returns an email', async () => {
    const key = await join(h, 'Private', 'secret.person@example.com');
    await score(h.app, key, 10);
    const responses = [
      await me(h.app, key),
      await h.app.leaderboard.GET(request('GET', '/api/leaderboard')),
      await h.app.page.GET(request('GET', '/')),
      await h.app.me.PATCH(request('PATCH', '/api/me', { key, body: { nickname: 'Private2' } })),
    ];
    for (const response of responses) assert.ok(!(await response.text()).includes('secret.person'));
  });

  test('a database failure gives 500 server_error and logs no email or key', async () => {
    const key = await join(h, 'logger');
    const failing = createApp({
      query: async () => {
        throw Object.assign(new Error('Key (email)=(leak@example.com) broke'), { code: 'XX000' });
      },
      log: (message) => h.logs.push(message),
      ipHashSecret: TEST_IP_SECRET,
    });
    await expectError(
      await failing.join.POST(
        request('POST', '/api/join', { body: { nickname: 'leaky', email: 'leak@example.com' } }),
      ),
      500,
      'server_error',
    );
    await expectError(
      await failing.me.GET(request('GET', '/api/me', { key })),
      500,
      'server_error',
    );
    await expectError(await failing.page.GET(request('GET', '/')), 500, 'server_error');
    assert.equal(h.logs.length, 3);
    for (const line of h.logs) {
      assert.equal(line, 'leaderboard: request failed: Error (code XX000)');
      assert.ok(!line.includes('leak@') && !line.includes(key));
    }
  });
});
