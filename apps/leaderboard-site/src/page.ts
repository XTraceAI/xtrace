// The public HTML page. It shows exactly what GET /api/leaderboard returns.
// It is drawn as an arcade high-score table with HTML, CSS and inline SVG only:
// the page runs no scripts.
import type { ListedRow } from './listing.js';

export const PAGE_CACHE_CONTROL = 'public, s-maxage=60, stale-while-revalidate=60';

/** The page's own font, served from public/fonts (unmodified Press Start 2P, OFL 1.1). */
export const PAGE_FONT_PATH = '/fonts/PressStart2P-Regular.ttf';

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

/**
 * Tenths as `12.3X`: always one decimal, computed without floating point.
 * A plain X, because the page's pixel font draws it like the digits.
 */
export function formatLeverage(tenths: number): string {
  return `${Math.floor(tenths / 10)}.${tenths % 10}X`;
}

/** Shown where a number would be when there is none, in the same shape as formatLeverage. */
const NO_LEVERAGE = '-.-X';

/** A rank as an arcade ordinal: 1ST, 2ND, 3RD, 4TH, 11TH, 12TH, 13TH, 21ST, 111TH. */
export function ordinal(rank: number): string {
  const lastTwo = rank % 100;
  const last = rank % 10;
  const suffix =
    lastTwo >= 11 && lastTwo <= 13
      ? 'TH'
      : last === 1
        ? 'ST'
        : last === 2
          ? 'ND'
          : last === 3
            ? 'RD'
            : 'TH';
  return `${rank}${suffix}`;
}

/** The board always shows at least this many rows; missing ones are open places. */
const BOARD_ROWS = 8;

/** Below this many tenths (1.0X) a row is dimmed. */
const DIM_BELOW_TENTHS = 10;

/** The row's color: the top three ranks have their own, anyone below 1.0X is dimmed. */
function tone(row: ListedRow): string {
  if (row.leverage_tenths < DIM_BELOW_TENTHS) return 'dim';
  if (row.rank === 1) return 'gold';
  if (row.rank === 2) return 'orange';
  if (row.rank === 3) return 'pink';
  return 'cyan';
}

// The rocket drawn beside the top three ranks, one character per pixel:
// X takes the row's color, W is white and R is the flame.
const ROCKET = [
  '...X...',
  '..XXX..',
  '..XWX..',
  '..XXX..',
  '.XXXXX.',
  'XX.X.XX',
  'X..X..X',
  '..R.R..',
  '...R...',
];
const ROCKET_FILLS: Record<string, string> = {
  X: 'currentColor',
  W: '#f2f2ff',
  R: '#ff6a3d',
};

function rocketRects(): string {
  const rects: string[] = [];
  ROCKET.forEach((line, y) => {
    let x = 0;
    while (x < line.length) {
      const pixel = line[x] ?? '.';
      let end = x + 1;
      while (line[end] === pixel) end += 1;
      const fill = ROCKET_FILLS[pixel];
      if (fill)
        rects.push(`<rect x="${x}" y="${y}" width="${end - x}" height="1" fill="${fill}"/>`);
      x = end;
    }
  });
  return rects.join('');
}

const ROCKET_RECTS = rocketRects();

function rocket(pixelSize: number): string {
  return (
    `<svg class="rocket" width="${7 * pixelSize}" height="${9 * pixelSize}" viewBox="0 0 7 9" ` +
    `shape-rendering="crispEdges" aria-hidden="true" focusable="false">${ROCKET_RECTS}</svg>`
  );
}

// The XTrace mark (the app's sidebar logo) as inline SVG.
const LOGO =
  '<svg width="22" height="22" viewBox="0 0 3 3" aria-hidden="true" focusable="false">' +
  '<defs><linearGradient id="xg" x1="1" y1="0" x2="0" y2="1">' +
  '<stop offset="0" stop-color="#7F8CFF"/><stop offset="1" stop-color="#A35CFF"/></linearGradient>' +
  '<mask id="xm"><rect width="3" height="3" fill="#fff"/>' +
  '<path d="M1 1V0A1 1 0 0 1 2 1Z M1 1H0A1 1 0 0 0 1 2Z M2 2V1A1 1 0 0 1 3 2Z M2 2H1A1 1 0 0 0 2 3Z" fill="#000"/>' +
  '</mask></defs><rect width="3" height="3" rx="0.42" fill="url(#xg)" mask="url(#xm)"/></svg>';

