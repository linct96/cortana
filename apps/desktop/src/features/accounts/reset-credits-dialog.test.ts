import { describe, expect, it, vi } from 'vitest';
import { resetCreditOutcomeNotice, runResetCreditAttempt } from './types';

describe('reset credit redemption', () => {
  it('only runs one request while confirmation is pending', async () => {
    let finish!: (value: boolean) => void;
    const request = vi.fn(() => new Promise<boolean>((resolve) => (finish = resolve)));
    const lock = { current: false };

    const first = runResetCreditAttempt(lock, request);
    const duplicate = runResetCreditAttempt(lock, request);

    expect(request).toHaveBeenCalledOnce();
    expect(await duplicate).toBe(false);
    finish(true);
    expect(await first).toBe(true);
  });

  it('maps every server outcome to the expected feedback', () => {
    expect(resetCreditOutcomeNotice('reset').kind).toBe('success');
    expect(resetCreditOutcomeNotice('alreadyRedeemed').kind).toBe('success');
    expect(resetCreditOutcomeNotice('nothingToReset').message).toContain('未消耗');
    expect(resetCreditOutcomeNotice('noCredit').message).toContain('无可用');
  });
});
