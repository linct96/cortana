import { RefreshCw } from 'lucide-react';
import type { ComponentProps } from 'react';
import { useEffect, useRef, useState } from 'react';
import { Button } from './ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

export function RefreshButton({
  label = '刷新',
  disabled,
  minSpinDuration = 1000,
  onRefresh,
  ...props
}: Omit<ComponentProps<typeof Button>, 'children' | 'onClick'> & {
  label?: string;
  minSpinDuration?: number;
  onRefresh: () => Promise<unknown>;
}) {
  const [refreshing, setRefreshing] = useState(false);
  const resetTimer = useRef<number | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    };
  }, []);

  async function refresh() {
    const startedAt = performance.now();
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      if (mounted.current) {
        const remaining = Math.max(0, minSpinDuration - (performance.now() - startedAt));
        if (remaining === 0) {
          setRefreshing(false);
        } else {
          resetTimer.current = window.setTimeout(() => setRefreshing(false), remaining);
        }
      }
    }
  }

  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={label}
          aria-busy={refreshing}
          disabled={disabled || refreshing}
          onClick={refresh}
          {...props}
        >
          <RefreshCw className={refreshing ? 'animate-spin' : ''} />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
