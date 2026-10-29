import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { after, before, beforeEach, describe, test } from 'node:test';
import { createApp } from '../src/app.js';
import {
  harness,
  join,
  me,
  MINUTE,
  readJson,
  request,
  score,
  START,
  startJoin,
  TEST_IP_SECRET,
  verify,
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

const sha256 = (text: string) => createHash('sha256').update(text).digest('hex');

async function expectError(response: Response, status: number, code: string) {
  assert.equal(response.status, status);
  assert.deepEqual(await readJson(response), { error: code });
}

const pendingRows = () =>
  h.query(
    `select id, nickname, nickname_key, email, encode(code_hash, 'hex') as code_hash, attempts,
            expires_at = $1::timestamptz as expires_in_15
     from pending_joins`,
    [new Date(START.getTime() + 15 * MINUTE).toISOString()],
  );
const verifyWith = (pendingId: string, code: string, extra: Record<string, unknown>) =>
  h.app.verify.POST(
    request('POST', '/api/join/verify', { body: { pending_id: pendingId, code, ...extra } }),
  );
const memberCount = async () =>
  Number((await h.query('select count(*)::int as n from members'))[0]!.n);

describe('POST /api/join', () => {
  test('stores a pending join with only the code hash, emails the code, answers 202', async () => {
    const response = await h.app.join.POST(
      request('POST', '/api/join', {
        body: { nickname: '  Ada_01 ', email: '  Ada@Example.COM ' },
        ip: '192.0.2.1',
      }),
    );
    assert.equal(response.status, 202);
    const body = (await readJson(response)) as { pending_id: string };
    assert.deepEqual(Object.keys(body), ['pending_id']);
    assert.match(body.pending_id, /^[A-Za-z0-9_-]{22}$/);
    assert.equal(Buffer.from(body.pending_id, 'base64url').length, 16);

    assert.equal(h.mailer.sent.length, 1);
    const { to, code } = h.mailer.sent[0]!;
    assert.equal(to, 'ada@example.com');
    assert.match(code, /^[0-9]{6}$/);

    const rows = await pendingRows();
    assert.deepEqual(rows, [
      {
        id: body.pending_id,
        nickname: 'Ada_01',
        nickname_key: 'ada_01',
        email: 'ada@example.com',
        code_hash: sha256(code),
        attempts: 0,
        expires_in_15: true,
      },
    ]);
    const whole = JSON.stringify(
      await h.query('select row_to_json(pending_joins)::text as t from pending_joins'),
    );
    assert.ok(!whole.includes(`"${code}"`) && !whole.includes(`:${code}`));
    assert.equal(await memberCount(), 0);
  });

  test('join never checks the nickname: a taken one with a new email still gets 202', async () => {
    await join(h, 'Owner');
    for (const nickname of ['owner', ' OWNER ']) {
      const response = await h.app.join.POST(
        request('POST', '/api/join', { body: { nickname, email: 'new@example.com' } }),
      );
      assert.equal(response.status, 202);
    }
  });

  test('join gives no way to tell whether an email belongs to a member', async () => {
    await join(h, 'Owner', 'owner@example.com');
    await join(h, 'Member', 'member@example.com');
    // A known-taken nickname used to give 202 for a member's email and 409 otherwise.
    const ask = (email: string) =>
      h.app.join.POST(request('POST', '/api/join', { body: { nickname: 'Owner', email } }));
    const member = await ask('member@example.com');
    const stranger = await ask('stranger@example.com');
    assert.equal(member.status, 202);
    assert.equal(stranger.status, 202);
    assert.deepEqual([...member.headers], [...stranger.headers]);
    const shape = (body: unknown) =>
      JSON.stringify(body).replace(/"pending_id":"[A-Za-z0-9_-]{22}"/, '"pending_id":"x"');
    assert.equal(shape(await readJson(member)), shape(await readJson(stranger)));
  });

  test('a failed send removes the pending join and answers 503 email_unavailable', async () => {
    h.mailer.failNext = true;
    await expectError(
      await h.app.join.POST(
        request('POST', '/api/join', { body: { nickname: 'mailless', email: 'm@example.com' } }),
      ),
      503,
      'email_unavailable',
    );
    assert.deepEqual(await pendingRows(), []);
    assert.deepEqual(h.logs, ['leaderboard: email not sent (Error)']);
  });
});

describe('POST /api/join/verify', () => {
  test('the right code creates the member with a new key; the code works once', async () => {
    const pendingId = await startJoin(h, 'Verified', 'v@example.com');
    const code = h.mailer.lastCodeFor('v@example.com');
    const response = await verify(h, pendingId, code);
    assert.equal(response.status, 201);
    const body = (await readJson(response)) as Record<string, unknown>;
    assert.deepEqual(Object.keys(body).sort(), ['key', 'nickname', 'rejoined']);
    assert.equal(body.nickname, 'Verified');
    assert.equal(body.rejoined, false);
    assert.match(String(body.key), /^[A-Za-z0-9_-]{43}$/);
    const rows = await h.query(
      `select nickname, email, encode(key_hash, 'hex') as key_hash, row_to_json(members)::text as whole
       from members`,
    );
    assert.equal(rows.length, 1);
    assert.equal(rows[0]!.key_hash, sha256(String(body.key)));
    assert.equal(rows[0]!.email, 'v@example.com');
    assert.ok(!String(rows[0]!.whole).includes(String(body.key)));
    assert.deepEqual(await pendingRows(), []);
    await expectError(await verify(h, pendingId, code), 410, 'code_expired');
    assert.equal((await me(h.app, String(body.key))).status, 200);
  });

  test('bodies: invalid_request for missing or non-string fields', async () => {
    const post = (body: unknown) =>
      h.app.verify.POST(request('POST', '/api/join/verify', { body }));
    await expectError(await post({ code: '123456' }), 400, 'invalid_request');
    await expectError(await post({ pending_id: 'x' }), 400, 'invalid_request');
    await expectError(await post({ pending_id: 1, code: '123456' }), 400, 'invalid_request');
    await expectError(await post({ pending_id: 'x', code: 123456 }), 400, 'invalid_request');
    await expectError(
      await h.app.verify.POST(request('POST', '/api/join/verify', { raw: 'nope' })),
      400,
      'invalid_request',
    );
  });

  test('a code that is not exactly 6 ASCII digits is invalid_code and not an attempt', async () => {
    const pendingId = await startJoin(h, 'shape', 's@example.com');
    for (const code of ['12345', '1234567', 'abcdef', ' 12345', '12345 ', '１２３４５６', ''])
      await expectError(await verify(h, pendingId, code), 400, 'invalid_code');
    assert.equal((await pendingRows())[0]!.attempts, 0);
  });

  test('unknown pending ids are 410 code_expired', async () => {
    await expectError(await verify(h, 'AAAAAAAAAAAAAAAAAAAAAA', '123456'), 410, 'code_expired');
  });

  test('a code expires after 15 minutes', async () => {
    const early = await startJoin(h, 'early', 'early@example.com');
    h.advance(15 * MINUTE - 1);
    assert.equal((await verify(h, early, h.mailer.lastCodeFor('early@example.com'))).status, 201);

    h.setNow(START);
    const late = await startJoin(h, 'late', 'late@example.com');
    h.advance(15 * MINUTE);
    await expectError(
      await verify(h, late, h.mailer.lastCodeFor('late@example.com')),
      410,
      'code_expired',
    );
    assert.deepEqual(await pendingRows(), []);
  });

  test('a wrong code counts; the 5th wrong attempt ends the pending join', async () => {
    const pendingId = await startJoin(h, 'guesser', 'g@example.com');
    const code = h.mailer.lastCodeFor('g@example.com');
    const wrong = code === '000000' ? '111111' : '000000';
    for (let attempt = 1; attempt <= 4; attempt += 1) {
      await expectError(await verify(h, pendingId, wrong), 400, 'invalid_code');
      assert.equal((await pendingRows())[0]!.attempts, attempt);
    }
    await expectError(await verify(h, pendingId, wrong), 400, 'invalid_code');
    assert.deepEqual(await pendingRows(), []);
    await expectError(await verify(h, pendingId, code), 410, 'code_expired');
    assert.equal(await memberCount(), 0);
  });

  test('after 4 wrong attempts the right code still works', async () => {
    const pendingId = await startJoin(h, 'lucky', 'l@example.com');
    const code = h.mailer.lastCodeFor('l@example.com');
    const wrong = code === '000000' ? '111111' : '000000';
    for (let attempt = 1; attempt <= 4; attempt += 1) await verify(h, pendingId, wrong);
    assert.equal((await verify(h, pendingId, code)).status, 201);
  });

  test('a new join does not cancel earlier live codes for the same email', async () => {
    const first = await startJoin(h, 'twice', 't@example.com');
    const firstCode = h.mailer.lastCodeFor('t@example.com');
    const second = await startJoin(h, 'twice', 'T@Example.com');
    const secondCode = h.mailer.lastCodeFor('t@example.com');
    assert.equal((await pendingRows()).length, 2);
    const a = await verify(h, first, firstCode);
    assert.equal(a.status, 201);
    assert.equal(((await readJson(a)) as { rejoined: boolean }).rejoined, false);
    // The second code still works; the email is now a member, so it signs back in.
    const b = await verify(h, second, secondCode);
    assert.equal(b.status, 201);
    assert.equal(((await readJson(b)) as { rejoined: boolean }).rejoined, true);
    assert.equal(await memberCount(), 1);
  });

  test("someone else's joins for my email do not break my code", async () => {
    const mine = await startJoin(h, 'Victim', 'victim@example.com', '192.0.2.200');
    const myCode = h.mailer.lastCodeFor('victim@example.com');
    for (let i = 0; i < 3; i += 1)
      assert.equal(
        (
          await h.app.join.POST(
            request('POST', '/api/join', {
              body: { nickname: 'Attacker', email: 'victim@example.com' },
              ip: '198.51.100.66',
            }),
          )
        ).status,
        202,
      );
    const response = await verify(h, mine, myCode, '192.0.2.200');
    assert.equal(response.status, 201);
    assert.equal(((await readJson(response)) as { nickname: string }).nickname, 'Victim');
  });

  test('a nickname tried at verify and refused with 409 is kept for the next try', async () => {
    const holder = await join(h, 'Wanted');
    const pendingId = await startJoin(h, 'First_Pick', 'kept@example.com');
    const code = h.mailer.lastCodeFor('kept@example.com');
    await expectError(
      await verifyWith(pendingId, code, { nickname: 'wanted' }),
      409,
      'nickname_taken',
    );
    const rows = await pendingRows();
    assert.equal(rows[0]!.nickname, 'wanted');
    assert.equal(rows[0]!.nickname_key, 'wanted');
    // Once the nickname is free, a verify without `nickname` uses the saved one.
    assert.equal(
      (await h.app.me.DELETE(request('DELETE', '/api/me', { key: holder }))).status,
      204,
    );
    const response = await verify(h, pendingId, code);
    assert.equal(response.status, 201);
    assert.equal(((await readJson(response)) as { nickname: string }).nickname, 'wanted');
  });

  test('a racing 410 does not remove a pending join that a 409 just put back', async () => {
    await join(h, 'Busy');
    const pendingId = await startJoin(h, 'busy', 'race@example.com');
    const code = h.mailer.lastCodeFor('race@example.com');
    // Order the two requests' statements: A claims the row; B's claim then finds
    // nothing; A puts the row back; only then does B's 410 cleanup run.
    let claimed!: () => void;
    let missed!: () => void;
    let putBack!: () => void;
    const claimedP = new Promise<void>((resolve) => (claimed = resolve));
    const missedP = new Promise<void>((resolve) => (missed = resolve));
    const putBackP = new Promise<void>((resolve) => (putBack = resolve));
    const gated = createApp({
      query: async (text, params) => {
        const isClaim =
          /delete from pending_joins where id = \$1 and expires_at > .* returning id,/s.test(text);
        const isPutBack = /on conflict \(id\) do nothing/.test(text);
        const isDeadCleanup =
          /^\s*delete from pending_joins\s+where id = \$1 and \(expires_at/.test(text);
        if (isPutBack) await missedP;
        if (isDeadCleanup) await putBackP;
        const rows = await h.query(text, params);
        if (isClaim) (rows.length ? claimed : missed)();
        if (isPutBack) putBack();
        return rows;
      },
      clock: h.now,
      mailer: h.mailer,
      ipHashSecret: TEST_IP_SECRET,
    });
    const post = (ip: string) =>
      gated.verify.POST(
        request('POST', '/api/join/verify', { body: { pending_id: pendingId, code }, ip }),
      );
    const a = post('192.0.2.210');
    await claimedP;
    const b = post('192.0.2.211');
    const [ra, rb] = await Promise.all([a, b]);
    assert.equal(ra.status, 409);
    assert.equal(rb.status, 410);
    assert.equal((await pendingRows()).length, 1);
    assert.equal((await verifyWith(pendingId, code, { nickname: 'not_busy' })).status, 201);
  });

  test('a taken nickname at verify is 409; the pending join stays for another try', async () => {
    const a = await startJoin(h, 'Racer', 'a@example.com');
    const b = await startJoin(h, 'racer', 'b@example.com');
    assert.equal((await verify(h, a, h.mailer.lastCodeFor('a@example.com'))).status, 201);
    const codeB = h.mailer.lastCodeFor('b@example.com');
    const before = await pendingRows();
    await expectError(await verify(h, b, codeB), 409, 'nickname_taken');
    // Kept unchanged: same id, code hash, attempts and expiry.
    assert.deepEqual(await pendingRows(), before);
    assert.equal(await memberCount(), 1);
    // The same code with a free nickname now joins.
    const response = await verifyWith(b, codeB, { nickname: ' Racer_2 ' });
    assert.equal(response.status, 201);
    const body = (await readJson(response)) as Record<string, unknown>;
    assert.equal(body.nickname, 'Racer_2');
    assert.equal(body.rejoined, false);
    assert.deepEqual(await pendingRows(), []);
    await expectError(await verify(h, b, codeB), 410, 'code_expired');
  });

  test('409s do not use up the 5 wrong-code attempts', async () => {
    await join(h, 'Popular');
    const pendingId = await startJoin(h, 'popular', 'p@example.com');
    const code = h.mailer.lastCodeFor('p@example.com');
    const wrong = code === '000000' ? '111111' : '000000';
    for (let i = 0; i < 4; i += 1) await verify(h, pendingId, wrong);
    for (let i = 0; i < 8; i += 1)
      await expectError(await verify(h, pendingId, code), 409, 'nickname_taken');
    assert.equal((await pendingRows())[0]!.attempts, 4);
    assert.equal((await verifyWith(pendingId, code, { nickname: 'fresh_name' })).status, 201);
  });

  test('the optional nickname: checked without counting an attempt; null means none', async () => {
    const pendingId = await startJoin(h, 'Original', 'o@example.com');
    const code = h.mailer.lastCodeFor('o@example.com');
    await expectError(
      await verifyWith(pendingId, code, { nickname: 'no' }),
      400,
      'invalid_nickname',
    );
    await expectError(
      await verifyWith(pendingId, code, { nickname: 'bad name' }),
      400,
      'invalid_nickname',
    );
    await expectError(await verifyWith(pendingId, code, { nickname: 7 }), 400, 'invalid_request');
    // A bad code is reported before a bad nickname.
    await expectError(await verifyWith(pendingId, '12', { nickname: 'no' }), 400, 'invalid_code');
    assert.equal((await pendingRows())[0]!.attempts, 0);
    const response = await verifyWith(pendingId, code, { nickname: null });
    assert.equal(((await readJson(response)) as { nickname: string }).nickname, 'Original');
  });

  test('racing duplicate verifies with the right code make one member and use the code once', async () => {
    const pendingId = await startJoin(h, 'Twin', 'twin@example.com');
    const code = h.mailer.lastCodeFor('twin@example.com');
    const results = await Promise.all(
      Array.from({ length: 6 }, (_, i) => verifyWith(pendingId, code, { nickname: `twin_${i}` })),
    );
    const statuses = results.map((response) => response.status).sort();
    assert.deepEqual(statuses, [201, 410, 410, 410, 410, 410]);
    assert.equal(await memberCount(), 1);
    assert.deepEqual(await pendingRows(), []);
  });

  test('racing wrong guesses never count past 5 and never let a guess through after', async () => {
    const pendingId = await startJoin(h, 'Target', 'target@example.com');
    const code = h.mailer.lastCodeFor('target@example.com');
    const guesses = Array.from({ length: 12 }, (_, i) =>
      String((Number(code) + 1 + i) % 1_000_000).padStart(6, '0'),
    );
    const results = await Promise.all([
      ...guesses.map((guess) => verify(h, pendingId, guess)),
      verify(h, pendingId, code),
    ]);
    const statuses = results.map((response) => response.status);
    const wrongCounted = statuses.filter((status) => status === 400).length;
    assert.ok(wrongCounted <= 5, `${wrongCounted} wrong guesses counted`);
    const joined = statuses.at(-1) === 201;
    // The right code only works if it was compared before the 5th wrong guess.
    assert.ok(joined ? wrongCounted <= 5 : statuses.at(-1) === 410);
    assert.equal(await memberCount(), joined ? 1 : 0);
  });
});

describe('signing back in', () => {
  test('the same email gets a new key, keeps nickname and score; the old key stops', async () => {
    const oldKey = await join(h, 'Returning', 'back@example.com');
    await score(h.app, oldKey, 77);
    await join(h, 'Taken_By_Other', 'other@example.com');

    // Any valid nickname is accepted and ignored, even one another member has.
    const pendingId = await startJoin(h, 'taken_by_other', '  BACK@example.com ');
    // A nickname sent at verify is ignored for a member's email too.
    const response = await verifyWith(pendingId, h.mailer.lastCodeFor('back@example.com'), {
      nickname: 'Brand_New',
    });
    assert.equal(response.status, 201);
    const body = (await readJson(response)) as { key: string; nickname: string; rejoined: boolean };
    assert.equal(body.nickname, 'Returning');
    assert.equal(body.rejoined, true);
    assert.notEqual(body.key, oldKey);

    await expectError(await me(h.app, oldKey), 401, 'unknown_key');
    const standing = (await readJson(await me(h.app, body.key))) as Record<string, unknown>;
    assert.equal(standing.nickname, 'Returning');
    assert.equal(standing.leverage_tenths, 77);
    assert.equal(await memberCount(), 2);
  });

  test('join answers a member and a newcomer the same way', async () => {
    await join(h, 'Member', 'member@example.com');
    const member = await h.app.join.POST(
      request('POST', '/api/join', { body: { nickname: 'anything', email: 'member@example.com' } }),
    );
    const newcomer = await h.app.join.POST(
      request('POST', '/api/join', {
        body: { nickname: 'anything2', email: 'newcomer@example.com' },
      }),
    );
    assert.equal(member.status, 202);
    assert.equal(newcomer.status, 202);
    assert.deepEqual(Object.keys((await readJson(member)) as object), ['pending_id']);
    assert.deepEqual(Object.keys((await readJson(newcomer)) as object), ['pending_id']);
  });

  test('members.email is unique in the database', async () => {
    await join(h, 'unique1', 'u@example.com');
    await assert.rejects(
      h.query(
        `insert into members (nickname, nickname_key, email, key_hash)
         values ('unique2', 'unique2', 'u@example.com', decode('00', 'hex'))`,
      ),
      /unique/,
    );
  });
});

describe('codes and emails stay out of the logs', () => {
  test('after joins, wrong codes, a failed send and a database error', async () => {
    const pendingId = await startJoin(h, 'quiet', 'quiet@example.com');
    const code = h.mailer.lastCodeFor('quiet@example.com');
    await verify(h, pendingId, code === '000000' ? '111111' : '000000');
    await verify(h, pendingId, code);
    h.mailer.failNext = true;
    await h.app.join.POST(
      request('POST', '/api/join', { body: { nickname: 'quiet2', email: 'quiet2@example.com' } }),
    );
    await h.query('drop table pending_joins');
    try {
      const failed = await h.app.join.POST(
        request('POST', '/api/join', { body: { nickname: 'quiet3', email: 'quiet3@example.com' } }),
      );
      assert.equal(failed.status, 500);
    } finally {
      const { applySchema } = await import('../src/db.js');
      const { readSchema } = await import('../src/local.js');
      await applySchema(h.query, readSchema());
    }
    assert.ok(h.logs.length >= 2);
    const text = h.logs.join('\n');
    for (const secret of ['quiet@', 'quiet2@', 'quiet3@', code, pendingId])
      assert.ok(!text.includes(secret), `logs mention ${secret}`);
  });
});
