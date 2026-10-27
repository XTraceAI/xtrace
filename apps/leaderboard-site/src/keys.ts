// Member keys. Only the SHA-256 of a key is stored; the key itself is returned
// once, on join, and never logged.
import { createHash, randomBytes } from 'node:crypto';

const KEY = /^[A-Za-z0-9_-]{43}$/;

export function hashKey(key: string): string {
  return createHash('sha256').update(key, 'utf8').digest('hex');
}

/** 32 random bytes, base64url without padding (43 characters). */
export function newKey(): { key: string; keyHashHex: string } {
  const key = randomBytes(32).toString('base64url');
  return { key, keyHashHex: hashKey(key) };
}

/** The key hash from `Authorization: Bearer <key>`, or null when absent or malformed. */
export function bearerKeyHash(request: Request): string | null {
  const match = /^Bearer[ \t]+(\S+)[ \t]*$/i.exec(request.headers.get('authorization') ?? '');
  const key = match?.[1];
  return key && KEY.test(key) ? hashKey(key) : null;
}
