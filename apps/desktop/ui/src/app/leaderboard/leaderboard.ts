import type { LeaderboardField } from '../../data/generated/LeaderboardField';
import type { LeaderboardRow } from '../../data/generated/LeaderboardRow';
import type { LeaderboardStanding } from '../../data/generated/LeaderboardStanding';
import { leverageText } from '../metric-format';

/** The one line the join card shows before anyone joins, word for word. */
export const JOIN_TEXT =
  "🏆 See where your 7-day leverage ranks. Pick a nickname, and we email you a code to confirm it's you.";
/** Said next to the email field: who sees the email, and what it is for. */
export const EMAIL_PRIVATE =
  '🔒 Your email stays private: other users never see it. We only use it to send your code, which keeps out spam and fake entries.';

/** The title of the card that unlocks the page. */
export const JOIN_TITLE = 'Join the leaderboard';

/** The join card's disclosure: its label, then the rest of what joining shares. */
export const SHARED_SUMMARY = 'What is shared';
export const SHARED_FACTS: readonly string[] = [
  'Public: your nickname and 7-day leverage.',
  'Private: your email, used only for sign-in codes.',
  'The same email on a new Mac signs you back in; only your latest Mac sends.',
  'The site also sees your IP address and app version.',
  'Nothing is sent or read until you join. The blurred list is an example.',
];

/** The joined page's public list: its title and what it says when it has no rows. */
export const TABLE_TITLE = 'Public leaderboard';
export const TABLE_EMPTY = 'No one is listed yet.';
/** Said under the list: the leaderboard serves it from a short-lived cache. */
export const TABLE_UPDATES = 'The public list updates every minute or two.';
/** Said when the app could not answer at all (the native call itself failed). */
export const TABLE_UNREADABLE = 'The public list could not be read.';
/** Said while the list is being read. */
export const TABLE_READING = 'Reading the public list…';
/** The medal drawn before the top three ranks, for fun; the number stays. */
export const rankMedal = (rank: number): string | null => ['🥇', '🥈', '🥉'][rank - 1] ?? null;
/** Said to assistive technology after the member's own nickname. */
export const YOU_LABEL = '(this is you)';

/**
 * Made-up rows drawn blurred behind the join card, so the locked page looks
 * like the list it unlocks. Decoration only, not a product decision: the app
 * reads nothing from the leaderboard before joining. The `sample-` nicknames
 * and numbers no real view uses keep them from passing for real data, and the
 * page hides them from assistive technology.
 */
const SAMPLE_NAMES = [
  'otter',
  'fern',
  'quartz',
  'you',
  'maple',
  'kite',
  'harbor',
  'dune',
  'cedar',
  'lynx',
  'ember',
  'moss',
  'heron',
  'slate',
  'birch',
  'comet',
  'reed',
  'flint',
  'aspen',
  'wren',
  'cobalt',
  'tide',
  'juniper',
  'orbit',
  'sable',
  'thistle',
  'pike',
  'loam',
  'gale',
  'onyx',
  'willow',
  'ridge',
  'plover',
  'basalt',
  'larch',
  'meadow',
];
/** Enough rows to run past the bottom of a tall window; the frame clips the rest. */
export const SAMPLE_ROWS: readonly LeaderboardRow[] = SAMPLE_NAMES.map((name, index) => ({
  rank: index + 1,
  nickname: `sample-${name}`,
  leverage_tenths: Math.round(482 * 0.9 ** index),
  is_you: name === 'you',
}));

/** The code step's line, naming where the code went. */
export const codeSentText = (email: string) =>
  `We sent a 6-digit code to ${email}. Enter it below. It expires in 15 minutes.`;

/** When the number is sent, in the order the app sends it. */
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
      return `#${standing.rank}`;
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
