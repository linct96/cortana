import { createContext, type Dispatch, type SetStateAction, useContext } from 'react';
import type { AccountProduct } from '../products';

export { productName, type AccountProduct } from '../products';

export type MainPath =
  | '/accounts'
  | '/sessions'
  | '/analytics'
  | '/prompts'
  | '/models'
  | '/config';
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
