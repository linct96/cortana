import antigravityIcon from './assets/antigravity.svg';
import chatGptIcon from './assets/chatgpt.svg';
import claudeIcon from './assets/claude.svg';
import grokIcon from './assets/grok.svg';
import piIcon from './assets/pi.svg';

export type AccountProduct = 'codex' | 'claude' | 'antigravity' | 'grok' | 'pi';
export type ProductPath =
  | '/accounts'
  | '/analytics'
  | '/sessions'
  | '/prompts'
  | '/models'
  | '/config';

export type ProductCapabilities = {
  models: boolean;
  sessions: boolean;
  analytics: boolean;
  prompts: boolean;
  config: boolean;
  gateway: boolean;
  usage: boolean;
  relay: boolean;
  browserOAuth: boolean;
  pasteCredential: boolean;
  importCurrent: boolean;
  refreshAccount: boolean;
  refreshAllAccounts: boolean;
  openCliFromAccount: boolean;
  showAuthPath: boolean;
  showCliAlert: boolean;
};

type ProductMeta = {
  name: string;
  icon: string;
  capabilities: ProductCapabilities;
};

const full = {
  models: true,
  sessions: true,
  analytics: true,
  prompts: true,
  config: true,
  usage: true,
  browserOAuth: true,
  pasteCredential: true,
  importCurrent: true,
  refreshAccount: true,
  refreshAllAccounts: true,
  showCliAlert: true,
};

const PRODUCTS: Record<AccountProduct, ProductMeta> = {
  codex: {
    name: 'Codex',
    icon: chatGptIcon,
    capabilities: {
      ...full,
      gateway: true,
      relay: true,
      openCliFromAccount: true,
      showAuthPath: false,
    },
  },
  claude: {
    name: 'Claude',
    icon: claudeIcon,
    capabilities: {
      ...full,
      gateway: false,
      relay: true,
      openCliFromAccount: false,
      showAuthPath: true,
    },
  },
  antigravity: {
    name: 'Antigravity',
    icon: antigravityIcon,
    capabilities: {
      ...full,
      models: false,
      gateway: false,
      relay: false,
      pasteCredential: false,
      openCliFromAccount: false,
      showAuthPath: true,
    },
  },
  grok: {
    name: 'Grok',
    icon: grokIcon,
    capabilities: {
      ...full,
      gateway: false,
      relay: true,
      openCliFromAccount: false,
      showAuthPath: false,
    },
  },
  pi: {
    name: 'Pi',
    icon: piIcon,
    capabilities: {
      models: false,
      sessions: false,
      analytics: false,
      prompts: false,
      config: false,
      gateway: false,
      usage: true,
      relay: true,
      browserOAuth: false,
      pasteCredential: false,
      importCurrent: true,
      refreshAccount: true,
      refreshAllAccounts: true,
      openCliFromAccount: false,
      showAuthPath: true,
      showCliAlert: false,
    },
  },
};

export const PRODUCT_ORDER: AccountProduct[] = [
  'antigravity',
  'claude',
  'codex',
  'grok',
  'pi',
];

export function productMeta(product: AccountProduct) {
  return PRODUCTS[product];
}

export function productName(product: AccountProduct) {
  return PRODUCTS[product].name;
}

export function productSupportsPath(
  product: AccountProduct,
  path: ProductPath,
) {
  if (path === '/accounts') return true;
  const key = path.slice(1) as Exclude<
    keyof ProductCapabilities,
    | 'gateway'
    | 'usage'
    | 'relay'
    | 'browserOAuth'
    | 'pasteCredential'
    | 'importCurrent'
    | 'refreshAccount'
    | 'refreshAllAccounts'
    | 'openCliFromAccount'
    | 'showAuthPath'
    | 'showCliAlert'
  >;
  return PRODUCTS[product].capabilities[key];
}
