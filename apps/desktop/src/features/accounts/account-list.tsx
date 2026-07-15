import { useSortable } from '@dnd-kit/react/sortable';
import {
  ArrowLeftRight,
  Ellipsis,
  GripVertical,
  LoaderCircle,
  Pencil,
  RefreshCw,
  Tickets,
  Trash2,
} from 'lucide-react';
import { Fragment } from 'react';
import chatGptIcon from '../../assets/chatgpt.svg';
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
import type { Profile, UsageWindow } from './types';

export function AccountRow({
  profile,
  index,
  isBusy,
  isRefreshing,
  onSwitch,
  onRefresh,
  onEdit,
  onViewResetCredits,
  onDelete,
}: {
  profile: Profile;
  index: number;
  isBusy: boolean;
  isRefreshing: boolean;
  onSwitch: () => void;
  onRefresh: () => void;
  onEdit: () => void;
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
          <img src={chatGptIcon} alt="" className="size-5" />
        ) : (
          (profile.alias || profile.email || '?').slice(0, 1).toUpperCase()
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-0.5 flex items-center gap-2">
          <strong className="truncate text-sm font-medium">{profile.alias}</strong>
          {profile.planType && <Badge variant="outline">{planLabel(profile.planType)}</Badge>}
          {profile.isActive && <Badge>使用中</Badge>}
        </div>
        <AccountMeta profile={profile} />
      </div>
      <div className="pointer-events-none flex min-w-25 justify-end gap-1 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 has-[:focus-visible]:pointer-events-auto has-[:focus-visible]:opacity-100 has-[[data-popup-open]]:pointer-events-auto has-[[data-popup-open]]:opacity-100">
        {!profile.isActive && (
          <Tooltip>
            <TooltipTrigger render={<span className="inline-flex" />}>
              <Button size="icon" type="button" onClick={onSwitch} disabled={isBusy}>
                {isBusy ? <LoaderCircle className="animate-spin" /> : <ArrowLeftRight />}
                <span className="sr-only">切换</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>切换</TooltipContent>
          </Tooltip>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon" type="button" />}>
            <Ellipsis />
            <span className="sr-only">更多</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              <DropdownMenuItem onClick={onEdit}>
                <Pencil /> 编辑
              </DropdownMenuItem>
              {profile.accountType === 'oauth' && (
                <>
                  <DropdownMenuItem onClick={onRefresh} disabled={isRefreshing}>
                    <RefreshCw className={isRefreshing ? 'animate-spin' : ''} /> 刷新
                  </DropdownMenuItem>
                  {profile.planType.trim() && profile.planType.trim().toLowerCase() !== 'free' ? (
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
  return (
    <div className="min-w-0 flex-1">
      <div className="mb-3 flex items-center gap-2">
        <strong className="text-sm font-medium">剩余额度</strong>
        {profile.planType && <Badge variant="outline">{planLabel(profile.planType)}</Badge>}
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
      {windows.length ? (
        <div className="flex flex-col gap-3">
          {windows.map((window, index) => {
            const remaining = remainingPercent(window);
            return (
              <Progress
                key={window.windowMinutes ?? index}
                value={remaining}
                className={
                  remaining <= 10
                    ? 'gap-1.5 [&_[data-slot=progress-indicator]]:bg-destructive'
                    : 'gap-1.5'
                }
              >
                <ProgressLabel>
                  {window.resetsAt ? `${formatResetTime(window.resetsAt)}重置` : '剩余额度'}
                </ProgressLabel>
                <ProgressValue />
              </Progress>
            );
          })}
        </div>
      ) : (
        <span className="text-sm text-muted-foreground">额度未查询</span>
      )}
    </div>
  );
}

function AccountMeta({ profile }: { profile: Profile }) {
  const items: { key: string; label: string; destructive?: boolean }[] = [];
  if (profile.accountType === 'relay') {
    if (profile.apiBaseUrl) items.push({ key: 'api', label: profile.apiBaseUrl });
  } else {
    [profile.usagePrimary, profile.usageSecondary].forEach((window, index) => {
      if (!window) return;
      const remaining = remainingPercent(window);
      items.push({
        key: `usage-${index}`,
        label: `${remaining}% 剩余`,
        destructive: remaining <= 10,
      });
      if (window.resetsAt !== null) {
        items.push({ key: `reset-${index}`, label: `${formatResetTime(window.resetsAt)}重置` });
      }
    });
    if (profile.resetCreditsAvailableCount !== null) {
      items.push({
        key: 'reset-credits',
        label: `${profile.resetCreditsAvailableCount}次可用重置`,
      });
    }
    if (!profile.usageUpdatedAt) items.push({ key: 'empty', label: '额度未查询' });
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

function remainingPercent(window: UsageWindow) {
  return Math.round(Math.max(0, Math.min(100, 100 - window.usedPercent)));
}

function planLabel(planType: string) {
  const normalized = planType.trim().toLowerCase();
  if (normalized === 'free') return 'Free';
  if (normalized === 'plus') return 'Plus';
  if (normalized === 'pro') return 'Pro';
  if (normalized === 'team') return 'Team';
  return normalized;
}
