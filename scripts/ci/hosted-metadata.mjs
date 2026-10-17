import { open } from 'node:fs/promises';

export const metadataLimit = 65_536;
const headerNames = new Set([
  'content-type',
  'server',
  'via',
  'retry-after',
  'x-github-request-id',
  'x-request-id',
  'x-ratelimit-limit',
  'x-ratelimit-remaining',
  'x-ratelimit-reset',
  'x-ratelimit-used',
  'x-ratelimit-resource',
]);

// curl writes raw response files only inside disposable staging. Read at most
// one extra byte to detect truncation; never upload raw response headers.
export async function readMetadataCapture(path) {
  let file;
  try {
    file = await open(path, 'r');
  } catch (error) {
    if (error.code === 'ENOENT') return { text: '', truncated: false };
    throw error;
  }
  try {
    const bytes = Buffer.alloc(metadataLimit + 1);
    let bytesRead = 0;
    while (bytesRead < bytes.length) {
      const part = await file.read(bytes, bytesRead, bytes.length - bytesRead, bytesRead);
      if (part.bytesRead === 0) break;
      bytesRead += part.bytesRead;
    }
    return {
      text: bytes.subarray(0, Math.min(bytesRead, metadataLimit)).toString('utf8'),
      truncated: bytesRead > metadataLimit,
    };
  } finally {
    await file.close();
  }
}

export function inspectMetadataResponse(result, headers, body) {
  const status = /^\d{3}$/.test(result.stdout?.trim() || '') ? Number(result.stdout.trim()) : null;
  const diagnostic = {
    status,
    curlExit: result.status,
    signal: result.signal || null,
    transportError: result.error?.code || null,
    statusLines: [],
    headers: {},
    headersTruncated: headers.truncated,
    bodyTruncated: body.truncated,
    stderr: (result.stderr || '').slice(0, 2048),
  };
  for (const line of headers.text.slice(0, metadataLimit).split(/\r?\n/)) {
    if (/^HTTP\/\S+ \d{3}\b/.test(line)) diagnostic.statusLines.push(line.slice(0, 200));
    const colon = line.indexOf(':');
    const name = line.slice(0, colon).toLowerCase();
    if (colon > 0 && headerNames.has(name))
      diagnostic.headers[name] = line
        .slice(colon + 1)
        .trim()
        .slice(0, 512);
  }
  diagnostic.statusLines = diagnostic.statusLines.slice(-4);
  let failure;
  let value;
  if (headers.truncated || body.truncated) failure = 'Metadata response exceeded the capture limit';
  else if (result.error || result.signal || result.status !== 0)
    failure = 'Metadata transport failed';
  else if (status === null || status < 200 || status >= 300)
    failure = 'Metadata HTTP response was not successful';
  else {
    try {
      value = JSON.parse(body.text);
    } catch {
      failure = 'Metadata response was not valid JSON';
    }
  }
  if (failure) diagnostic.errorBody = body.text.slice(0, 2048);
  return { diagnostic, failure, value };
}
