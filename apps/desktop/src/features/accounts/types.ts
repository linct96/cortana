export type Profile = {
  id: string;
  accountType: 'oauth' | 'relay';
  apiBaseUrl: string | null;
  accountId: string;
  email: string;
  alias: string;
  planType: string;
  usagePrimary: UsageWindow | null;
  usageSecondary: UsageWindow | null;
  creditsBalance: string | null;
  creditsUnlimited: boolean;
  usageUpdatedAt: number | null;
  isActive: boolean;
  lastUsedAt: number | null;
  updatedAt: number;
};

export type UsageWindow = {
  usedPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
};

export type AuthState = {
  kind: 'managed' | 'external' | 'unmanaged' | 'missing';
  message: string;
};

export type AppStatus = {
  profiles: Profile[];
  detectedProfile: Profile | null;
  activeProfileId: string | null;
  authPath: string;
  authState: AuthState;
  autostartEnabled: boolean;
};

export type OAuthProgress = {
  stage: 'browser_opening' | 'waiting' | 'exchanging' | 'success' | 'error';
  message: string;
  profile: Profile | null;
};

export type PendingConfirm = {
  kind: 'force-switch' | 'delete';
  profile: Profile;
} | null;

export type AddMode = 'browser' | 'paste' | 'relay';
