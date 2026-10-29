// Draws the page's tab and home-screen icons: the XTrace mark (src/mark.ts,
// the same shape as the header logo) as 8-bit pixel art.
// Usage: node --import ./scripts/ts-resolve.mjs scripts/icons.ts
// Writes public/favicon.svg, public/favicon-32.png and public/apple-touch-icon.png;
// a test checks the files in public/ are what this draws.
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';
import { insideMark, MARK_COLORS } from '../src/mark.js';

/** Pixels across the mark: 5 per cell of its 3×3 grid. */
export const ICON_GRID = 15;

/** The page background, behind the home-screen icon. */
const BACKGROUND = '#07071a';

function rgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function hex(color: number[]): string {
  return `#${color.map((c) => Math.round(c).toString(16).padStart(2, '0').toUpperCase()).join('')}`;
}

/** The mark's gradient in three flat steps: its two ends and the colour halfway. */
export const ICON_SHADES = [
  MARK_COLORS[0],
  hex(rgb(MARK_COLORS[0]).map((c, i) => (c + rgb(MARK_COLORS[1])[i]!) / 2)),
  MARK_COLORS[1],
];

/** Each pixel's shade index into ICON_SHADES, or null where the mark is not drawn. */
export function iconPixels(): (number | null)[][] {
  const step = 3 / ICON_GRID;
  return Array.from({ length: ICON_GRID }, (_, row) =>
    Array.from({ length: ICON_GRID }, (_, col) => {
      const x = (col + 0.5) * step;
      const y = (row + 0.5) * step;
      if (!insideMark(x, y)) return null;
      // Gradient position: 0 at the top right, 1 at the bottom left.
      const t = (3 - x + y) / 6;
      return Math.min(ICON_SHADES.length - 1, Math.floor(t * ICON_SHADES.length));
    }),
  );
}

/** The tab icon as SVG: one square per pixel, drawn without smoothing. */
export function iconSvg(): string {
  const rects = iconPixels()
    .flatMap((row, y) =>
      row.map((shade, x) =>
        shade === null
          ? ''
          : `<rect x="${x}" y="${y}" width="1" height="1" fill="${ICON_SHADES[shade]}"/>`,
      ),
    )
    .join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${ICON_GRID} ${ICON_GRID}" shape-rendering="crispEdges">${rects}</svg>\n`;
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
  // 15 pixels × 2 = 30, centred in 32 with a 1px clear edge.
  'favicon-32.png': () => iconPng(32, 2, null),
  // 15 × 10 = 150, centred in 180 on the page background.
  'apple-touch-icon.png': () => iconPng(180, 10, BACKGROUND),
} as const;

/** The unfiltered RGBA rows a PNG of iconPng stores, so tests can compare pixels, not compressed bytes. */
export function iconRows(size: number, scale: number, background: string | null): Buffer {
  const pixels = iconPixels();
  const offset = Math.floor((size - ICON_GRID * scale) / 2);
  const back = background === null ? null : rgb(background);
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // no filter
    for (let x = 0; x < size; x++) {
      const gx = Math.floor((x - offset) / scale);
      const gy = Math.floor((y - offset) / scale);
      const inGrid = x >= offset && y >= offset && gx < ICON_GRID && gy < ICON_GRID;
      const shade = inGrid ? (pixels[gy]![gx] ?? null) : null;
      const color = shade === null ? back : rgb(ICON_SHADES[shade]!);
      const at = y * (size * 4 + 1) + 1 + x * 4;
      if (color) raw.set([...color, 255], at);
    }
  }
  return raw;
}

/**
 * A square RGBA PNG of `size` pixels: each mark pixel becomes a `scale`-wide
 * block, centred, on `background` (transparent when null).
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
