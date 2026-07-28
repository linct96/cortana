import { describe, expect, it } from 'vitest';
import { resetCreditOutcomeNotice } from './types';

describe('reset credit redemption', () => {
  it('maps every server outcome to the expected feedback', () => {
    expect(resetCreditOutcomeNotice('reset').kind).toBe('success');
    expect(resetCreditOutcomeNotice('alreadyRedeemed').kind).toBe('success');
    expect(resetCreditOutcomeNotice('nothingToReset').message).toContain('未消耗');
    expect(resetCreditOutcomeNotice('noCredit').message).toContain('无可用');
  });
});
