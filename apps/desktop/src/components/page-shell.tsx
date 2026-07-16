import type { ReactNode } from 'react';
import { cn } from '../utils';

export function PageShell({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <main className={cn('h-full overflow-y-auto bg-background text-foreground', className)}>
      {children}
    </main>
  );
}

export function PageHeader({
  title,
  actions,
  leading,
}: {
  title: string;
  actions?: ReactNode;
  leading?: ReactNode;
}) {
  return (
    <header className="flex h-8 w-full shrink-0 items-center justify-between gap-4 px-4 sm:px-8 lg:px-12">
      <div className="flex h-8 min-w-0 items-center gap-3">
        {leading}
        <h1 className="truncate text-lg font-semibold">{title}</h1>
      </div>
      {actions && <div className="flex h-8 shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}
