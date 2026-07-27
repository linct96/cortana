import type { AccountProduct } from '../../components/app-shell-context';

export type AgentsProfile = {
  id: string;
  name: string;
  content: string;
  isActive: boolean;
};

export type AgentsStatus = {
  profiles: AgentsProfile[];
  path: string;
  fileState: 'managed' | 'unmanaged' | 'missing';
  unmanagedContent: string | null;
};

export function instructionFilename(product: AccountProduct) {
  return product === 'claude' ? 'CLAUDE.md' : product === 'antigravity' ? 'GEMINI.md' : 'AGENTS.md';
}
