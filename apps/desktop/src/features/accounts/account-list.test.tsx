import { DragDropProvider } from '@dnd-kit/react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { AccountRow } from './account-list';
import type { Profile } from './types';

const profile: Profile = {
  id: 'account-1',
  product: 'grok',
  accountType: 'relay',
  apiBaseUrl: 'https://relay.example/v1',
  upstreamProtocol: 'openaiResponses',
  upstreamAuthMode: 'bearer',
  anthropicMaxTokens: 16_384,
  accountId: 'fingerprint',
  email: '',
  alias: 'Relay',
  planType: '',
  usagePrimary: null,
  usageSecondary: null,
  antigravityQuota: null,
  usageUpdatedAt: null,
  resetCreditsAvailableCount: null,
  needsReauthorization: false,
  isRenewable: true,
  isActive: false,
  lastUsedAt: null,
  updatedAt: 1,
};

function renderRow(modelProfileName?: string, isActive = false) {
  return renderToStaticMarkup(
    <DragDropProvider>
      <AccountRow
        profile={{ ...profile, isActive }}
        modelProfileName={modelProfileName}
        index={0}
        isBusy={false}
        isRefreshing={false}
        isOpeningCli={false}
        onSwitch={vi.fn()}
        onEnabledChange={vi.fn()}
        onOpenCli={vi.fn()}
        onRefresh={vi.fn()}
        onEdit={vi.fn()}
        onViewQuota={vi.fn()}
        onViewResetCredits={vi.fn()}
        onDelete={vi.fn()}
      />
    </DragDropProvider>,
  );
}

describe('AccountRow', () => {
  it('使用切换按钮启用 Grok 中转，并在未关联模型方案时禁用', () => {
    const disabled = renderRow();
    expect(disabled).toContain('aria-label="切换 Relay"');
    expect(disabled).toContain('disabled=""');

    const enabled = renderRow('共享方案');
    expect(enabled).toContain('aria-label="切换 Relay"');
    expect(enabled).not.toContain('disabled=""');
  });

  it('已启用的 Grok 中转显示取消按钮', () => {
    const active = renderRow('共享方案', true);
    expect(active).toContain('aria-label="取消 Relay"');
    expect(active).not.toContain('aria-label="切换 Relay"');
  });
});
