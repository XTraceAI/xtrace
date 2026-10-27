import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { after, before, beforeEach, describe, test } from 'node:test';
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

  test("a nickname that is another member's (any case) is 409 nickname_taken", async () => {
    await join(h, 'Owner');
    for (const nickname of ['owner', ' OWNER '])
      await expectError(
        await h.app.join.POST(
          request('POST', '/api/join', { body: { nickname, email: 'new@example.com' } }),
        ),
        409,
        'nickname_taken',
      );
    assert.deepEqual(await pendingRows(), []);
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

  test('only the newest code for an email works', async () => {
    const first = await startJoin(h, 'twice', 't@example.com');
    const firstCode = h.mailer.lastCodeFor('t@example.com');
    const second = await startJoin(h, 'twice', 'T@Example.com');
    const secondCode = h.mailer.lastCodeFor('t@example.com');
    assert.equal((await pendingRows()).length, 1);
    await expectError(await verify(h, first, firstCode), 410, 'code_expired');
    assert.equal((await verify(h, second, secondCode)).status, 201);
  });

  test('if the nickname was taken meanwhile, verify is 409 and the pending join is gone', async () => {
    const a = await startJoin(h, 'Racer', 'a@example.com');
    const b = await startJoin(h, 'racer', 'b@example.com');
    assert.equal((await verify(h, a, h.mailer.lastCodeFor('a@example.com'))).status, 201);
    await expectError(
      await verify(h, b, h.mailer.lastCodeFor('b@example.com')),
      409,
      'nickname_taken',
    );
    assert.deepEqual(await pendingRows(), []);
    assert.equal(await memberCount(), 1);
  });
});

describe('signing back in', () => {
  test('the same email gets a new key, keeps nickname and score; the old key stops', async () => {
    const oldKey = await join(h, 'Returning', 'back@example.com');
    await score(h.app, oldKey, 77);
    await join(h, 'Taken_By_Other', 'other@example.com');

    // Any valid nickname is accepted and ignored, even one another member has.
    const pendingId = await startJoin(h, 'taken_by_other', '  BACK@example.com ');
    const response = await verify(h, pendingId, h.mailer.lastCodeFor('back@example.com'));
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
