// The one country rule. A member's country is only ever read from the header
// Vercel adds from the caller's IP address; the IP itself is never stored.

/** The header Vercel sets to the two-letter country of the caller's IP address. */
export const COUNTRY_HEADER = 'x-vercel-ip-country';

const COUNTRY_CODE = /^[A-Z]{2}$/;

/**
 * The caller's country as two uppercase ASCII letters, or null when it is not
 * known: no header, or a value that is not exactly two uppercase letters, or
 * `XX` (no country). Callers keep the stored country when this is null.
 */
export function requestCountry(request: Request): string | null {
  const value = request.headers.get(COUNTRY_HEADER);
  if (value === null || !COUNTRY_CODE.test(value) || value === 'XX') return null;
  return value;
}

/** A–Z → the matching Unicode regional-indicator letter (two of them draw a flag). */
const REGIONAL_INDICATOR_A = 0x1f1e6;

/**
 * A two-letter code as its flag emoji (`US` → 🇺🇸), or '' when it is not
 * exactly two uppercase ASCII letters.
 */
export function countryFlag(code: string): string {
  if (!COUNTRY_CODE.test(code)) return '';
  return String.fromCodePoint(
    ...[...code].map((letter) => REGIONAL_INDICATOR_A + letter.charCodeAt(0) - 65),
  );
}
