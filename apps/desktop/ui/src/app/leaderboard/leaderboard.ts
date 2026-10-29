import type { LeaderboardField } from '../../data/generated/LeaderboardField';
import type { LeaderboardRow } from '../../data/generated/LeaderboardRow';
import type { LeaderboardStanding } from '../../data/generated/LeaderboardStanding';
import { leverageText } from '../metric-format';

/** The one line the join card shows before anyone joins, word for word. */
export const JOIN_TEXT =
  "See where your 7-day leverage ranks. Pick a nickname; we email you a code to confirm it's you. Your email is never shown.";

/** The title of the card that unlocks the page. */
export const JOIN_TITLE = 'Join the leaderboard';

/** The join card's disclosure: its label, then the rest of what joining shares. */
export const SHARED_SUMMARY = 'What is shared';
export const SHARED_FACTS: readonly string[] = [
  'The public page, leaderboard.xtrace.ai, shows your nickname and your leverage for the last 7 days.',
  'We keep your email so we can reach you. It is never shown.',
  'Joining again with the same email, for example on a new Mac, signs you back in to your entry. Only the Mac you joined from most recently sends your number.',
  "Like any web request, the leaderboard also sees your IP address and this app's version.",
  'Until you join, the app sends and reads nothing. The blurred list behind this card is a made-up example.',
];

/** The joined page's public list: its title and what it says when it has no rows. */
export const TABLE_TITLE = 'Public leaderboard';
export const TABLE_EMPTY = 'No one is listed yet.';
/** Said when the app could not answer at all (the native call itself failed). */
export const TABLE_UNREADABLE = 'The public list could not be read.';
/** Said while the list is being read. */
export const TABLE_READING = 'Reading the public list…';
/** Said to assistive technology after the member's own nickname. */
export const YOU_LABEL = '(this is you)';

/**
 * Made-up rows drawn blurred behind the join card, so the locked page looks
 * like the list it unlocks. Decoration only, not a product decision: the app
 * reads nothing from the leaderboard before joining. The `sample-` nicknames
 * and numbers no real view uses keep them from passing for real data, and the
 * page hides them from assistive technology.
 */
export const SAMPLE_ROWS: readonly LeaderboardRow[] = [
  { rank: 1, nickname: 'sample-otter', leverage_tenths: 482, is_you: false },
  { rank: 2, nickname: 'sample-fern', leverage_tenths: 317, is_you: false },
  { rank: 3, nickname: 'sample-quartz', leverage_tenths: 274, is_you: false },
  { rank: 4, nickname: 'sample-you', leverage_tenths: 199, is_you: true },
  { rank: 5, nickname: 'sample-maple', leverage_tenths: 141, is_you: false },
  { rank: 6, nickname: 'sample-kite', leverage_tenths: 96, is_you: false },
  { rank: 7, nickname: 'sample-harbor', leverage_tenths: 63, is_you: false },
  { rank: 8, nickname: 'sample-dune', leverage_tenths: 28, is_you: false },
];

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
