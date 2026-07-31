import { move } from '@dnd-kit/helpers';
import { DragDropProvider } from '@dnd-kit/react';
import {
  CircleAlert,
  CircleCheck,
  LoaderCircle,
  LogIn,
  Plus,
  RefreshCw,
  Server,
} from 'lucide-react';
import type { ComponentType } from 'react';
import { productName, useAppShell } from '../../components/app-shell-context';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Card, CardContent } from '../../components/ui/card';
import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '../../components/ui/empty';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { AddAccountDialog, ConfirmAccountDialog, EditAccountDialog } from './account-dialog';
import { AccountBalance, AccountRow } from './account-list';
import { AntigravityQuotaDialog } from './antigravity-quota-dialog';
import { ResetCreditsDialog } from './reset-credits-dialog';
import { planLabel, type AccountProduct, type AuthState } from './types';
import { useAccountManager } from './use-account-manager';

const statusStyles: Record<
  AuthState['kind'],
  { icon: ComponentType<{ size?: number }>; iconClass: string }
> = {
  managed: { icon: CircleCheck, iconClass: 'bg-primary/10 text-primary' },
  unmanaged: { icon: CircleAlert, iconClass: 'bg-secondary text-secondary-foreground' },
  missing: { icon: CircleAlert, iconClass: 'bg-secondary text-secondary-foreground' },
};

export default function AccountsPage() {
  const { activeProduct } = useAppShell();
  return <ProductAccountsPage key={activeProduct} product={activeProduct} />;
}

function ProductAccountsPage({ product }: { product: AccountProduct }) {
  const account = useAccountManager(product);

  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title="账号"
        actions={
          <>
            <Tooltip>
              <TooltipTrigger render={<span className="inline-flex" />}>
                <Button
                  variant="ghost"
                  size="icon"
                  type="button"
                  onClick={() => void account.refreshAllAccounts()}
                  disabled={account.loading || account.busy === 'refresh:all'}
                >
                  <RefreshCw
                    size={18}
                    className={
                      account.loading || account.busy === 'refresh:all' ? 'animate-spin' : ''
                    }
                  />
                  <span className="sr-only">刷新</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {account.product === 'claude' ? '更新全部登录令牌' : '刷新全部'}
              </TooltipContent>
            </Tooltip>
            <Button
              type="button"
              onClick={() => account.setAddOpen(true)}
              disabled={account.busy === 'oauth' || account.busy === 'import'}
            >
              <Plus data-icon="inline-start" /> 添加账号
            </Button>
          </>
        }
      />
      <AccountContent account={account} />
    </PageShell>
  );
}

