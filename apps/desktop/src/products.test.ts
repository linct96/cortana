import { describe, expect, it } from 'vitest';
import { productMeta, productSupportsPath } from './products';

it('enables Pi analytics while keeping other sections limited', () => {
  const pi = productMeta('pi');
  expect(pi.name).toBe('Pi');
  expect(pi.capabilities.importCurrent).toBe(true);
  expect(pi.capabilities.usage).toBe(true);
  expect(pi.capabilities.refreshAccount).toBe(true);
  expect(pi.capabilities.refreshAllAccounts).toBe(true);
  expect(pi.capabilities.showAuthPath).toBe(true);
  expect(pi.capabilities.analytics).toBe(true);
  expect(productSupportsPath('pi', '/analytics')).toBe(true);
  for (const path of ['/sessions', '/prompts', '/models', '/config'] as const) {
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
