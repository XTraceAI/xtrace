// Nickname, email and score rules. The app shows the server's answer, so these
// are the only copies of these rules.

const NICKNAME = /^[A-Za-z0-9_-]{3,20}$/;
export const MAX_LEVERAGE_TENTHS = 100000;
const MAX_EMAIL_LENGTH = 254;

export type Nickname = { nickname: string; nicknameKey: string };

/** Trims and checks a nickname. `nicknameKey` is what uniqueness is decided on. */
export function parseNickname(value: string): Nickname | null {
  const nickname = value.trim();
  if (!NICKNAME.test(nickname)) return null;
  return { nickname, nicknameKey: nickname.toLowerCase() };
}

/** Trims, checks and lowercases an email. Returns null when it is not acceptable. */
export function parseEmail(value: string): string | null {
  const email = value.trim();
  if (email.length === 0 || email.length > MAX_EMAIL_LENGTH || /\s/.test(email)) return null;
  const parts = email.split('@');
  if (parts.length !== 2) return null;
  const [local, domain] = parts as [string, string];
  if (local.length === 0) return null;
  if (!domain.includes('.') || domain.startsWith('.') || domain.endsWith('.')) return null;
  return email.toLowerCase();
}

/** A 7-day leverage times ten, or null for "no number". Undefined means invalid. */
export function parseLeverageTenths(value: unknown): number | null | undefined {
  if (value === null) return null;
  if (typeof value !== 'number' || !Number.isInteger(value)) return undefined;
  if (value < 0 || value > MAX_LEVERAGE_TENTHS) return undefined;
  return value;
}
