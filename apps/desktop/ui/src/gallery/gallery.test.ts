import { readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { expect, it } from 'vitest';
import * as kit from '../kit';
import { stories } from './stories';

const ui = resolve(import.meta.dirname, '../..');
it('covers every exported kit component with a unique declared story and matching barrel', async () => {
  const components = Object.keys(kit).sort();
  expect([...new Set(stories.flatMap((story) => story.components))].sort()).toEqual(
    components.sort(),
  );
  expect(new Set(stories.map(({ id }) => id)).size).toBe(stories.length);
  for (const { id, size } of stories) {
    expect(id).toMatch(/^[a-z0-9-]+\/[a-z0-9-]+$/);
    expect(size.every((value) => Number.isInteger(value) && value > 0)).toBe(true);
  }
});
it('keeps the generated review inventory in sync with the story registry', async () => {
  const inventory = stories.map(({ id, components, size }) => ({ id, components, size }));
  const path = join(ui, 'src/gallery/inventory.json');
  if (process.env.UPDATE_GALLERY_INVENTORY === '1')
    await writeFile(path, JSON.stringify(inventory, null, 2) + '\n');
  expect(JSON.parse(await readFile(path, 'utf8'))).toEqual(inventory);
});
