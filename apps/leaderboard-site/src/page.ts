// The public HTML page. It shows exactly what GET /api/leaderboard returns.
import { LISTED_WITHIN_HOURS, type ListedRow } from './listing.js';

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

/** The medal drawn before the top three ranks; the number stays. */
const MEDALS = ['🥇', '🥈', '🥉'];

/**
 * How full a row's bar is: its number against the top number, in whole
 * percent, at least 2 so a small number still shows a sliver.
 */
export function barPercent(tenths: number, topTenths: number): number {
  if (topTenths <= 0) return 2;
  return Math.max(2, Math.round((tenths * 100) / topTenths));
}

const STYLE = `
:root {
  color-scheme: light dark;
  --bg: #fafafb;
  --panel: #ffffff;
  --line: #e7e7ec;
  --fg: #17171c;
  --muted: #62636d;
  --faint: #8b8c96;
  --accent: #6d5ce8;
  --accent-soft: rgba(109, 92, 232, 0.10);
  --gold: rgba(232, 176, 36, 0.14);
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0e0e11;
    --panel: #16161b;
    --line: #26262e;
    --fg: #ececf1;
    --muted: #a0a1ab;
    --faint: #6f707b;
    --accent: #9384ff;
    --accent-soft: rgba(147, 132, 255, 0.12);
    --gold: rgba(245, 196, 81, 0.10);
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 15px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
}
a { color: inherit; }
.wrap { max-width: 46rem; margin: 0 auto; padding: 0 1rem; }
.top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 1.25rem 0;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.55rem;
  font-weight: 650;
  letter-spacing: -0.01em;
  text-decoration: none;
}
.brand svg { display: block; }
.brand .tag {
  margin-left: 0.2rem;
  padding: 0.1rem 0.5rem;
  border: 1px solid var(--line);
  border-radius: 999px;
  color: var(--muted);
  font-size: 0.75rem;
  font-weight: 500;
}
.get {
  padding: 0.4rem 0.8rem;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  font-size: 0.85rem;
  font-weight: 550;
  text-decoration: none;
}
.get:hover { border-color: var(--accent); }
.hero { padding: 2.5rem 0 1.75rem; }
.eyebrow {
  margin: 0 0 0.75rem;
  color: var(--accent);
  font: 600 0.72rem/1 ui-monospace, "SF Mono", Menlo, monospace;
  letter-spacing: 0.12em;
  text-transform: uppercase;
}
h1 {
  margin: 0 0 0.75rem;
  font-size: clamp(2rem, 6vw, 2.75rem);
  line-height: 1.05;
  letter-spacing: -0.03em;
}
.lede { max-width: 34rem; margin: 0; color: var(--muted); font-size: 1rem; }
.stats {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75rem;
  max-width: 24rem;
  margin: 1.75rem 0 0;
}
.stats div {
  padding: 0.8rem 1rem;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--panel);
}
.stats dt {
  color: var(--faint);
  font: 500 0.68rem/1 ui-monospace, "SF Mono", Menlo, monospace;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}
.stats dd {
  margin: 0.45rem 0 0;
  font-size: 1.5rem;
  font-weight: 650;
  letter-spacing: -0.02em;
  font-variant-numeric: tabular-nums;
}
.board {
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: var(--panel);
}
table { width: 100%; border-collapse: collapse; }
th {
  padding: 0.75rem 1rem 0.6rem;
  border-bottom: 1px solid var(--line);
  color: var(--faint);
  font: 500 0.68rem/1 ui-monospace, "SF Mono", Menlo, monospace;
  letter-spacing: 0.1em;
  text-align: left;
  text-transform: uppercase;
  white-space: nowrap;
}
td { padding: 0.8rem 1rem; border-bottom: 1px solid var(--line); vertical-align: middle; }
tbody tr:last-child td { border-bottom: 0; }
tbody tr:hover td { background: var(--accent-soft); }
tr[data-rank="1"] td { background: var(--gold); }
.rank {
  width: 4.5rem;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.medal { margin-right: 0.35rem; }
.name { font-weight: 550; overflow-wrap: anywhere; }
th:last-child { text-align: right; }
.lev { width: 52%; }
.levrow { display: flex; align-items: center; gap: 0.9rem; }
.track {
  flex: 1;
  height: 0.5rem;
  overflow: hidden;
  border-radius: 999px;
  background: var(--accent-soft);
}
.fill { display: block; height: 100%; border-radius: 999px; background: var(--accent); opacity: 0.85; }
tr[data-rank="1"] .fill { background: linear-gradient(90deg, var(--accent), #e8b024); opacity: 1; }
.value {
  min-width: 4.5rem;
  font-weight: 650;
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.empty { margin: 0; padding: 2.5rem 1.25rem; text-align: center; color: var(--muted); }
.empty strong { display: block; margin-bottom: 0.35rem; color: var(--fg); font-size: 1.05rem; }
.foot {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 1.25rem;
  padding: 2rem 0 3rem;
  color: var(--muted);
  font-size: 0.85rem;
}
.foot h2 { margin: 0 0 0.3rem; color: var(--fg); font-size: 0.85rem; font-weight: 600; }
.foot p { margin: 0; }
@media (max-width: 40rem) {
  .hero { padding-top: 1.5rem; }
  .foot { grid-template-columns: 1fr; gap: 1rem; }
  th, td { padding-left: 0.75rem; padding-right: 0.75rem; }
  .rank { width: 3.75rem; }
  .lev { width: 44%; }
  .levrow { gap: 0.6rem; }
  .value { min-width: 3.75rem; }
}
`;

