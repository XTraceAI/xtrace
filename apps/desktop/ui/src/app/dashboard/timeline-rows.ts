/** The 7-day range's eight days fill the plot; longer ranges keep that row size and scroll. */
export const VISIBLE_ROWS = 8;
/** Below this a row's bar and its gap stop reading as separate rows. */
const MIN_ROW_PX = 28;
/** With the agents line: a taller track, so the line's height reads. */
const MIN_PAIRED_ROW_PX = 34;

/**
 * The human timeline's row height for a plot `plotHeight` pixels tall: the
 * height eight rows share, never below the view's minimum ("human + agent"
 * when `paired`).
 */
export const timelineRowHeight = (plotHeight: number, paired: boolean) =>
  Math.max(paired ? MIN_PAIRED_ROW_PX : MIN_ROW_PX, Math.floor(plotHeight / VISIBLE_ROWS));
