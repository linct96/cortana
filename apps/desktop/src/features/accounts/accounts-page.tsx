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
import { Button } from '../../components/ui/button';
import { Card, CardContent } from '../../components/ui/card';
import { Separator } from '../../components/ui/separator';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { AddAccountDialog, ConfirmAccountDialog, EditAccountDialog } from './account-dialog';
import { AccountBalance, AccountRow } from './account-list';
import { AntigravityQuotaDialog } from './antigravity-quota-dialog';
import { ResetCreditsDialog } from './reset-credits-dialog';
import type { AuthState } from './types';
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
  const account = useAccountManager(activeProduct);

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
  const authInfo = account.status ? statusStyles[account.status.authState.kind] : null;
  const AuthIcon = authInfo?.icon ?? CircleAlert;
  const statusTone = authInfo?.iconClass ?? 'bg-secondary text-secondary-foreground';

  return (
    <>
      {account.product !== 'antigravity' && (
        <div className="mt-7 w-full px-4 sm:px-8 lg:px-12">
          <Card className="w-full" aria-label={`当前 ${productName(account.product)} 状态`}>
            <CardContent className="flex flex-col gap-4 sm:flex-row sm:items-center">
              <div className="flex min-w-0 flex-1 items-center gap-3">
                <span
                  className={`grid size-10 shrink-0 place-items-center rounded-full ${statusTone}`}
                >
                  <AuthIcon size={19} />
                </span>
                <div className="min-w-0">
                  <span className="mb-1 block text-xs font-medium tracking-wide text-muted-foreground">
                    当前账户
                  </span>
                  <strong className="block truncate text-base font-semibold">
                    {account.activeProfile?.alias ?? '尚未选择账户'}
                  </strong>
                  {!account.activeProfile && (
                    <span className="block truncate text-sm text-muted-foreground">
                      {account.status?.authState.kind === 'missing'
                        ? account.product === 'claude'
                          ? '未检测到 settings.json 中的 Claude 凭据'
                          : '未检测到 auth.json'
                        : '导入当前登录态后即可管理'}
                    </span>
                  )}
                  {account.product === 'claude' && account.status?.authPath && (
                    <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                      {account.status.authPath}
                    </span>
                  )}
                </div>
                {account.status &&
                  account.status.authState.kind !== 'managed' &&
                  account.status.authState.kind !== 'missing' &&
                  (account.product !== 'claude' || account.status.detectedProfile) && (
                    <Button
                      variant="secondary"
                      className="ml-auto shrink-0 text-primary"
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
                account.activeProfile?.accountType === 'oauth' &&
                !account.status?.detectedProfile && (
                  <>
                    <Separator className="sm:h-auto sm:w-px sm:self-stretch" />
                    <AccountBalance
                      profile={account.activeProfile}
                      isRefreshing={
                        account.loading || account.busy === `refresh:${account.activeProfile.id}`
                      }
                      onRefresh={() => void account.refreshAccount(account.activeProfile!)}
                    />
                  </>
                )}
              {account.activeProfile?.accountType === 'relay' &&
                account.activeProfile.apiBaseUrl && (
                  <>
                    <Separator className="sm:h-auto sm:w-px sm:self-stretch" />
                    <div className="flex min-w-0 flex-1 items-center gap-3">
                      <Server className="shrink-0 text-muted-foreground" />
                      <div className="min-w-0">
                        <strong className="block text-sm font-medium">中转站 API</strong>
                        <span className="block truncate text-sm text-muted-foreground">
                          {account.activeProfile.apiBaseUrl}
                        </span>
                      </div>
                    </div>
                  </>
                )}
            </CardContent>
          </Card>
        </div>
      )}

      <section className="mt-9 flex min-h-0 w-full flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          {account.loading ? (
            <EmptyState loading />
          ) : account.status?.profiles.length ? (
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
                    index={index}
                    isBusy={account.busy === `switch:${profile.id}`}
                    isRefreshing={account.busy === `refresh:${profile.id}`}
                    onSwitch={() => void account.switchTo(profile)}
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
          onSubmit={account.submitAdd}
          onClose={() =>
            account.busy === 'oauth'
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
          setAlias={account.setEditingAlias}
          setAuthJson={account.setEditingAuthJson}
          setRelayApiKey={account.setEditingRelayApiKey}
          setRelayApiBaseUrl={account.setEditingRelayApiBaseUrl}
          setShowRelayApiKey={account.setShowEditingRelayApiKey}
          onSubmit={account.saveProfile}
          onClose={account.closeEditor}
        />
      )}
      {account.confirm && (
        <ConfirmAccountDialog
          confirm={account.confirm}
          onClose={() => account.setConfirm(null)}
          onConfirm={() =>
            account.confirm?.kind === 'delete'
              ? void account.deleteProfile(account.confirm.profile)
              : account.confirm && void account.switchTo(account.confirm.profile, true)
          }
        />
      )}
      {account.resetCreditsProfile && (
        <ResetCreditsDialog
          profile={account.resetCreditsProfile}
          credits={account.resetCredits}
          onClose={() => account.setResetCreditsProfile(null)}
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

function EmptyState({ loading = false, onAdd }: { loading?: boolean; onAdd?: () => void }) {
  return (
    <div className="flex min-h-52 flex-col items-center justify-center gap-3 border-y text-sm text-muted-foreground">
      {loading ? (
        <>
          <LoaderCircle size={22} className="animate-spin" /> 正在读取账户
        </>
      ) : (
        <>
          <div className="grid size-11 place-items-center rounded-full bg-secondary text-primary">
            <LogIn size={22} />
          </div>
          <strong className="text-sm font-medium text-foreground">还没有账户档案</strong>
          <Button variant="secondary" className="text-primary" type="button" onClick={onAdd}>
            <Plus data-icon="inline-start" /> 添加账号
          </Button>
        </>
      )}
    </div>
  );
}
