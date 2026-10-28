import type { LeaderboardField } from '../../data/generated/LeaderboardField';
import type { LeaderboardStanding } from '../../data/generated/LeaderboardStanding';
import { leverageText } from '../metric-format';

/** What the page says before anyone joins, word for word. */
export const JOIN_TEXT =
  "Join the public XTrace leaderboard. It shows your nickname and your leverage for the last 7 days at leaderboard.xtrace.ai. We email you a 6-digit code to check the email is yours, and keep the email so we can reach you; it is never shown. Joining again with the same email, for example on a new Mac, signs you back in to your entry; only the Mac you joined from most recently sends your number. Like any web request, the leaderboard also sees your IP address and this app's version.";

/** The code step's line, naming where the code went. */
export const codeSentText = (email: string) =>
  `We sent a 6-digit code to ${email}. Enter it below. It expires in 15 minutes.`;

/** When the number is sent, in the order the app sends it. */
export const SEND_TIMES =
  'The app sends your number when it starts, when it first finishes reading your history, and every hour while it runs.';

/** The confirmation Leave asks for. */
export const LEAVE_QUESTION = 'Leave the leaderboard?';
export const LEAVE_TEXT =
  'This removes you from the public page and deletes your nickname and email from it.';
/** Said with the confirmation: deletion is not instant everywhere. */
export const LEAVE_BACKUPS =
  "The leaderboard's database host can keep a copy in its backups for a limited time before it is gone.";

/**
 * The number the leaderboard holds (tenths, as the app sent it), written the
 * way the Dashboard writes leverage: `12.3×`. The app sends exactly the tenths
 * the Dashboard writes, so this reads the same as the Dashboard's 7-day tile.
 */
export const leaderboardNumber = (tenths: number | null): string =>
  tenths === null ? 'No 7-day number' : leverageText(tenths / 10);

/** The rank line, from the standing the app reports. */
export function standingText(standing: LeaderboardStanding): string {
  switch (standing.state) {
    case 'ranked':
      return `#${standing.rank} on the public page`;
    case 'not_listed':
      return standing.reason;
    case 'unknown':
      return 'Unknown: the leaderboard could not be asked.';
  }
}

/** A refused change's words: the native side rejects with a `LeaderboardRefusal`. */
export const refusalText = (error: unknown): string =>
  typeof error === 'string'
    ? error
    : typeof error === 'object' &&
        error !== null &&
        'message' in error &&
        typeof error.message === 'string' &&
        error.message
      ? error.message
      : 'The leaderboard could not be reached.';

/** The form field a refusal names, if any. */
export const refusalField = (error: unknown): LeaderboardField | null =>
  typeof error === 'object' &&
  error !== null &&
  'field' in error &&
  (error.field === 'nickname' || error.field === 'email' || error.field === 'code')
    ? error.field
    : null;
