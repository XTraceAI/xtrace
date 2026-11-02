import { useId, type CSSProperties, type ReactNode } from 'react';
import { RuleChip } from './RuleChip';
import type { RuleId } from './rules';
import '../styles/metrics.css';

export function SectionCard({
  title,
  titleNote,
  ruleId,
  meta,
  metaFull,
  right,
  headerHeight = 40,
  padding = '12px 14px 10px',
  children,
  footer,
}: {
  title: string;
  /**
   * Drawn right after the title in its font but not in capitals, e.g. a
   * window such as "(last 48 hours)". The card's name stays the title alone.
   */
  titleNote?: string;
  ruleId?: RuleId;
  meta?: string;
  /**
   * The subtitle in full, when it says more than the short line: its tooltip,
   * and what assistive technology reads in place of the short line.
   */
  metaFull?: string;
  right?: ReactNode;
  headerHeight?: 36 | 40;
  padding?: CSSProperties['padding'];
  children: ReactNode;
  footer?: ReactNode;
}) {
  const id = useId();
  return (
    <section className="xt-section-card" aria-labelledby={id}>
      <header className="xt-section-header" style={{ height: headerHeight }}>
        <h2 id={id} title={title}>
          {title}
        </h2>
        {titleNote && <span className="xt-section-title-note">{titleNote}</span>}
        {ruleId && <RuleChip ruleId={ruleId} subject={title} />}
        {meta && (
          <span className="xt-section-meta" title={metaFull ?? meta}>
            {metaFull ? (
              <>
                <span aria-hidden="true">{meta}</span>
                <span className="sr-only">{metaFull}</span>
              </>
            ) : (
              meta
            )}
          </span>
        )}
        {right && <span className="xt-section-right">{right}</span>}
      </header>
      <div className="xt-section-panel" style={{ padding }}>
        {children}
        {footer && <footer className="xt-section-footer">{footer}</footer>}
      </div>
    </section>
  );
}