// The stepped hill along the bottom: column heights on an 8-step grid.
const HILL_HEIGHTS = [
  2, 2, 3, 3, 4, 5, 5, 6, 6, 5, 4, 4, 3, 3, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 3, 2, 2, 3, 4, 4, 3, 2,
];

function hillPath(): string {
  const parts = [`M0 8`];
  HILL_HEIGHTS.forEach((height, column) => {
    parts.push(`V${8 - height}H${column + 1}`);
  });
  parts.push('V8Z');
  return parts.join('');
}

const HILL =
  `<svg class="hill" viewBox="0 0 ${HILL_HEIGHTS.length} 8" preserveAspectRatio="none" ` +
  `shape-rendering="crispEdges" aria-hidden="true" focusable="false">` +
  `<path fill="currentColor" d="${hillPath()}"/></svg>`;

/**
 * Square pixel stars at fixed, random-looking places, as one box-shadow list
 * in viewport units so they cover the whole screen at any size.
 */
function starShadows(count: number, seed: number, colors: string[]): string {
  let state = seed;
  const next = () => {
    // Park-Miller: small enough to stay exact in floating point.
    state = (state * 48271) % 2147483647;
    return state / 2147483647;
  };
  const stars: string[] = [];
  for (let index = 0; index < count; index += 1) {
    const x = (next() * 100).toFixed(1);
    const y = (next() * 100).toFixed(1);
    const color = colors[Math.floor(next() * colors.length)] ?? '#f2f2ff';
    stars.push(`${x}vw ${y}vh 0 ${color}`);
  }
  return stars.join(', ');
}

const SMALL_STARS = starShadows(56, 7, ['#f2f2ff', '#f2f2ff', '#f2f2ff', '#4de8ff', '#a35cff']);
const BIG_STARS = starShadows(14, 41, ['#f2f2ff', '#ffe14d', '#4de8ff', '#a35cff']);

