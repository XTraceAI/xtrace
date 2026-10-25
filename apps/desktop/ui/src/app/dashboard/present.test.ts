import { expect, it } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import { delta, percent } from '../../kit/format';
import {
  fromPoints,
  outputShareRows,
  ruleId,
  shareText,
  tileDelta,
  usd,
  windowLabel,
  zoneLabel,
} from './present';

const report = (fixture as FixtureExport).dashboards[0];

it('converts report percentage points to formatter fractions exactly once', () => {
  expect(percent(fromPoints(50))).toBe('50%');
  expect(percent(fromPoints(0))).toBe('0%');
  expect(percent(fromPoints(66.7))).toBe('66.7%');
  expect(fromPoints(null)).toBeNull();
  expect(fromPoints(Number.NaN)).toBeNull();
  const tile = { ...report.tiles.agent_hours, delta: { previous: 1, pct: 50, suppressed: false } };
  expect(delta(tileDelta(tile))).toBe('▲50%');
  expect(tileDelta({ ...tile, delta: { ...tile.delta, suppressed: true } })).toBeUndefined();
});

it('formats API-equivalent dollars: cents below $100, whole dollars from $100', () => {
  expect(usd(0)).toBe('$0.00');
  expect(usd(-0)).toBe('$0.00');
  expect(usd(0.001)).toBe('<$0.01');
  expect(usd(12.4)).toBe('$12.40');
  expect(usd(99.99)).toBe('$99.99');
  // A value that would round to $100.00 is already whole dollars.
  expect(usd(99.995)).toBe('$100');
  expect(usd(250.4)).toBe('$250');
  expect(usd(1234.567)).toBe('$1,235');
});

it('falls back to a known rule definition and system zone labels', () => {
  expect(ruleId('M-09', 'M-05')).toBe('M-09');
  expect(ruleId('not-a-rule', 'M-05')).toBe('M-05');
  const local = { ...report.window, timezone: 'system-local' };
  expect(zoneLabel(local)).toBe('system time zone');
  expect(windowLabel(local)).toMatch(/2026/);
  expect(windowLabel({ ...report.window, timezone: 'Not/AZone' })).toMatch(/2026/);
});

it('names each output share row and gives its share of the rows, as every share is written', () => {
  expect(
    outputShareRows([
      { of: { kind: 'model', model: 'synthetic-a' }, output_tokens: 995 },
      { of: { kind: 'unknown_model' }, output_tokens: 4 },
      { of: { kind: 'model', model: 'synthetic-b' }, output_tokens: 1 },
    ]),
  ).toEqual([
    { label: 'synthetic-a', value: '100%' },
    { label: 'no model recorded', value: '<1%' },
    { label: 'synthetic-b', value: '<1%' },
  ]);
  // A folded row is named by its count and shares the same sum.
  expect(
    outputShareRows([
      { of: { kind: 'model', model: 'synthetic-a' }, output_tokens: 60 },
      { of: { kind: 'other_models', models: 2 }, output_tokens: 40 },
    ]),
  ).toEqual([
    { label: 'synthetic-a', value: '60%' },
    { label: '2 other models', value: '40%' },
  ]);
  expect(outputShareRows([])).toEqual([]);
  expect(shareText(1, 200)).toBe('1%');
  expect(shareText(0, 10)).toBe('0%');
  expect(shareText(1, 0)).toBe('—');
});