function AccountContent({ account }: { account: ReturnType<typeof useAccountManager> }) {
  if (!account.status) return null;

  const enabledGrokRelays =
    account.product === 'grok'
      ? account.status.profiles.filter(
          (profile) => profile.accountType === 'relay' && profile.isActive,
        )
      : [];
  const authInfo = account.status ? statusStyles[account.status.authState.kind] : null;
  const AuthIcon = authInfo?.icon ?? CircleAlert;
  const statusTone = authInfo?.iconClass ?? 'bg-secondary text-secondary-foreground';

  return (
    <>
      <div className="mt-7 w-full px-4 sm:px-8 lg:px-12">
        <Card size="sm" className="w-full" aria-label={`当前 ${productName(account.product)} 状态`}>
          <CardContent className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] items-center gap-(--card-spacing)">
            <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2 xl:grid-cols-[auto_minmax(0,1fr)_auto]">
              <span
                className={`grid size-10 shrink-0 place-items-center rounded-full ${statusTone}`}
              >
                <AuthIcon size={19} />
              </span>
              <div className="min-w-0">
                <span className="mb-1 block text-xs font-medium tracking-wide text-muted-foreground">
                  当前账户
                </span>
                <div className="flex min-w-0 items-center gap-2">
                  <strong className="truncate text-base font-semibold">
                    {enabledGrokRelays.length
                      ? `已启用 ${enabledGrokRelays.length} 个中转账号`
                      : (account.activeProfile?.alias ?? '尚未选择账户')}
                  </strong>
                  {!enabledGrokRelays.length && account.activeProfile?.planType && (
                    <Badge variant="outline">{planLabel(account.activeProfile.planType)}</Badge>
                  )}
                </div>
                {enabledGrokRelays.length > 0 && (
                  <span className="block truncate text-sm text-muted-foreground">
                    {enabledGrokRelays.map((profile) => profile.alias).join('、')}
                  </span>
                )}
                {!account.activeProfile && (
                  <span className="block truncate text-sm text-muted-foreground">
                    {account.product === 'antigravity'
                      ? account.status?.authState.message
                      : account.status?.authState.kind === 'missing'
                        ? account.product === 'claude'
                          ? '未检测到 settings.json 中的 Claude 凭据'
                          : '未检测到 auth.json'
                        : '导入当前登录态后即可管理'}
                  </span>
                )}
                {(account.product === 'claude' || account.product === 'antigravity') &&
                  account.status?.authPath && (
                    <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                      {account.status.authPath}
                    </span>
                  )}
              </div>
              {account.status &&
                account.status.authState.kind !== 'managed' &&
                account.status.authState.kind !== 'missing' &&
                ((account.product !== 'claude' && account.product !== 'antigravity') ||
                  account.status.detectedProfile) && (
                  <Button
                    variant="secondary"
                    className="col-start-2 justify-self-start text-primary xl:col-start-3 xl:row-start-1"
                    type="button"
                    onClick={() => void account.importCurrent()}
                    disabled={account.busy === 'import'}
                  >
                    {account.busy === 'import' ? (
                      <LoaderCircle className="animate-spin" />
                    ) : (
                      <LogIn />
                    )}
                    {account.status.detectedProfile ? '同步该账号' : '导入当前状态'}
                  </Button>
                )}
            </div>
            {account.product !== 'claude' &&
              account.product !== 'antigravity' &&
              account.activeProfile?.accountType === 'oauth' &&
              !account.status?.detectedProfile && (
                <div className="min-w-0 border-l pl-(--card-spacing)">
                  <AccountBalance
                    profile={account.activeProfile}
                    isRefreshing={
                      account.loading || account.busy === `refresh:${account.activeProfile.id}`
                    }
                    onRefresh={() => void account.refreshAccount(account.activeProfile!)}
                  />
                </div>
              )}
            {account.product !== 'grok' &&
              account.product !== 'antigravity' &&
              account.activeProfile?.accountType === 'relay' &&
              account.activeProfile.apiBaseUrl && (
                <div className="min-w-0 border-l pl-(--card-spacing)">
                  <div className="flex min-w-0 flex-1 items-center gap-3">
                    <Server className="shrink-0 text-muted-foreground" />
                    <div className="min-w-0">
                      <strong className="block text-sm font-medium">中转站 API</strong>
                      <span className="block truncate text-sm text-muted-foreground">
                        {account.activeProfile.apiBaseUrl}
                      </span>
                    </div>
                  </div>
                </div>
              )}
          </CardContent>
        </Card>
      </div>

      <section className="mt-9 flex min-h-0 w-full flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          {account.status.profiles.length ? (
            <DragDropProvider
              onDragEnd={(event) =>
                void account.reorderProfiles(move(account.status!.profiles, event))
              }
            >
              <div className="flex w-full flex-col gap-3 px-4 pt-2 pb-6 sm:px-8 lg:px-12">
                {account.status.profiles.map((profile, index) => (
                  <AccountRow
                    key={profile.id}
                    profile={profile}
                    modelProfileName={
                      account.modelStatus?.profiles.find((modelProfile) =>
                        modelProfile.assignments.some(
                          (assignment) => assignment.accountId === profile.id,
                        ),
                      )?.name
                    }
                    index={index}
                    isBusy={
                      account.busy === `switch:${profile.id}` ||
                      account.busy === `relay:${profile.id}`
                    }
                    isRefreshing={account.busy === `refresh:${profile.id}`}
                    isOpeningCli={account.busy === `open-cli:${profile.id}`}
                    onSwitch={() => void account.switchTo(profile)}
                    onEnabledChange={(enabled) =>
                      void account.setGrokRelayEnabled(profile, enabled)
                    }
                    onOpenCli={() => void account.openCli(profile)}
                    onRefresh={() => void account.refreshAccount(profile)}
                    onEdit={() => void account.openEditor(profile)}
                    onViewQuota={() => account.setQuotaProfileId(profile.id)}
                    onViewResetCredits={() => void account.viewResetCredits(profile)}
                    onDelete={() => account.setConfirm({ kind: 'delete', profile })}
                  />
                ))}
              </div>
            </DragDropProvider>
          ) : (
            <EmptyState onAdd={() => account.setAddOpen(true)} />
          )}
        </div>
      </section>

      {account.addOpen && (
        <AddAccountDialog
          {...account}
          onGenerateOAuth={account.generateOAuthLink}
          onOpenOAuth={() => void account.openOAuthLink()}
          onSubmit={account.submitAdd}
          onClose={() =>
            account.busy === 'oauth' ||
            account.busy?.startsWith('oauth:') ||
            Boolean(account.oauthUrl)
              ? void account.cancelOAuth()
              : account.busy !== 'auth-json' &&
                account.busy !== 'relay' &&
                account.busy !== 'import' &&
                account.closeAddDialog()
          }
        />
      )}
      {account.editing && (
        <EditAccountDialog
          editing={account.editing}
          busy={account.busy}
          alias={account.editingAlias}
          authJson={account.editingAuthJson}
          relayApiKey={account.editingRelayApiKey}
          relayApiBaseUrl={account.editingRelayApiBaseUrl}
          showRelayApiKey={account.showEditingRelayApiKey}
          modelStatus={account.modelStatus}
          customModelEnabled={account.customModelEnabled}
          modelProfileId={account.modelProfileId}
          defaultModelId={account.defaultModelId}
          setAlias={account.setEditingAlias}
          setAuthJson={account.setEditingAuthJson}
          setRelayApiKey={account.setEditingRelayApiKey}
          setRelayApiBaseUrl={account.setEditingRelayApiBaseUrl}
          setShowRelayApiKey={account.setShowEditingRelayApiKey}
          setCustomModelEnabled={account.setCustomModelEnabled}
          setModelProfileId={account.setModelProfileId}
          setDefaultModelId={account.setDefaultModelId}
          onSubmit={account.saveProfile}
          onClose={account.closeEditor}
        />
      )}
      {account.confirm && (
        <ConfirmAccountDialog
          confirm={account.confirm}
          busy={
            account.busy ===
            `${
              account.confirm.kind === 'delete'
                ? 'delete'
                : account.confirm.kind === 'force-grok-relay'
                  ? 'relay'
                  : account.confirm.kind === 'force-grok-edit'
                    ? 'edit'
                    : 'switch'
            }:${account.confirm.profile.id}`
          }
          onClose={() => account.setConfirm(null)}
          onConfirm={() =>
            account.confirm?.kind === 'delete'
              ? void account.deleteProfile(account.confirm.profile)
              : account.confirm?.kind === 'force-grok-relay'
                ? void account.setGrokRelayEnabled(
                    account.confirm.profile,
                    account.confirm.enabled,
                    true,
                  )
                : account.confirm?.kind === 'force-grok-edit'
                  ? void account.saveProfile(undefined, true)
                  : account.confirm && void account.switchTo(account.confirm.profile, true)
          }
        />
      )}
      {account.resetCreditsProfile && (
        <ResetCreditsDialog
          profile={account.resetCreditsProfile}
          credits={account.resetCredits}
          busyCreditId={
            account.busy?.startsWith('consume-reset-credit:')
              ? account.busy.slice('consume-reset-credit:'.length)
              : null
          }
          onConsume={account.consumeResetCredit}
          onClose={() => {
            account.setResetCreditsProfile(null);
          }}
        />
      )}
      {account.quotaProfile && (
        <AntigravityQuotaDialog
          profile={account.quotaProfile}
          isRefreshing={account.busy === `refresh:${account.quotaProfile.id}`}
          onRefresh={() => void account.refreshAccount(account.quotaProfile!)}
          onClose={() => account.setQuotaProfileId(null)}
        />
      )}
    </>
  );
}

function EmptyState({ onAdd }: { onAdd?: () => void }) {
  return (
    <Empty className="min-h-52 rounded-none border-y">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <LogIn />
        </EmptyMedia>
        <EmptyTitle>还没有账户档案</EmptyTitle>
      </EmptyHeader>
      <EmptyContent>
        <Button variant="secondary" className="text-primary" type="button" onClick={onAdd}>
          <Plus data-icon="inline-start" /> 添加账号
        </Button>
      </EmptyContent>
    </Empty>
  );
}
