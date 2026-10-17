import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import test from 'node:test';
import { inspectMetadataResponse, readMetadataCapture, metadataLimit } from './hosted-metadata.mjs';

const capture = (text) => ({ text, truncated: false });
const success = { stdout: '200', stderr: '', status: 0 };

test('403 keeps diagnostic evidence and rejects the body before interpreting it as tag JSON', () => {
  const response = inspectMetadataResponse(
    { ...success, stdout: '403', status: 56 },
    capture(
      'HTTP/2 403\r\nServer: example-gateway\r\nVia: intermediary\r\nX-RateLimit-Remaining: 0\r\nRetry-After: 60\r\nX-GitHub-Request-Id: request-1\r\n',
    ),
    capture('{"message":"example denial; cause must be read from evidence"}'),
  );
  assert.equal(response.value, undefined);
  assert.equal(response.failure, 'Metadata transport failed');
  assert.equal(response.diagnostic.status, 403);
  assert.equal(response.diagnostic.curlExit, 56);
  assert.deepEqual(response.diagnostic.statusLines, ['HTTP/2 403']);
  assert.equal(response.diagnostic.headers['x-ratelimit-remaining'], '0');
  assert.equal(response.diagnostic.headers.via, 'intermediary');
  assert.match(response.diagnostic.errorBody, /example denial/);
  assert.equal(
    inspectMetadataResponse({ ...success, stdout: '403' }, capture(''), capture('not JSON'))
      .failure,
    'Metadata HTTP response was not successful',
  );
});

test('only a completed 2xx JSON response can proceed', () => {
  assert.deepEqual(
    inspectMetadataResponse(success, capture(''), capture('{"ref":"refs/tags/v0.1.6"}')).value,
    { ref: 'refs/tags/v0.1.6' },
  );
  for (const result of [
    { ...success, stdout: '302' },
    { ...success, stdout: '000' },
    { ...success, signal: 'SIGKILL' },
    { ...success, error: { code: 'ETIMEDOUT' } },
    { ...success, status: 22 },
  ])
    assert.ok(inspectMetadataResponse(result, capture(''), capture('{}')).failure);
  assert.equal(
    inspectMetadataResponse(success, capture(''), capture('bad JSON')).failure,
    'Metadata response was not valid JSON',
  );
});

test('diagnostics allowlist headers and bound repeated status lines, values, error body and stderr', () => {
  const headers =
    'HTTP/2 403\r\n'.repeat(10) +
    `Server: ${'s'.repeat(1000)}\r\nSet-Cookie: private\r\nAuthorization: private\r\nLocation: https://example.invalid/private\r\n`;
  const response = inspectMetadataResponse(
    { ...success, stdout: '403', stderr: 'e'.repeat(5000) },
    capture(headers),
    capture('b'.repeat(5000)),
  );
  assert.deepEqual(Object.keys(response.diagnostic.headers), ['server']);
  assert.equal(response.diagnostic.statusLines.length, 4);
  assert.equal(response.diagnostic.headers.server.length, 512);
  assert.equal(response.diagnostic.errorBody.length, 2048);
  assert.equal(response.diagnostic.stderr.length, 2048);
});

test('raw captures are bounded on disk reads and truncated responses cannot pass', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'metadata-capture-'));
  try {
    const path = join(directory, 'response');
    assert.deepEqual(await readMetadataCapture(path), capture(''));
    await writeFile(path, 'x'.repeat(metadataLimit + 20));
    const data = await readMetadataCapture(path);
    assert.equal(data.text.length, metadataLimit);
    assert.equal(data.truncated, true);
    for (const [headers, body] of [
      [data, capture('{}')],
      [capture(''), data],
    ])
      assert.equal(
        inspectMetadataResponse(success, headers, body).failure,
        'Metadata response exceeded the capture limit',
      );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
