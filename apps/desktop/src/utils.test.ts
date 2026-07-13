import { describe, expect, it } from 'vitest';
import { authFileParent, formatResetTime } from './utils';

describe('authFileParent', () => {
  it('supports macOS and Windows Codex auth paths', () => {
    expect(authFileParent('/Users/you/.codex/auth.json')).toBe('/Users/you/.codex');
    expect(authFileParent('C:\\Users\\you\\.codex\\auth.json')).toBe('C:\\Users\\you\\.codex');
  });
});

describe('formatResetTime', () => {
  it('formats reset timestamps as MM-DD HH:mm', () => {
    expect(formatResetTime(new Date(2026, 5, 7, 12, 23).getTime())).toBe('06-07 12:23');
  });
});
