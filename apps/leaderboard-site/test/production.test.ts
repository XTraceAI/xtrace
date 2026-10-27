// The production wiring: environment variables, Resend over fetch, failing closed.
import assert from 'node:assert/strict';
import { after, before, beforeEach, describe, test } from 'node:test';
import { createApp } from '../src/app.js';
import { codeEmail, DEFAULT_FROM, MailError, resendMailer } from '../src/mail.js';
import { appFromEnvironment } from '../src/runtime.js';
import { harness, readJson, request, TEST_IP_SECRET, type Harness } from './support.js';

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

const joinBody = { nickname: 'prod_user', email: 'prod@example.com' };
const pendingCount = async () =>
  Number((await h.query('select count(*)::int as n from pending_joins'))[0]!.n);

describe('environment', () => {
  test('without RESEND_API_KEY, join is 503 email_unavailable and nothing is left pending', async () => {
    const app = appFromEnvironment({ IP_HASH_SECRET: 'secret' }, h.query);
    const response = await app.join.POST(request('POST', '/api/join', { body: joinBody }));
    assert.equal(response.status, 503);
    assert.deepEqual(await readJson(response), { error: 'email_unavailable' });
    assert.equal(await pendingCount(), 0);
  });

  test('without IP_HASH_SECRET, every rate-limited endpoint is 503 server_misconfigured', async () => {
    const app = appFromEnvironment({ RESEND_API_KEY: 're_test' }, h.query);
    const key = { key: 'A'.repeat(43) };
    const responses = [
      await app.join.POST(request('POST', '/api/join', { body: joinBody })),
      await app.verify.POST(
        request('POST', '/api/join/verify', { body: { pending_id: 'x', code: '123456' } }),
      ),
      await app.score.PUT(request('PUT', '/api/score', { ...key, body: { leverage_tenths: 1 } })),
      await app.me.GET(request('GET', '/api/me', key)),
      await app.me.PATCH(request('PATCH', '/api/me', { ...key, body: { nickname: 'abc' } })),
      await app.me.DELETE(request('DELETE', '/api/me', key)),
    ];
    for (const response of responses) {
      assert.equal(response.status, 503);
      assert.deepEqual(await readJson(response), { error: 'server_misconfigured' });
    }
    // An empty secret counts as missing.
    const empty = appFromEnvironment({ RESEND_API_KEY: 're_test', IP_HASH_SECRET: '' }, h.query);
    assert.equal((await empty.me.GET(request('GET', '/api/me', key))).status, 503);
    // The public reads still work.
    assert.equal((await app.leaderboard.GET(request('GET', '/api/leaderboard'))).status, 200);
    assert.equal((await app.page.GET(request('GET', '/'))).status, 200);
    assert.equal(await pendingCount(), 0);
  });
});

describe('Resend', () => {
  type Call = { url: string; init: RequestInit };
  const fakeFetch = (status: number, calls: Call[]) =>
    (async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      return new Response('{"id":"x"}', { status });
    }) as typeof fetch;

  test('sends one POST with the key, sender, recipient, subject and plain text', async () => {
    const calls: Call[] = [];
    await resendMailer({ apiKey: 're_123', fetch: fakeFetch(200, calls) }).sendCode(
      'to@example.com',
      '042917',
    );
    assert.equal(calls.length, 1);
    const { url, init } = calls[0]!;
    assert.equal(url, 'https://api.resend.com/emails');
    assert.equal(init.method, 'POST');
    const headers = new Headers(init.headers);
    assert.equal(headers.get('authorization'), 'Bearer re_123');
    assert.equal(headers.get('content-type'), 'application/json');
    assert.deepEqual(JSON.parse(String(init.body)), {
      from: DEFAULT_FROM,
      to: ['to@example.com'],
      subject: 'Your XTrace leaderboard code: 042917',
      text:
        'Your code is 042917. It expires in 15 minutes.\n' +
        'Enter it in the XTrace app to join the leaderboard.\n' +
        'If you did not ask to join the XTrace leaderboard, you can ignore this email.\n',
    });
    assert.equal(DEFAULT_FROM, 'XTrace Leaderboard <noreply@emails.xtrace.ai>');
    assert.equal(codeEmail('000001').subject, 'Your XTrace leaderboard code: 000001');
  });

  test('LEADERBOARD_FROM_EMAIL replaces the sender', async () => {
    const calls: Call[] = [];
    await resendMailer({
      apiKey: 're_123',
      from: 'Other <x@example.com>',
      fetch: fakeFetch(200, calls),
    }).sendCode('to@example.com', '123456');
    assert.equal(JSON.parse(String(calls[0]!.init.body)).from, 'Other <x@example.com>');
  });

  test('a refused send is a 503, removes the pending join and logs no secret', async () => {
    const calls: Call[] = [];
    const mailer = resendMailer({ apiKey: 're_secret_key', fetch: fakeFetch(422, calls) });
    await assert.rejects(mailer.sendCode('to@example.com', '123456'), MailError);
    const app = createApp({
      query: h.query,
      mailer,
      ipHashSecret: TEST_IP_SECRET,
      log: (message) => h.logs.push(message),
    });
    const response = await app.join.POST(request('POST', '/api/join', { body: joinBody }));
    assert.equal(response.status, 503);
    assert.deepEqual(await readJson(response), { error: 'email_unavailable' });
    assert.equal(await pendingCount(), 0);
    const sentCode = /code: ([0-9]{6})/.exec(
      JSON.parse(String(calls.at(-1)!.init.body)).subject,
    )![1]!;
    const text = h.logs.join('\n');
    assert.deepEqual(h.logs, ['leaderboard: email not sent (MailError)']);
    for (const secret of ['prod@', sentCode, 're_secret_key']) assert.ok(!text.includes(secret));
  });
});
