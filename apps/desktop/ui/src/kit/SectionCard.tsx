import { useId, type CSSProperties, type ReactNode } from 'react';
import { RuleChip } from './RuleChip';
import type { RuleId } from './rules';
import '../styles/metrics.css';

export function SectionCard({
  title,
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
