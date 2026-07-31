import { describe, expect, it } from 'vitest';
import { resetCreditOutcomeNotice, usageWindowLabel } from './types';

describe('reset credit redemption', () => {
  it('maps every server outcome to the expected feedback', () => {
    expect(resetCreditOutcomeNotice('reset').kind).toBe('success');
    expect(resetCreditOutcomeNotice('alreadyRedeemed').kind).toBe('success');
    expect(resetCreditOutcomeNotice('nothingToReset').message).toContain('未消耗');
    expect(resetCreditOutcomeNotice('noCredit').message).toContain('无可用');
  });
});

describe('usage window labels', () => {
  it('labels the supported quota periods', () => {
    expect(usageWindowLabel(300)).toBe('5h额度');
    expect(usageWindowLabel(10_080)).toBe('周额度');
    expect(usageWindowLabel(43_800)).toBe('月额度');
  });
});
