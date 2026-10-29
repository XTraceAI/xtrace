// Draws the page's tab and home-screen icons: the XTrace mark (src/mark.ts,
// the same shape as the header logo) as chunky 8-bit pixel art in its own two
// colours, with few enough pixels that the steps still show in a 16px tab.
// Usage: node --import ./scripts/ts-resolve.mjs scripts/icons.ts
// Writes public/favicon.svg, public/favicon-32.png and public/apple-touch-icon.png;
// a test checks the files in public/ are what this draws.
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';
import { insideMark, MARK_COLORS } from '../src/mark.js';
import { PAGE_COLORS } from '../src/page.js';

/** Pixels across the mark. */
export const MARK_PIXELS = 8;

/**
 * Each pixel's colour, row by row, or null where the mark is not drawn: the
 * gradient's first colour above the diagonal from top left to bottom right,
 * its second below, as the smooth logo runs from top right to bottom left.
 */
export function iconPixels(): (string | null)[][] {
  return Array.from({ length: MARK_PIXELS }, (_, y) =>
    Array.from({ length: MARK_PIXELS }, (_, x) => {
      if (!insideMark(((x + 0.5) * 3) / MARK_PIXELS, ((y + 0.5) * 3) / MARK_PIXELS)) return null;
      return y <= x ? MARK_COLORS[0] : MARK_COLORS[1];
    }),
  );
}

/** The tab icon as SVG: one square per pixel, drawn without smoothing, on a clear background. */
export function iconSvg(): string {
  const rects = iconPixels()
    .flatMap((row, y) =>
      row.map((color, x) =>
        color === null ? '' : `<rect x="${x}" y="${y}" width="1" height="1" fill="${color}"/>`,
      ),
    )
    .join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${MARK_PIXELS} ${MARK_PIXELS}" shape-rendering="crispEdges">${rects}</svg>\n`;
}

function rgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function chunk(type: string, data: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

/** The three icon files, as public/ holds them. */
export const ICON_FILES = {
  'favicon.svg': () => iconSvg(),
  // 8 pixels × 4 = 32, on a clear background like the SVG.
  'favicon-32.png': () => iconPng(32, 4, null),
  // 8 × 18 = 144, centred in 180 on the page's navy: home screens fill clear pixels with black.
  'apple-touch-icon.png': () => iconPng(180, 18, PAGE_COLORS.bg),
} as const;

/** The unfiltered RGBA rows iconPng compresses: one filter byte, then each pixel's colour. */
function iconRows(size: number, scale: number, background: string | null): Buffer {
  const pixels = iconPixels();
  const offset = Math.floor((size - MARK_PIXELS * scale) / 2);
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // no filter
    for (let x = 0; x < size; x++) {
      const gx = Math.floor((x - offset) / scale);
      const gy = Math.floor((y - offset) / scale);
      const color = (x >= offset && y >= offset ? pixels[gy]?.[gx] : null) ?? background;
      if (color) raw.set([...rgb(color), 255], y * (size * 4 + 1) + 1 + x * 4);
    }
  }
  return raw;
}

/**
 * A square RGBA PNG of `size` pixels: each mark pixel is a `scale`-wide block,
 * centred, on `background` (clear when null).
 */
export function iconPng(size: number, scale: number, background: string | null): Buffer {
  const raw = iconRows(size, scale, background);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header.set([8, 6, 0, 0, 0], 8); // 8-bit RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  for (const [name, draw] of Object.entries(ICON_FILES))
    writeFileSync(new URL(`../public/${name}`, import.meta.url), draw());
  console.log('Wrote public/favicon.svg, favicon-32.png and apple-touch-icon.png.');
}
