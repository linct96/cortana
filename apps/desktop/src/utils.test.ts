import { describe, expect, it } from 'vitest';
import { formatResetTime } from './utils';

describe('formatResetTime', () => {
  it('formats the remaining reset duration', () => {
    const now = Date.UTC(2026, 6, 24);

    expect(formatResetTime(now + ((3 * 24 + 12) * 60 + 30) * 60_000, now)).toBe('重置于3天12时');
    expect(formatResetTime(now + (3 * 60 + 12) * 60_000, now)).toBe('重置于3时12分');
    expect(formatResetTime(now + 12 * 60_000, now)).toBe('重置于12分');
  });
});
