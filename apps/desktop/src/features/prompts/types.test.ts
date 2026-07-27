import { describe, expect, it } from 'vitest';
import { instructionFilename } from './types';

describe('instructionFilename', () => {
  it('maps each product to its native global instruction file', () => {
    expect(instructionFilename('codex')).toBe('AGENTS.md');
    expect(instructionFilename('claude')).toBe('CLAUDE.md');
    expect(instructionFilename('antigravity')).toBe('GEMINI.md');
    expect(instructionFilename('grok')).toBe('AGENTS.md');
  });
});
