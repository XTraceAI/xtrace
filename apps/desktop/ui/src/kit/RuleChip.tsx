import { RulePopover } from './RulePopover';
import type { RuleId } from './rules';

export function RuleChip({
  ruleId,
  subject,
  size = 'normal',
}: {
  ruleId: RuleId;
  subject: string;
  size?: 'normal' | 'sm';
}) {
  return (
    <RulePopover ruleId={ruleId}>
      <button
        type="button"
        className="xt-rule-chip"
        data-size={size}
        aria-label={`About ${subject}`}
      >
        Info
      </button>
    </RulePopover>
  );
}