const STYLE = `
@font-face {
  font-family: "Press Start 2P";
  src: url("${PAGE_FONT_PATH}") format("truetype");
  font-display: swap;
}
:root {
  color-scheme: dark;
  --bg: #07071a;
  --fg: #f2f2ff;
  --yellow: #ffe14d;
  --orange: #ff9b3d;
  --pink: #ff4dd2;
  --cyan: #4de8ff;
  --red: #ff4d4d;
  --dim: #7a7aa8;
  --empty: #4a4a78;
  --hill: #1a1550;
  --violet: #a35cff;
  --deep: #3a1f7a;
  --pixel: "Press Start 2P", ui-monospace, "SF Mono", Menlo, Consolas, monospace;
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; background: var(--bg); }
body {
  display: flex;
  flex-direction: column;
  min-height: 100vh;
  min-height: 100dvh;
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 10px/1.6 var(--pixel);
  text-transform: uppercase;
}
a { color: inherit; }
a:focus-visible { outline: 2px dashed var(--yellow); outline-offset: 4px; }
.sky, .sky::after {
  position: fixed;
  top: 0;
  left: 0;
  width: 2px;
  height: 2px;
  box-shadow: ${SMALL_STARS};
  pointer-events: none;
}
.sky { z-index: 0; }
/*
 * Layers, back to front: stars, scanlines, hill, page content, then a soft vignette.
 * The scanlines stay behind the content so they never cross a character; the
 * vignette only darkens the screen's edges evenly.
 */
.sky::after { content: ""; width: 3px; height: 3px; box-shadow: ${BIG_STARS}; }
body::before {
  content: "";
  position: fixed;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  background: repeating-linear-gradient(to bottom, transparent 0 2px, rgba(0, 0, 0, 0.28) 2px 3px);
}
.crt {
  position: fixed;
  inset: 0;
  z-index: 4;
  pointer-events: none;
  background: radial-gradient(ellipse at center, transparent 60%, rgba(0, 0, 0, 0.55) 100%);
}
.wrap {
  position: relative;
  z-index: 3;
  flex: 1 0 auto;
  width: 100%;
  max-width: 640px;
  margin: 0 auto;
  padding: 0 16px;
}
.top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 20px 0 24px;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  text-decoration: none;
}
.brand svg { display: block; flex: none; }
/*
 * Text sits on solid backdrops in the page color, reaching 8px past it, so no star
 * shows between or right beside its pixels.
 */
.brand, .get, .hud > div, h1, .sub, table, .open, .foot {
  background: var(--bg);
  box-shadow: 0 0 0 8px var(--bg);
}
h1, .sub, .open, .foot { width: fit-content; max-width: 100%; margin-inline: auto; }
.get {
  flex: none;
  padding: 8px 10px;
  border: 2px solid var(--fg);
  box-shadow: 3px 3px 0 var(--violet), 0 0 0 8px var(--bg);
  font-size: 9px;
  text-decoration: none;
  white-space: nowrap;
}
.get:hover { border-color: var(--yellow); color: var(--yellow); }
.hud {
  display: grid;
  grid-template-columns: minmax(0, 3fr) minmax(0, 2fr) minmax(0, 2fr);
  gap: 8px;
  margin: 0 0 28px;
  text-align: center;
}
.hud dt { color: var(--red); font-size: 9px; }
.hud dd { margin: 6px 0 0; font-size: 11px; overflow-wrap: anywhere; }
h1 {
  margin-top: 0;
  margin-bottom: 16px;
  padding: 2px 8px 8px 2px;
  color: var(--yellow);
  font-size: clamp(16px, 5.2vw, 30px);
  font-weight: 400;
  line-height: 1.4;
  text-align: center;
  text-shadow: 3px 3px 0 var(--violet), 6px 6px 0 var(--deep);
}
.sub {
  margin-top: 0;
  margin-bottom: 30px;
  padding: 0 4px;
  color: var(--cyan);
  font-size: clamp(8px, 2.4vw, 10px);
  line-height: 1.9;
  text-align: center;
}
.sr {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}
table {
  width: 100%;
  border-collapse: collapse;
  font-size: clamp(9px, 2.8vw, 14px);
}
th {
  padding: 0 6px 12px;
  color: var(--red);
  font-size: 0.8em;
  font-weight: 400;
  text-align: left;
  white-space: nowrap;
}
td { padding: 8px 6px; vertical-align: middle; white-space: nowrap; }
th:first-child, td:first-child { padding-left: 0; }
th:last-child, td:last-child { padding-right: 0; text-align: right; }
.ship { width: 1px; padding-left: 2px; padding-right: 2px; text-align: center; }
.rocket { display: block; margin: 0 auto; }
td.name { width: 100%; max-width: 0; white-space: normal; }
.line { display: flex; align-items: baseline; gap: 8px; }
/* Every nickname is shown in full: a long one wraps and the dots shrink away. */
.nick { flex: 0 1 auto; min-width: 0; overflow-wrap: anywhere; }
.dots { flex: 1 1 0; min-width: 0; border-bottom: 3px dotted currentColor; opacity: 0.35; }
.gold { color: var(--yellow); }
.orange { color: var(--orange); }
.pink { color: var(--pink); }
.cyan { color: var(--cyan); }
.dim { color: var(--dim); }
tr[data-empty] { color: var(--empty); }
.open {
  margin-top: 30px;
  margin-bottom: 30px;
  padding: 0 4px;
  color: var(--yellow);
  font-size: clamp(8px, 2.4vw, 11px);
  line-height: 1.9;
  text-align: center;
  animation: blink 1s steps(1, end) infinite;
}
/* The text blinks; its backdrop stays, so the stars behind it do not. */
@keyframes blink { 50% { color: transparent; } }
@media (prefers-reduced-motion: reduce) {
  .open { animation: none; }
}
.foot { margin-top: 0; margin-bottom: 24px; padding: 0 4px; font-size: 10px; text-align: center; }
.free { color: var(--yellow); }
.src {
  color: var(--cyan);
  text-decoration: underline;
  text-decoration-thickness: 2px;
  text-underline-offset: 5px;
}
.hill {
  position: relative;
  z-index: 2;
  display: block;
  flex: none;
  width: 100%;
  height: 56px;
  color: var(--hill);
}
@media (max-width: 380px) {
  .brand { font-size: 10px; }
  .get { padding: 6px 8px; font-size: 8px; }
  .hud dt { font-size: 7px; }
  .hud dd { font-size: 9px; }
  td { padding: 7px 4px; }
  th { padding: 0 4px 10px; }
  .line { gap: 6px; }
}
`;

