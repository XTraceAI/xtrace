// Draws the page's tab and home-screen icons: the XTrace mark (src/mark.ts,
// the same shape as the header logo) as chunky 8-bit pixel art in the arcade
// title's style: yellow with a hard violet shadow, on the page's navy.
// Usage: node --import ./scripts/ts-resolve.mjs scripts/icons.ts
// Writes public/favicon.svg, public/favicon-32.png and public/apple-touch-icon.png;
// a test checks the files in public/ are what this draws.
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';
import { insideMark } from '../src/mark.js';
import { PAGE_COLORS } from '../src/page.js';

/** Pixels across the mark. Few enough that the steps still show in a 16px tab. */
export const MARK_PIXELS = 8;

/** Pixels across the whole icon: the mark, a one-pixel edge, and its shadow one pixel down and right. */
export const ICON_TILE = MARK_PIXELS + 2;

/** Each icon pixel's colour, row by row: navy, the violet shadow, or the yellow mark. */
export function iconPixels(): string[][] {
  const mark = (col: number, row: number) =>
    col >= 0 &&
    row >= 0 &&
    col < MARK_PIXELS &&
    row < MARK_PIXELS &&
    insideMark(((col + 0.5) * 3) / MARK_PIXELS, ((row + 0.5) * 3) / MARK_PIXELS);
  return Array.from({ length: ICON_TILE }, (_, y) =>
    Array.from({ length: ICON_TILE }, (_, x) => {
      if (mark(x - 1, y - 1)) return PAGE_COLORS.yellow;
      if (mark(x - 2, y - 2)) return PAGE_COLORS.violet;
      return PAGE_COLORS.bg;
    }),
  );
}

/** The tab icon as SVG: the navy tile, then one square per shadow or mark pixel, drawn without smoothing. */
export function iconSvg(): string {
  const rects = iconPixels()
    .flatMap((row, y) =>
      row.map((color, x) =>
        color === PAGE_COLORS.bg
          ? ''
          : `<rect x="${x}" y="${y}" width="1" height="1" fill="${color}"/>`,
      ),
    )
    .join('');
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${ICON_TILE} ${ICON_TILE}" shape-rendering="crispEdges">` +
    `<rect width="${ICON_TILE}" height="${ICON_TILE}" fill="${PAGE_COLORS.bg}"/>${rects}</svg>\n`
  );
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
  // 10 pixels × 3 = 30, centred in 32 on navy.
  'favicon-32.png': () => iconPng(32, 3),
  // 10 × 18 = 180 exactly.
  'apple-touch-icon.png': () => iconPng(180, 18),
} as const;

/** The unfiltered RGBA rows a PNG of iconPng stores, so tests can compare pixels, not compressed bytes. */
export function iconRows(size: number, scale: number): Buffer {
  const pixels = iconPixels();
  const offset = Math.floor((size - ICON_TILE * scale) / 2);
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // no filter
    for (let x = 0; x < size; x++) {
      const gx = Math.floor((x - offset) / scale);
      const gy = Math.floor((y - offset) / scale);
      const color = pixels[gy]?.[gx] ?? PAGE_COLORS.bg;
      raw.set([...rgb(color), 255], y * (size * 4 + 1) + 1 + x * 4);
    }
  }
  return raw;
}

/** A square, opaque RGBA PNG of `size` pixels: each icon pixel is a `scale`-wide block, centred on navy. */
export function iconPng(size: number, scale: number): Buffer {
  const raw = iconRows(size, scale);
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
