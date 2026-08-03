import type { AccountProduct } from '../../components/app-shell-context';

export type Profile = {
  id: string;
  product: AccountProduct;
  accountType: 'oauth' | 'relay';
  apiBaseUrl: string | null;
  upstreamProtocol: UpstreamProtocol;
  upstreamAuthMode: UpstreamAuthMode;
  anthropicMaxTokens: number;
  accountId: string;
  email: string;
  alias: string;
  planType: string;
  usagePrimary: UsageWindow | null;
  usageSecondary: UsageWindow | null;
  antigravityQuota: AntigravityQuota | null;
  usageUpdatedAt: number | null;
  resetCreditsAvailableCount: number | null;
  needsReauthorization: boolean;
  isRenewable: boolean;
  isActive: boolean;
  lastUsedAt: number | null;
  updatedAt: number;
};

export type UpstreamProtocol = 'openaiResponses' | 'openaiChatCompletions' | 'anthropicMessages';
export type UpstreamAuthMode = 'bearer' | 'xApiKey';

export type CodexGatewayStatus = {
  enabled: boolean;
  available: boolean;
  activeProfileId: string | null;
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

export type ResetCreditConsumeOutcome = 'reset' | 'alreadyRedeemed' | 'nothingToReset' | 'noCredit';

export type ResetCreditConsumeResult = {
  outcome: ResetCreditConsumeOutcome;
  profile: Profile;
  credits: ResetCredits;
};

export function resetCreditOutcomeNotice(outcome: ResetCreditConsumeOutcome): {
  kind: 'success' | 'info';
  message: string;
} {
  switch (outcome) {
    case 'reset':
      return { kind: 'success', message: '重置成功。' };
    case 'alreadyRedeemed':
      return { kind: 'success', message: '重置已完成，额度已刷新。' };
    case 'nothingToReset':
      return { kind: 'info', message: '当前没有可重置的额度窗口，重置卡未消耗。' };
    case 'noCredit':
      return { kind: 'info', message: '已无可用重置卡。' };
  }
}

export async function runResetCreditAttempt(
  lock: { current: boolean },
  attempt: () => Promise<boolean>,
) {
  if (lock.current) return false;
  lock.current = true;
  try {
    return await attempt();
  } finally {
    lock.current = false;
  }
}

export type UsageRefreshResult = {
  profile: Profile;
  refreshed: boolean;
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

export function usageWindowLabel(minutes: number | null) {
  if (minutes === 300) return '5h额度';
  if (minutes === 10_080) return '周额度';
  if (minutes !== null && minutes >= 28 * 1_440 && minutes <= 32 * 1_440) return '月额度';
  return '剩余额度';
}

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

export type PendingConfirm =
  | { kind: 'force-switch' | 'delete'; profile: Profile }
  | { kind: 'force-grok-relay'; profile: Profile; enabled: boolean }
  | { kind: 'force-grok-edit'; profile: Profile }
  | { kind: 'enable-gateway'; profile: Profile; action: 'switch' | 'open-cli' }
  | null;

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
