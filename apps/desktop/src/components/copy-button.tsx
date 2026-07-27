import { Check, Copy } from 'lucide-react';
import type { ComponentProps } from 'react';
import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Button } from './ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

export function CopyButton({
  value,
  label = '复制',
  disabled,
  ...props
}: Omit<ComponentProps<typeof Button>, 'children' | 'onClick'> & {
  value: string;
  label?: string;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number | null>(null);

  useEffect(() => {
    setCopied(false);
    if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    return () => {
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    };
  }, [value]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error('复制失败，请重试。');
    }
  }

  const currentLabel = copied ? '已复制' : label;
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={currentLabel}
          disabled={disabled || !value}
          onClick={copy}
          {...props}
        >
          {copied ? <Check /> : <Copy />}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{currentLabel}</TooltipContent>
    </Tooltip>
  );
}