const LOGO = `<svg width="22" height="22" viewBox="0 0 22 22" aria-hidden="true"><rect x="1" y="1" width="9" height="9" rx="2" fill="#8b7cf6"/><rect x="12" y="1" width="9" height="9" rx="2" fill="#c4bbff"/><rect x="1" y="12" width="9" height="9" rx="2" fill="#c4bbff"/><rect x="12" y="12" width="9" height="9" rx="2" fill="#6d5ce8"/></svg>`;

function boardHtml(rows: ListedRow[]): string {
  if (rows.length === 0)
    return '<div class="board"><p class="empty"><strong>No one is listed yet.</strong>Be the first: open the XTrace app and choose Leaderboard.</p></div>';
  const top = rows[0]?.leverage_tenths ?? 0;
  return [
    '<div class="board">',
    '<table>',
    '<thead><tr><th scope="col">Rank</th><th scope="col">Nickname</th><th scope="col">Leverage, 7 days</th></tr></thead>',
    '<tbody>',
    ...rows.map((row) => {
      const medal = MEDALS[row.rank - 1];
      const rank = medal
        ? `<span class="medal" aria-hidden="true">${medal}</span>${row.rank}`
        : `${row.rank}`;
      return (
        `<tr data-rank="${row.rank}"><td class="rank">${rank}</td>` +
        `<td class="name">${escapeHtml(row.nickname)}</td>` +
        `<td class="lev"><div class="levrow"><span class="track" aria-hidden="true">` +
        `<span class="fill" style="width:${barPercent(row.leverage_tenths, top)}%"></span></span>` +
        `<span class="value">${formatLeverage(row.leverage_tenths)}</span></div></td></tr>`
      );
    }),
    '</tbody>',
    '</table>',
    '</div>',
  ].join('\n');
}

export function renderPage(rows: ListedRow[]): string {
  const top = rows[0];
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>XTrace Leaderboard</title>
<meta name="description" content="Leverage over the last 7 days, shared by people who opted in from the XTrace app.">
<style>${STYLE}</style>
</head>
<body>
<div class="wrap">
<header class="top">
<a class="brand" href="https://xtrace.ai">${LOGO}XTrace<span class="tag">Leaderboard</span></a>
<a class="get" href="https://xtrace.ai">Get XTrace</a>
</header>
<main>
<section class="hero">
<p class="eyebrow">Last 7 days · updates every minute</p>
<h1>XTrace Leaderboard</h1>
<p class="lede">Leverage over the last 7 days: agent hours divided by your own hours. Shared by people who opted in from the XTrace app.</p>
<dl class="stats">
<div><dt>Members</dt><dd>${rows.length}</dd></div>
<div><dt>Top leverage</dt><dd>${top ? formatLeverage(top.leverage_tenths) : '—'}</dd></div>
</dl>
</section>
${boardHtml(rows)}
</main>
<footer class="foot">
<div><h2>How to join</h2><p>Open the XTrace app, choose Leaderboard and pick a nickname.</p></div>
<div><h2>What is public</h2><p>Only your nickname and number. Emails are never shown.</p></div>
<div><h2>Who is listed</h2><p>Members whose app sent a number in the last ${LISTED_WITHIN_HOURS} hours.</p></div>
</footer>
</div>
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
