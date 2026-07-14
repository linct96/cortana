import { LoaderCircle } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import type { Profile, ResetCredit, ResetCredits } from './types';

export function ResetCreditsDialog({
  profile,
  credits,
  onClose,
}: {
  profile: Profile;
  credits: ResetCredits | null;
  onClose: () => void;
}) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-xl" initialFocus={false}>
        <DialogHeader>
          <DialogTitle>{profile.alias} 的重置卡</DialogTitle>
        </DialogHeader>
        {!credits ? (
          <div className="flex min-h-32 items-center justify-center text-muted-foreground">
            <LoaderCircle className="animate-spin" />
            <span className="sr-only">正在查询重置卡</span>
          </div>
        ) : (
          <div className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto">
            <strong className="text-sm font-medium">{credits.availableCount} 次可用重置</strong>
            {credits.credits.length ? (
              <div className="divide-y rounded-md border">
                {credits.credits.map((credit) => (
                  <ResetCreditRow key={credit.id} credit={credit} />
                ))}
              </div>
            ) : (
              <span className="text-sm text-muted-foreground">暂无重置卡</span>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function ResetCreditRow({ credit }: { credit: ResetCredit }) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-x-3 gap-y-1 p-3 text-sm">
      <span className="text-muted-foreground">状态</span>
      <span>{statusLabel(credit.status)}</span>
      <span className="text-muted-foreground">获得时间</span>
      <span>{formatDateTime(credit.grantedAt)}</span>
      <span className="text-muted-foreground">过期时间</span>
      <span>{formatDateTime(credit.expiresAt)}</span>
      <span className="text-muted-foreground">ID</span>
      <code className="min-w-0 break-all text-xs">{credit.id}</code>
    </div>
  );
}

function statusLabel(status: string) {
  return { available: '可用', redeemed: '已兑换', expired: '已过期' }[status] ?? status;
}

function formatDateTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('zh-CN', { hourCycle: 'h23' });
}
