import { useSortable } from '@dnd-kit/react/sortable';
import {
  ArrowLeftRight,
  Ellipsis,
  Gauge,
  GripVertical,
  LoaderCircle,
  Pencil,
  RefreshCw,
  SquareTerminal,
  Tickets,
  Trash2,
  X,
} from 'lucide-react';
import { Fragment } from 'react';
import antigravityIcon from '../../assets/antigravity.svg';
import chatGptIcon from '../../assets/chatgpt.svg';
import claudeIcon from '../../assets/claude.svg';
import grokIcon from '../../assets/grok.svg';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import { Progress, ProgressLabel, ProgressValue } from '../../components/ui/progress';
import { Separator } from '../../components/ui/separator';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { cn, formatResetTime } from '../../utils';
import { planLabel, type Profile, type UsageWindow, usageWindowLabel } from './types';

export function AccountRow({
  profile,
  modelProfileName,
  index,
  isBusy,
  isRefreshing,
  isOpeningCli,
  onSwitch,
  onEnabledChange,
  onOpenCli,
  onRefresh,
  onEdit,
  onViewQuota,
  onViewResetCredits,
  onDelete,
}: {
  profile: Profile;
  modelProfileName?: string;
  index: number;
  isBusy: boolean;
  isRefreshing: boolean;
  isOpeningCli: boolean;
  onSwitch: () => void;
  onEnabledChange: (enabled: boolean) => void;
  onOpenCli: () => void;
  onRefresh: () => void;
  onEdit: () => void;
  onViewQuota: () => void;
  onViewResetCredits: () => void;
  onDelete: () => void;
}) {
  const sortable = useSortable({
    id: profile.id,
    index,
    transition: {
      duration: 220,
      easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
      idle: true,
    },
  });

  return (
    <article
      ref={sortable.ref}
      className={cn(
        'group flex h-[68px] items-center gap-3 rounded-md border border-border bg-card px-2 py-3 transition-[border-color,box-shadow,opacity,background-color] duration-200 hover:border-primary/30',
        profile.isActive && 'bg-primary/5',
        sortable.isDragging && 'opacity-95 shadow-lg ring-1 ring-primary/20',
        sortable.isDropping && 'shadow-md',
      )}
    >
      <button
        ref={sortable.handleRef}
        className="touch-none cursor-grab text-muted-foreground active:cursor-grabbing"
        type="button"
        aria-label={`拖动 ${profile.alias} 排序`}
      >
        <GripVertical size={18} />
      </button>
      <div className="grid size-9 shrink-0 place-items-center rounded-full bg-secondary text-sm font-semibold text-primary">
        {profile.accountType === 'oauth' ? (
          <img
            src={
              profile.product === 'antigravity'
                ? antigravityIcon
                : profile.product === 'claude'
                  ? claudeIcon
                  : profile.product === 'grok'
                    ? grokIcon
                    : chatGptIcon
            }
            alt=""
            className="size-5"
          />
        ) : (
          (profile.alias || profile.email || '?').slice(0, 1).toUpperCase()
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-0.5 flex items-center gap-2">
          <strong className="truncate text-sm font-medium">{profile.alias}</strong>
          {profile.needsReauthorization && <Badge variant="destructive">已过期</Badge>}
          {profile.planType && <Badge variant="outline">{planLabel(profile.planType)}</Badge>}
          {profile.isActive && (
            <Badge>
              {profile.product === 'grok' && profile.accountType === 'relay' ? '已启用' : '使用中'}
            </Badge>
          )}
          {modelProfileName && <Badge variant="outline">{modelProfileName}</Badge>}
        </div>
        <AccountMeta profile={profile} onViewQuota={onViewQuota} />
      </div>
      <div className="pointer-events-none flex min-w-10 justify-end gap-1 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 has-focus-visible:pointer-events-auto has-focus-visible:opacity-100 has-data-popup-open:pointer-events-auto has-data-popup-open:opacity-100">
        {!profile.isActive && (
          <Tooltip>
            <TooltipTrigger render={<span className="inline-flex" />}>
              <Button
                size="icon"
                type="button"
                aria-label={`切换 ${profile.alias}`}
                onClick={onSwitch}
                disabled={
                  isBusy ||
                  (profile.product === 'grok' &&
                    profile.accountType === 'relay' &&
                    !modelProfileName)
                }
              >
                {isBusy ? <LoaderCircle className="animate-spin" /> : <ArrowLeftRight />}
                <span className="sr-only">切换</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {profile.product === 'grok' && profile.accountType === 'relay' && !modelProfileName
                ? '请先关联模型方案'
                : '切换'}
            </TooltipContent>
          </Tooltip>
        )}
        {profile.isActive && profile.product === 'grok' && profile.accountType === 'relay' && (
          <Tooltip>
            <TooltipTrigger render={<span className="inline-flex" />}>
              <Button
                variant="outline"
                size="icon"
                type="button"
                aria-label={`取消 ${profile.alias}`}
                onClick={() => onEnabledChange(false)}
                disabled={isBusy}
              >
                {isBusy ? <LoaderCircle className="animate-spin" /> : <X />}
                <span className="sr-only">取消</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>取消</TooltipContent>
          </Tooltip>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon" type="button" />}>
            <Ellipsis />
            <span className="sr-only">更多</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              {profile.product === 'codex' && (
                <DropdownMenuItem onClick={onOpenCli} disabled={isOpeningCli}>
                  {isOpeningCli ? <LoaderCircle className="animate-spin" /> : <SquareTerminal />}
                  打开终端
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={onEdit}>
                <Pencil /> 编辑
              </DropdownMenuItem>
              {profile.accountType === 'oauth' && (
                <>
                  <DropdownMenuItem
                    onClick={onRefresh}
                    disabled={
                      isRefreshing || (profile.product === 'claude' && !profile.isRenewable)
                    }
                  >
                    <RefreshCw className={isRefreshing ? 'animate-spin' : ''} />
                    {profile.product === 'claude' ? '更新登录令牌' : '刷新'}
                  </DropdownMenuItem>
                  {profile.product === 'antigravity' && (
                    <DropdownMenuItem onClick={onViewQuota}>
                      <Gauge /> 额度详情
                    </DropdownMenuItem>
                  )}
                  {profile.product === 'codex' &&
                  profile.planType.trim() &&
                  profile.planType.trim().toLowerCase() !== 'free' ? (
                    <DropdownMenuItem onClick={onViewResetCredits}>
                      <Tickets /> 重置卡
                    </DropdownMenuItem>
                  ) : null}
                </>
              )}
              <DropdownMenuItem variant="destructive" onClick={onDelete}>
                <Trash2 /> 删除
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </article>
  );
}

export function AccountBalance({
  profile,
  isRefreshing,
  onRefresh,
}: {
  profile: Profile;
  isRefreshing: boolean;
  onRefresh: () => void;
}) {
  const windows = [profile.usagePrimary, profile.usageSecondary].filter(
    (window): window is UsageWindow => Boolean(window),
  );
  const balanceTitle =
    [...new Set(windows.map((window) => usageWindowLabel(window.windowMinutes)))].join(' / ') ||
    '剩余额度';
  return (
    <div className="min-w-0 flex-1">
      <div className="mb-3 flex items-center gap-2">
        <strong className="text-sm font-medium">{balanceTitle}</strong>
        <Tooltip>
          <TooltipTrigger render={<span className="ml-auto inline-flex" />}>
            <Button
              variant="ghost"
              size="icon-xs"
              type="button"
              onClick={onRefresh}
              disabled={isRefreshing}
            >
              <RefreshCw className={isRefreshing ? 'animate-spin' : ''} />
              <span className="sr-only">刷新当前账户信息</span>
            </Button>
          </TooltipTrigger>
          <TooltipContent>刷新当前账户信息</TooltipContent>
        </Tooltip>
      </div>
      <UsageProgressList
        valueFirst
        items={windows.map((window, index) => ({
          key: String(window.windowMinutes ?? index),
          label: window.resetsAt ? formatResetTime(window.resetsAt) : '剩余额度',
          remainingPercent: remainingPercent(window),
          resetsAt: null,
        }))}
        emptyText={
          profile.product === 'grok' && profile.usageUpdatedAt
            ? '官方未返回额度百分比'
            : '额度未查询'
        }
      />
    </div>
  );
}

export function UsageProgressList({
  items,
  emptyText = '额度未查询',
  valueFirst = false,
}: {
  items: {
    key: string;
    label: string;
    remainingPercent: number;
    resetsAt: number | null;
  }[];
  emptyText?: string;
  valueFirst?: boolean;
}) {
  return items.length ? (
    <div className="flex flex-col gap-3">
      {items.map((item) => (
        <Progress
          key={item.key}
          value={item.remainingPercent}
          className={cn(
            'gap-1.5',
            item.remainingPercent <= 10 && '[&_[data-slot=progress-indicator]]:bg-destructive',
            valueFirst && '[&_[data-slot=progress-track]]:order-first',
          )}
        >
          <ProgressLabel
            className={cn(
              valueFirst && 'order-2 ml-auto font-normal text-muted-foreground tabular-nums',
            )}
          >
            {item.label}
            {item.resetsAt ? ` · ${formatResetTime(item.resetsAt)}` : ''}
          </ProgressLabel>
          <ProgressValue className={cn(valueFirst && 'order-1 ml-0')}>
            {valueFirst ? (formattedValue) => `${formattedValue}剩余` : undefined}
          </ProgressValue>
        </Progress>
      ))}
    </div>
  ) : (
    <span className="text-sm text-muted-foreground">{emptyText}</span>
  );
}

function AccountMeta({ profile, onViewQuota }: { profile: Profile; onViewQuota: () => void }) {
  if (profile.product === 'antigravity') {
    const quota = profile.antigravityQuota;
    const summaries = quota
      ? quota.groups
          .flatMap((group) =>
            group.buckets.map((bucket) => ({
              key: `${group.displayName}-${bucket.bucketId}`,
              label: `${group.displayName} ${quotaWindowLabel(bucket.window)} ${bucket.remainingPercent}%`,
              remaining: bucket.remainingPercent,
            })),
          )
          .concat(
            quota.groups.length
              ? []
              : quota.models.map((model) => ({
                  key: model.modelId,
                  label: `${model.displayName} ${model.remainingPercent}%`,
                  remaining: model.remainingPercent,
                })),
          )
          .sort((left, right) => left.remaining - right.remaining)
          .slice(0, 2)
      : [];
    return (
      <div className="mt-1.5 flex min-w-0 items-center text-xs text-muted-foreground">
        {profile.email && <span className="min-w-0 truncate">{profile.email}</span>}
        {quota?.forbidden ? (
          <MetaSeparatorItem label="无权查询额度" destructive onClick={onViewQuota} />
        ) : summaries.length ? (
          summaries.map((item) => (
            <MetaSeparatorItem
              key={item.key}
              label={item.label}
              destructive={item.remaining <= 10}
              onClick={onViewQuota}
            />
          ))
        ) : (
          <MetaSeparatorItem label="额度未查询" onClick={onViewQuota} />
        )}
      </div>
    );
  }
  const items: { key: string; label: string; destructive?: boolean }[] = [];
  if (profile.accountType === 'relay') {
    if (profile.apiBaseUrl) items.push({ key: 'api', label: profile.apiBaseUrl });
    if (profile.product === 'codex') {
      items.push({
        key: 'protocol',
        label:
          profile.upstreamProtocol === 'openaiResponses'
            ? 'Responses'
            : profile.upstreamProtocol === 'openaiChatCompletions'
              ? 'Chat Completions'
              : 'Anthropic Messages',
      });
    }
  } else if (profile.product === 'claude') {
    if (profile.email) {
      items.push({ key: 'email', label: profile.email });
    }
    if (!profile.isRenewable) {
      items.push({ key: 'reauthorize', label: '需重新授权' });
    }
  } else {
    if (profile.product === 'grok' && profile.email) {
      items.push({ key: 'email', label: profile.email });
    }
    [profile.usagePrimary, profile.usageSecondary].forEach((window, index) => {
      if (!window) return;
      const remaining = remainingPercent(window);
      items.push({
        key: `usage-${index}`,
        label: `${remaining}% 剩余`,
        destructive: remaining <= 10,
      });
      if (window.resetsAt !== null) {
        items.push({ key: `reset-${index}`, label: formatResetTime(window.resetsAt) });
      }
    });
    if (profile.resetCreditsAvailableCount !== null) {
      items.push({
        key: 'reset-credits',
        label: `${profile.resetCreditsAvailableCount}次可用重置`,
      });
    }
    if (!profile.usageUpdatedAt) {
      items.push({ key: 'empty', label: '额度未查询' });
    } else if (profile.product === 'grok' && !profile.usagePrimary) {
      items.push({ key: 'unavailable', label: '官方未返回额度百分比' });
    }
  }

  return (
    <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-y-1 text-xs text-muted-foreground">
      {items.map((item, index) => (
        <Fragment key={item.key}>
          {index > 0 && (
            <Separator
              orientation="vertical"
              className="mx-2 h-3 w-px self-center bg-muted-foreground/30"
            />
          )}
          <span className={cn('min-w-0 truncate', item.destructive && 'text-destructive')}>
            {item.label}
          </span>
        </Fragment>
      ))}
    </div>
  );
}

function MetaSeparatorItem({
  label,
  destructive = false,
  onClick,
}: {
  label: string;
  destructive?: boolean;
  onClick: () => void;
}) {
  return (
    <>
      <Separator
        orientation="vertical"
        className="mx-2 h-3 w-px shrink-0 self-center bg-muted-foreground/30"
      />
      <button
        type="button"
        className={cn(
          'min-w-0 truncate text-left hover:text-foreground',
          destructive && 'text-destructive',
        )}
        onClick={onClick}
      >
        {label}
      </button>
    </>
  );
}

function remainingPercent(window: UsageWindow) {
  return Math.round(Math.max(0, Math.min(100, 100 - window.usedPercent)));
}

function quotaWindowLabel(window: string) {
  if (window.toLowerCase() === 'weekly') return '周';
  if (window.toLowerCase() === '5h') return '5 小时';
  return window;
}
