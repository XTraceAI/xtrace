import type { Page } from '@playwright/test';

/** Let layout, resize/intersection observers, and their next paint finish. */
export const afterRendering = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))),
  );

/** Run SplitHandle's documented 150 ms deferred bounds probe and its queued frame.
 * The caller installs the browser clock before navigation.
 */
export const afterSplitSettles = async (page: Page) => {
  await afterRendering(page);
  await page.clock.runFor(150);
  await afterRendering(page);
};
