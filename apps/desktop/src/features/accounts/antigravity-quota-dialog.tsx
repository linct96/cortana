import { LoaderCircle, RefreshCw } from 'lucide-react';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Separator } from '../../components/ui/separator';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { UsageProgressList } from './account-list';
import { planLabel, type Profile } from './types';

export function AntigravityQuotaDialog({
  profile,
  isRefreshing,
  onRefresh,
  onClose,
}: {
  profile: Profile;
  isRefreshing: boolean;
  onRefresh: () => void;
  onClose: () => void;
}) {
  const quota = profile.antigravityQuota;
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl" initialFocus={false}>
        <DialogHeader>
          <DialogTitle>{profile.alias} 的额度</DialogTitle>
        </DialogHeader>
        <div className="flex min-w-0 items-center gap-2">
          {profile.planType && <Badge variant="outline">{planLabel(profile.planType)}</Badge>}
          {profile.usageUpdatedAt && (
            <span className="truncate text-xs text-muted-foreground">
              更新于 {formatUpdatedAt(profile.usageUpdatedAt)}
            </span>
          )}
          <Tooltip>
            <TooltipTrigger render={<span className="ml-auto inline-flex" />}>
              <Button
                variant="ghost"
                size="icon-xs"
                type="button"
                onClick={onRefresh}
                disabled={isRefreshing}
              >
                {isRefreshing ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
                <span className="sr-only">刷新账户信息</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>刷新账户信息</TooltipContent>
          </Tooltip>
        </div>
        <Separator />
        <div className="flex max-h-[65vh] flex-col gap-6 overflow-y-auto pr-1">
          {quota?.forbidden ? (
            <div className="flex min-h-32 items-center justify-center text-sm text-muted-foreground">
              当前授权无权查询额度，请重新授权。
            </div>
          ) : quota ? (
            <>
              {quota.groups.map((group) => (
                <section key={group.displayName} className="flex flex-col gap-3">
                  <strong className="text-sm font-medium">{group.displayName}</strong>
                  <UsageProgressList
                    items={group.buckets.map((bucket) => ({
                      key: bucket.bucketId,
                      label: bucket.displayName || quotaWindowLabel(bucket.window),
                      remainingPercent: bucket.remainingPercent,
                      resetsAt: bucket.resetsAt,
                    }))}
                  />
                </section>
              ))}
              <section className="flex flex-col gap-3">
                <strong className="text-sm font-medium">模型额度</strong>
                <UsageProgressList
                  items={quota.models.map((model) => ({
                    key: model.modelId,
                    label: model.displayName,
                    remainingPercent: model.remainingPercent,
                    resetsAt: model.resetsAt,
                  }))}
                />
              </section>
            </>
          ) : (
            <div className="flex min-h-32 items-center justify-center text-sm text-muted-foreground">
              额度未查询
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function quotaWindowLabel(window: string) {
  if (window.toLowerCase() === 'weekly') return '周额度';
  if (window.toLowerCase() === '5h') return '5 小时额度';
  return window || '剩余额度';
}

function formatUpdatedAt(value: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(value);
}
