// Member keys. Only the SHA-256 of a key is stored; the key itself is returned
// once, on join, and never logged.
import { createHash, randomBytes, randomInt, timingSafeEqual } from 'node:crypto';

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

/** A pending join's id: 16 random bytes, base64url without padding (22 characters). */
export function newPendingId(): string {
  return randomBytes(16).toString('base64url');
}

/** A 6-digit code, 000000–999999, from the CSPRNG. */
export function newCode(): string {
  return String(randomInt(0, 1_000_000)).padStart(6, '0');
}

/** Codes are stored and compared only as SHA-256 hashes. */
export function hashCode(code: string): Buffer {
  return createHash('sha256').update(code, 'utf8').digest();
}

export function sameHash(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && timingSafeEqual(a, b);
}
