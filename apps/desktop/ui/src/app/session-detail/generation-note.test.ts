import { expect, it } from 'vitest';
import { generationNote } from './transcript-adapter';

it('reuses the exact existing generation wording for full reads and excerpts', () => {
  expect(
    generationNote({ state: 'loaded', generation: { generation: 'indexed', appended: false } }),
  ).toBeNull();
  expect(
    generationNote({ state: 'loaded', generation: { generation: 'indexed', appended: true } }),
  ).toBe(
    'This session has been written to since it was last indexed. The measurements beside it describe the part that was indexed.',
  );
  expect(generationNote({ state: 'loaded', generation: { generation: 'unrecorded' } })).toBe(
    'This text is not tied to a measurement: at least one of this session’s files has no recorded reading.',
  );
  expect(generationNote({ state: 'unavailable' })).toBeNull();
});
