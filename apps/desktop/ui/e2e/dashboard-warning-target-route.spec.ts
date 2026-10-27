import { createHash } from 'node:crypto';
import { expect, test } from '@playwright/test';
import type { SessionPageSelection } from '../src/data/generated/SessionPageSelection';

test.use({ timezoneId: 'UTC' });
const productionReact = process.env.NODE_ENV === 'production';
const cases = [
  { id: 'correction:w-h3', uuid: 'w-h3', block: null, text: 'Exact Dashboard correction message.' },
  {
    id: 'repeated_failure:w-r1#0',
    uuid: 'w-r2',
    block: 0,
    text: 'Exact Dashboard original failed result.',
  },
] as const;
for (const host of ['claude', 'codex'] as const)
  for (const theme of ['light', 'dark'] as const)
    for (const target of cases)
      test(`Dashboard to exact ${target.block === null ? 'message' : 'result'} in ${host}, ${theme}`, async ({
        page,
      }, info) => {
        await page.setViewportSize({ width: 1440, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        const path = `/dashboard?range=7d&host=${host}&with_prs=1`;
        const runtimeResponse = page.waitForResponse((response) =>
          new URL(response.url()).pathname.endsWith('/react-dom_client.js'),
        );
        await page.goto(
          `/e2e/dashboard-warning.html?full-session-targets&host=${host}&path=${encodeURIComponent(path)}`,
        );
        const response = await runtimeResponse;
        const runtime = await response.text();
        const selectedExport = productionReact
          ? 'module.exports = require_react_dom_client_production();'
          : 'module.exports = require_react_dom_client_development();';
        expect(runtime.split('//# sourceMappingURL=')[0]).toContain(selectedExport);
        await info.attach('served-react-runtime', {
          body: JSON.stringify({
            url: response.url(),
            bytes: Buffer.byteLength(runtime),
            sha256: createHash('sha256').update(runtime).digest('hex'),
            selectedExport,
          }),
          contentType: 'application/json',
        });
        const dot = page.locator(`[data-testid="warning-dot"][data-warning-id="${target.id}"]`);
        await dot.focus();
        await page.keyboard.press('Enter');
        const modal = page.getByTestId('dashboard-warning-modal');
        await expect(modal.getByTestId('warning-context')).toContainText(target.text);
        const link = modal.getByRole('link', { name: 'View in full session' });
        const href = new URL((await link.getAttribute('href'))!, 'http://fixture');
        expect(href.pathname).toBe('/sessions/s-warned');
        expect(Object.fromEntries(href.searchParams)).toEqual({
          range: '7d',
          host,
          with_prs: '1',
          q: 's-warned',
          warning: target.id,
        });
        await link.click();
        await expect(modal).toHaveCount(0);
        await expect
          .poll(() =>
            page.evaluate(() => (window as unknown as { __warningRoute: string }).__warningRoute),
          )
          .toBe(href.pathname + href.search);
        const selected = page.getByRole('list', { name: 'Selected warning' });
        await expect(selected.getByTestId('warning-item')).toBeFocused();
        const conversation = page.getByRole('region', { name: 'Conversation content' });
        const original =
          target.block === null
            ? conversation.locator(`[data-record-id="${target.uuid}"]`)
            : conversation.locator(`[data-block-id="${target.uuid}:${target.block}"]`);
        await expect(original).toHaveAttribute('data-highlighted', 'true');
        await expect(original).toBeInViewport();
        await expect(original).toContainText(target.text);
        const reads = await page.evaluate(
          () =>
            (
              window as unknown as {
                __fullSessionReads: {
                  pages: SessionPageSelection[];
                  pageCalls: { id: string; selection: SessionPageSelection }[];
                  cancelled: string[];
                  contexts: { recordUuid: string; block: number | null }[];
                  full: number;
                };
              }
            ).__fullSessionReads,
        );
        await info.attach('source-reads', {
          body: JSON.stringify(reads),
          contentType: 'application/json',
        });
        const exactAnchor = {
          kind: 'anchor',
          expected: target.block === null ? 'message' : 'tool_block',
          record_uuid: target.uuid,
          block: target.block,
          generation: null,
        };
        // Development StrictMode replays mount effects; its first owner must be canceled.
        expect(reads.pages).toEqual(productionReact ? [exactAnchor] : [exactAnchor, exactAnchor]);
        expect(reads.pageCalls.map((call) => call.selection)).toEqual(reads.pages);
        expect(new Set(reads.pageCalls.map((call) => call.id)).size).toBe(reads.pages.length);
        if (!productionReact) expect(reads.cancelled).toContain(reads.pageCalls[0]!.id);
        const retained = reads.pageCalls.at(-1)!;
        expect(reads.cancelled).not.toContain(retained.id);
        expect(reads.pageCalls.filter((call) => !reads.cancelled.includes(call.id))).toEqual([
          retained,
        ]);
        const exactContext = { recordUuid: target.uuid, block: target.block };
        expect(reads.contexts).toEqual(
          productionReact ? [exactContext] : [exactContext, exactContext],
        );
        expect(reads.full).toBe(productionReact ? 1 : 2);
        await page.screenshot({
          path: info.outputPath(
            `${host}-${theme}-${target.block === null ? 'message' : 'result'}-route.png`,
          ),
          fullPage: true,
        });
      });
