import { createContext, type Dispatch, type SetStateAction, useContext } from 'react';

export type MainPath = '/accounts' | '/sessions' | '/analytics' | '/prompts' | '/config';
export type AccountProduct = 'codex' | 'claude' | 'antigravity' | 'grok';

export type AppShellContextValue = {
  topPadding: string;
  previousMainPath: MainPath;
  activeProduct: AccountProduct;
  setActiveProduct: Dispatch<SetStateAction<AccountProduct>>;
  cliAvailable: boolean | null;
  setCliAvailable: Dispatch<SetStateAction<boolean | null>>;
  hasUnsavedChanges: boolean;
  setHasUnsavedChanges: Dispatch<SetStateAction<boolean>>;
};

export const AppShellContext = createContext<AppShellContextValue | null>(null);

export function useAppShell() {
  const context = useContext(AppShellContext);
  if (!context) throw new Error('useAppShell must be used within AppShell');
  return context;
}

export function productName(product: AccountProduct) {
  return product === 'claude'
    ? 'Claude'
    : product === 'antigravity'
      ? 'Antigravity'
      : product === 'grok'
        ? 'Grok'
        : 'Codex';
}
