import type { AccountProduct } from '../../components/app-shell-context';

export type Profile = {
  id: string;
  product: AccountProduct;
  accountType: 'oauth' | 'relay';
  apiBaseUrl: string | null;
  accountId: string;
  email: string;
  alias: string;
  planType: string;
  usagePrimary: UsageWindow | null;
  usageSecondary: UsageWindow | null;
  antigravityQuota: AntigravityQuota | null;
  usageUpdatedAt: number | null;
  resetCreditsAvailableCount: number | null;
  isRenewable: boolean;
  isActive: boolean;
  lastUsedAt: number | null;
  updatedAt: number;
};

export type { AccountProduct };

export type AntigravityQuota = {
  projectId: string | null;
  forbidden: boolean;
  models: AntigravityModelQuota[];
  groups: AntigravityQuotaGroup[];
};

export type AntigravityModelQuota = {
  modelId: string;
  displayName: string;
  remainingPercent: number;
  resetsAt: number | null;
};

export type AntigravityQuotaGroup = {
  displayName: string;
  buckets: AntigravityQuotaBucket[];
};

export type AntigravityQuotaBucket = {
  bucketId: string;
  window: string;
  displayName: string;
  remainingPercent: number;
  resetsAt: number | null;
};

export type ResetCredits = {
  availableCount: number;
  credits: ResetCredit[];
};

export type ResetCredit = {
  id: string;
  title: string;
  status: string;
  expiresAt: string;
  grantedAt: string;
};

export type UsageWindow = {
  usedPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
};

export type AuthState = {
  kind: 'managed' | 'unmanaged' | 'missing';
  message: string;
};

export type AppStatus = {
  profiles: Profile[];
  detectedProfile: Profile | null;
  authPath: string;
  authState: AuthState;
  autostartEnabled: boolean;
};

export type OAuthProgress = {
  stage: 'browser_opening' | 'waiting' | 'exchanging' | 'success' | 'error' | 'cancelled';
  message: string;
  profile: Profile | null;
};

export type PendingConfirm = {
  kind: 'force-switch' | 'delete';
  profile: Profile;
} | null;

export type AddMode = 'browser' | 'paste' | 'relay';

export function planLabel(planType: string) {
  const normalized = planType.trim().toLowerCase();
  if (normalized.includes('ultra')) return 'Ultra';
  if (normalized.includes('pro')) return 'Pro';
  if (normalized.includes('free')) return 'Free';
  if (normalized === 'plus') return 'Plus';
  if (normalized === 'team') return 'Team';
  return planType.trim();
}
