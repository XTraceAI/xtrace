// The one country rule. A member's country is only ever read from the header
// Vercel adds from the caller's IP address; the IP itself is never stored.
// schema.sql's `members_country_check` stores the same rule in the database.

/**
 * The header whose documented meaning is the two-letter ISO 3166-1 country of
 * the requester's public IP address (https://vercel.com/docs/headers/request-headers).
 * Vercel's docs do not say whether a copy sent by the client is replaced.
 */
export const COUNTRY_HEADER = 'x-vercel-ip-country';

/**
 * Whether `value` is a country this site stores and shows: exactly two
 * uppercase ASCII letters, and not `XX` (which means no country).
 */
export function isCountryCode(value: string): boolean {
  return /^[A-Z]{2}$/.test(value) && value !== 'XX';
}

/**
 * The caller's country, or null when it is not known: no header, or a value
 * that is not a country code ([`isCountryCode`]). Callers keep the stored
 * country when this is null.
 */
export function requestCountry(request: Request): string | null {
  const value = request.headers.get(COUNTRY_HEADER);
  return value !== null && isCountryCode(value) ? value : null;
}

/** A → the first Unicode regional-indicator letter; two of them draw a flag. */
const REGIONAL_INDICATOR_A = 0x1f1e6;

/**
 * A country code as its flag emoji (`US` → 🇺🇸). It only converts: the code
 * must already be one [`isCountryCode`] accepts.
 */
export function countryFlag(code: string): string {
  return String.fromCodePoint(
    ...[...code].map((letter) => REGIONAL_INDICATOR_A + letter.charCodeAt(0) - 65),
  );
}
