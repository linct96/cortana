import dayjs from 'dayjs';
import { LoaderCircle, Tickets } from 'lucide-react';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle } from '../../components/ui/empty';
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
        {credits && credits.availableCount > 0 && (
          <div className="flex justify-end">
            <Badge className="h-6 bg-success/10 px-2.5 text-sm text-success">
              可用{credits.availableCount}次
            </Badge>
          </div>
        )}
        {!credits ? (
          <div className="flex min-h-32 items-center justify-center text-muted-foreground">
            <LoaderCircle className="animate-spin" />
            <span className="sr-only">正在查询重置卡</span>
          </div>
        ) : (
          <div className="flex max-h-[60vh] flex-col overflow-y-auto">
            {credits.credits.length ? (
              <div className="divide-y rounded-md border">
                {credits.credits.map((credit) => (
                  <ResetCreditRow key={credit.id} credit={credit} />
                ))}
              </div>
            ) : (
              <Empty className="min-h-32 p-4">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <Tickets />
                  </EmptyMedia>
                  <EmptyTitle>暂无重置卡</EmptyTitle>
                </EmptyHeader>
              </Empty>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function ResetCreditRow({ credit }: { credit: ResetCredit }) {
  const available = credit.status === 'available';
  return (
    <div className="flex min-h-20 items-center justify-between gap-4 p-4">
      <div className="min-w-0">
        <strong className="block truncate text-sm font-medium">{credit.title}</strong>
        <span className="mt-1 block text-sm text-muted-foreground">
          将于 {formatExpiryDate(credit.expiresAt)} 到期
        </span>
      </div>
      <Button className="px-4" type="button" disabled={!available}>
        {available ? '使用重置' : statusLabel(credit.status)}
      </Button>
    </div>
  );
}

function statusLabel(status: string) {
  return { available: '可用', redeemed: '已兑换', expired: '已过期' }[status] ?? status;
}

function formatExpiryDate(value: string) {
  const date = dayjs(value);
  return date.isValid() ? date.format('MM-DD HH:mm') : value;
}
