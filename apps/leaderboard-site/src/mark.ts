// The XTrace mark (the app's sidebar logo) on its 3×3 grid: a square with
// rounded corners, less four quarter discs of radius 1, one in each of four
// cells. The header logo (src/page.ts) and the pixel-art icons
// (scripts/icons.ts) are both drawn from these numbers.

/** How far each corner of the square is rounded. */
export const MARK_CORNER = 0.42;

/** The mark's gradient, top right to bottom left. */
export const MARK_COLORS = ['#7F8CFF', '#A35CFF'] as const;

/** Each cut: the cell it sits in (x0, y0) and the disc's centre, a corner of that cell. */
export const MARK_CUTS = [
  { x0: 1, y0: 0, cx: 1, cy: 1 },
  { x0: 0, y0: 1, cx: 1, cy: 1 },
  { x0: 2, y0: 1, cx: 2, cy: 2 },
  { x0: 1, y0: 2, cx: 2, cy: 2 },
] as const;

/** Whether the point (x, y) on the 3×3 grid is part of the mark. */
export function insideMark(x: number, y: number): boolean {
  const nx = Math.min(Math.max(x, MARK_CORNER), 3 - MARK_CORNER);
  const ny = Math.min(Math.max(y, MARK_CORNER), 3 - MARK_CORNER);
  if ((x - nx) ** 2 + (y - ny) ** 2 > MARK_CORNER ** 2) return false;
  return !MARK_CUTS.some(
    (c) =>
      x >= c.x0 &&
      x < c.x0 + 1 &&
      y >= c.y0 &&
      y < c.y0 + 1 &&
      (x - c.cx) ** 2 + (y - c.cy) ** 2 < 1,
  );
}

/** The cuts as one SVG path: each runs from its centre along one cell edge, round the arc, and back. */
export function markCutsPath(): string {
  return MARK_CUTS.map((c) => {
    // The cell's other corners on the disc: straight across from the centre.
    const ax = c.cx;
    const ay = c.y0 === c.cy ? c.y0 + 1 : c.y0;
    const bx = c.x0 === c.cx ? c.x0 + 1 : c.x0;
    const by = c.cy;
    const sweep = (ax - c.cx) * (by - c.cy) - (ay - c.cy) * (bx - c.cx) > 0 ? 1 : 0;
    return `M${c.cx} ${c.cy}L${ax} ${ay}A1 1 0 0 ${sweep} ${bx} ${by}Z`;
  }).join('');
}
