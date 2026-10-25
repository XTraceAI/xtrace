import type { Page } from '@playwright/test';

/** Let layout, resize/intersection observers, and their next paint finish. */
export const afterRendering = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))),
  );
