import { Tooltip } from '@base-ui/react/tooltip';
import { useId, type ReactElement } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';
import { ruleSummary, type RuleId } from './rules';
import '../styles/metrics.css';

/**
 * Hover waits this long before a definition opens, so passing the pointer
 * over a row of tiles does not flash one popover after another. Keyboard
 * focus still opens it at once: Base UI applies the delay to hover only.
 */
export const RULE_OPEN_DELAY_MS = 450;

/**
 * A small two-column table under a definition: a caption, the label and value
 * columns' headings, then one row per pair, values aligned at the end.
 */
export interface RuleTable {
  caption: string;
  /**
   * Draw the table alone, with no definition line or caption above it. Both
   * are still read in the description. A table with no rows keeps the
   * definition visible.
   */
  bare?: boolean;
  columns: readonly [label: string, value: string];
  rows: readonly { label: string; value: string }[];
}

/** The table as one sentence: `Share of output tokens: a 62%, b 30%.` */
export const tableText = (table: RuleTable) =>
  `${table.caption}: ${table.rows.map((row) => `${row.label} ${row.value}`).join(', ')}.`;

/**
 * Passive definition text: the rule's one-line summary, then optional rows
 * and a short note about this value. Base UI owns hover, focus, Escape, and
 * positioning.
 */
export function RulePopover({
  ruleId,
  context,
  table,
  text,
  closeOnClick = false,
  children,
}: {
  ruleId: RuleId;
  context?: string;
  /** Shown between the definition and the note; nothing when it has no rows. */
  table?: RuleTable;
  /** Plain words shown instead of the rule's own definition. */
  text?: string;
  /** True when clicking the trigger opens something else (a dialog). */
  closeOnClick?: boolean;
  children: ReactElement;
}) {
  const id = useId();
  const theme = useSurfaceTheme();
  const bare = Boolean(table?.bare && table.rows.length > 0);
  return (
    <Tooltip.Root disableHoverablePopup>
      <Tooltip.Trigger
        render={children}
        tabIndex={0}
        aria-describedby={id}
        delay={RULE_OPEN_DELAY_MS}
        closeOnClick={closeOnClick}
      />
      <Tooltip.Portal data-theme={theme}>
        <Tooltip.Positioner
          className="xt-rule-positioner"
          positionMethod="fixed"
          side="bottom"
          align="start"
          sideOffset={8}
          collisionPadding={8}
        >
          <Tooltip.Popup id={id} role="tooltip" className="xt-rule-popover xt-rule-definition">
            <span className={bare ? 'sr-only' : undefined}>{text ?? ruleSummary(ruleId)}</span>
            {table && table.rows.length > 0 && (
              <>
                {/* The popover is read as the trigger's description, where a
                    table counts only as its caption: assistive technology reads
                    these words instead, and the table is drawn for the eye. */}
                <span className="sr-only">{tableText(table)}</span>
                <table className="xt-rule-table" data-bare={bare || undefined} aria-hidden="true">
                  {!bare && <caption>{table.caption}</caption>}
                  <thead>
                    <tr>
                      <th scope="col">{table.columns[0]}</th>
                      <th scope="col">{table.columns[1]}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {table.rows.map((row, index) => (
                      <tr key={index}>
                        <td>{row.label}</td>
                        <td>{row.value}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            )}
            {context && <span className="xt-rule-context">{context}</span>}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
