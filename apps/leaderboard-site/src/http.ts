// Small response and request-body helpers shared by every endpoint.

/** Request bodies larger than this many bytes are refused with 413. */
export const MAX_BODY_BYTES = 4096;

const API_HEADERS = { 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' };

export function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...API_HEADERS, 'content-type': 'application/json; charset=utf-8' },
  });
}

export function noContent(): Response {
  return new Response(null, { status: 204, headers: API_HEADERS });
}

export function fail(status: number, code: string): Response {
  return json(status, { error: code });
}

export type Body = { ok: true; value: Record<string, unknown> } | { ok: false; response: Response };

/**
 * Reads a JSON object body of at most MAX_BODY_BYTES. Too large → 413
 * `body_too_large`; not UTF-8 JSON or not an object → 400 `invalid_request`.
 */
export async function readJsonObject(request: Request): Promise<Body> {
  const declared = Number(request.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES)
    return { ok: false, response: fail(413, 'body_too_large') };
  const chunks: Uint8Array[] = [];
  let total = 0;
  if (request.body) {
    const reader = request.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > MAX_BODY_BYTES) {
        await reader.cancel().catch(() => undefined);
        return { ok: false, response: fail(413, 'body_too_large') };
      }
      chunks.push(value);
    }
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    return { ok: false, response: fail(400, 'invalid_request') };
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value))
    return { ok: false, response: fail(400, 'invalid_request') };
  return { ok: true, value: value as Record<string, unknown> };
}
