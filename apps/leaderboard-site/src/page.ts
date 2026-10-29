// The public HTML page. It shows exactly what GET /api/leaderboard returns.
import type { ListedRow } from './listing.js';

export const PAGE_CACHE_CONTROL = 'public, s-maxage=60, stale-while-revalidate=60';

const ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (character) => ESCAPES[character] ?? character);
}

/** Tenths as `12.3×`: always one decimal, computed without floating point. */
export function formatLeverage(tenths: number): string {
  return `${Math.floor(tenths / 10)}.${tenths % 10}×`;
}

const STYLE = `
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --fg: #1b1b1f;
  --muted: #5d5f66;
  --line: #e4e4e8;
}
@media (prefers-color-scheme: dark) {
  :root { --bg: #131316; --fg: #ececf0; --muted: #a3a5ad; --line: #2b2b31; }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
main { max-width: 40rem; margin: 0 auto; padding: 2.5rem 1rem 3rem; }
h1 { font-size: 1.75rem; line-height: 1.2; margin: 0 0 0.5rem; }
.lede { margin: 0 0 1.75rem; color: var(--muted); }
table { width: 100%; border-collapse: collapse; }
th, td { padding: 0.6rem 0.25rem; border-bottom: 1px solid var(--line); text-align: left; }
th { font-size: 0.85rem; font-weight: 600; color: var(--muted); }
td:first-child { overflow-wrap: anywhere; }
th:last-child, td:last-child {
  text-align: right;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.empty { margin: 0; padding: 1.5rem 0; color: var(--muted); border-top: 1px solid var(--line); }
`;

export function renderPage(rows: ListedRow[]): string {
  const body =
    rows.length === 0
      ? '<p class="empty">No one is listed yet.</p>'
      : [
          '<table>',
          '<thead><tr><th scope="col">Nickname</th><th scope="col">Leverage (7 days)</th></tr></thead>',
          '<tbody>',
          ...rows.map(
            (row) =>
              `<tr><td>${escapeHtml(row.nickname)}</td><td>${formatLeverage(row.leverage_tenths)}</td></tr>`,
          ),
          '</tbody>',
          '</table>',
        ].join('\n');
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>XTrace Leaderboard</title>
<style>${STYLE}</style>
</head>
<body>
<main>
<h1>XTrace Leaderboard</h1>
<p class="lede">Leverage over the last 7 days: agent hours divided by your own hours. Shared by people who opted in from the XTrace app.</p>
${body}
</main>
</body>
</html>
`;
}

export function pageResponse(rows: ListedRow[]): Response {
  return new Response(renderPage(rows), {
    status: 200,
    headers: {
      'content-type': 'text/html; charset=utf-8',
      'cache-control': PAGE_CACHE_CONTROL,
      'x-content-type-options': 'nosniff',
      'content-security-policy':
        "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'",
    },
  });
}