function memberRow(row: ListedRow): string {
  const ship = row.rank <= 3 ? rocket(row.rank === 1 ? 3 : 2) : '';
  return (
    `<tr data-rank="${row.rank}" class="${tone(row)}"><td class="rank">${ordinal(row.rank)}</td>` +
    `<td class="ship">${ship}</td>` +
    `<td class="name"><div class="line"><span class="nick">${escapeHtml(row.nickname)}</span>` +
    `<span class="dots" aria-hidden="true"></span></div></td>` +
    `<td class="lev">${formatLeverage(row.leverage_tenths)}</td></tr>`
  );
}

function emptyRow(position: number): string {
  return (
    `<tr data-empty aria-hidden="true"><td class="rank">${ordinal(position)}</td><td class="ship"></td>` +
    `<td class="name"><div class="line"><span class="nick">---</span>` +
    `<span class="dots"></span></div></td><td class="lev">${NO_LEVERAGE}</td></tr>`
  );
}

function boardHtml(rows: ListedRow[]): string {
  const open: string[] = [];
  for (let position = rows.length + 1; position <= BOARD_ROWS; position += 1)
    open.push(emptyRow(position));
  return [
    '<table>',
    '<caption class="sr">Agent leverage over the last 7 days, highest first</caption>',
    '<thead><tr><th scope="col" class="rank">RANK</th><th scope="col" class="ship"></th>' +
      '<th scope="col" class="name">NAME</th><th scope="col" class="lev">LEVER</th></tr></thead>',
    '<tbody>',
    ...rows.map(memberRow),
    ...open,
    '</tbody>',
    '</table>',
  ].join('\n');
}

/** The blinking line under the board: the first open place, or how to join when the board is full. */
function openLine(count: number): string {
  return count < BOARD_ROWS
    ? `${ordinal(count + 1)}&nbsp;PLACE IS OPEN&nbsp;· JOIN FROM THE APP`
    : 'JOIN FROM THE XTRACE APP';
}

export function renderPage(rows: ListedRow[]): string {
  const leader = rows[0];
  const players = String(rows.length).padStart(2, '0');
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>XTrace Leaderboard</title>
<meta name="description" content="Leverage over the last 7 days, shared by people who opted in from the XTrace app.">
<meta name="theme-color" content="#07071a">
<link rel="preload" href="${PAGE_FONT_PATH}" as="font" type="font/ttf" crossorigin>
<style>${STYLE}</style>
</head>
<body>
<div class="sky" aria-hidden="true"></div>
<div class="wrap">
<header class="top">
<a class="brand" href="https://xtrace.ai">${LOGO}XTRACE</a>
<a class="get" href="https://xtrace.ai">GET XTRACE</a>
</header>
<main>
<dl class="hud">
<div><dt>1UP</dt><dd>${leader ? escapeHtml(leader.nickname) : '---'}</dd></div>
<div><dt>HI-LEVER</dt><dd>${leader ? formatLeverage(leader.leverage_tenths) : NO_LEVERAGE}</dd></div>
<div><dt>PLAYERS</dt><dd>${players}</dd></div>
</dl>
<h1>HIGH AGENT LEVERAGE</h1>
<p class="sub">AGENT HOURS&nbsp;÷ YOUR HOURS&nbsp;· LAST&nbsp;7&nbsp;DAYS</p>
${boardHtml(rows)}
<p class="open">${openLine(rows.length)}</p>
</main>
<footer class="foot">
<span class="free">FREE</span>&nbsp;· <a class="src" href="https://github.com/XTraceAI/xtrace">OPEN SOURCE</a>
</footer>
</div>
${HILL}
<div class="crt" aria-hidden="true"></div>
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
        "default-src 'none'; style-src 'unsafe-inline'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'",
    },
  });
}
