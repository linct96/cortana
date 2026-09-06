import { describe, expect, it } from 'vitest';
import { productMeta, productSupportsPath } from './products';

it('limits Pi to account management', () => {
  const pi = productMeta('pi');
  expect(pi.name).toBe('Pi');
  expect(pi.capabilities.importCurrent).toBe(true);
  expect(pi.capabilities.usage).toBe(true);
  expect(pi.capabilities.refreshAccount).toBe(true);
  expect(pi.capabilities.refreshAllAccounts).toBe(true);
  expect(pi.capabilities.showAuthPath).toBe(true);
  for (const path of ['/analytics', '/sessions', '/prompts', '/models', '/config'] as const) {
    expect(productSupportsPath('pi', path)).toBe(false);
  }
});

describe('existing product policy', () => {
  it('keeps model navigation behavior', () => {
    expect(productMeta('codex').capabilities.models).toBe(true);
    expect(productMeta('claude').capabilities.models).toBe(true);
    expect(productMeta('grok').capabilities.models).toBe(true);
    expect(productMeta('antigravity').capabilities.models).toBe(false);
  });
});
