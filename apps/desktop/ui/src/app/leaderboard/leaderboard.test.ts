import { expect, it } from 'vitest';
import { leverageText } from '../metric-format';
import { leaderboardNumber, refusalText, standingText } from './leaderboard';

// The tenths Rust sends for these 7-day leverage values
// (`written_tenths_round_the_stored_value_half_up` in dashboard.rs). The
// leaderboard's number, written back, must read exactly as the Dashboard
// writes the value itself.
const PAIRS: [number, number][] = [
  [0.05, 1],
  [0.15, 2],
  [0.25, 3],
  [0.95, 10],
  [1, 10],
  [1.049_999_999_999_999_8, 10],
  [1.05, 11],
  [1.25, 13],
  [2.675, 27],
  [12.34, 123],
  [12.35, 124],
];

it('writes the number the app sent exactly as the Dashboard writes the leverage', () => {
  for (const [value, tenths] of PAIRS) {
    expect(leaderboardNumber(tenths), String(value)).toBe(leverageText(value));
  }
  expect(leaderboardNumber(123)).toBe('12.3×');
  expect(leaderboardNumber(0)).toBe('0×');
  expect(leaderboardNumber(null)).toBe('No 7-day number');
});

it('states each standing in words', () => {
  expect(standingText({ state: 'ranked', rank: 4 })).toBe('#4 on the public page');
  expect(
    standingText({
      state: 'not_listed',
      reason: 'Not listed yet: there is no leverage number for your last 7 days.',
    }),
  ).toBe('Not listed yet: there is no leverage number for your last 7 days.');
  expect(standingText({ state: 'unknown' })).toBe('Unknown: the leaderboard could not be asked.');
});

it('shows the native answer as given', () => {
  expect(refusalText('That nickname is taken. Try another one.')).toBe(
    'That nickname is taken. Try another one.',
  );
  expect(refusalText(new Error('No connection'))).toBe('No connection');
  expect(refusalText(undefined)).toBe('The leaderboard could not be reached.');
});
