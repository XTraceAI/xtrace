import type { LeaderboardStanding } from '../../data/generated/LeaderboardStanding';
import { leverageText } from '../metric-format';

/** What the page says before anyone joins, word for word. */
export const JOIN_TEXT =
  'Join the public XTrace leaderboard. It shows your nickname and your leverage for the last 7 days at leaderboard.xtrace.ai. We also keep your email so we can reach you; it is never shown. Nothing else is sent.';

/** The confirmation Leave asks for. */
export const LEAVE_QUESTION = 'Leave the leaderboard?';
export const LEAVE_TEXT =
  'This removes you from the public page and deletes your nickname and email from it.';

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

/** A refused change's words: the native side rejects with plain text. */
export const refusalText = (error: unknown): string =>
  typeof error === 'string'
    ? error
    : error instanceof Error && error.message
      ? error.message
      : 'The leaderboard could not be reached.';
