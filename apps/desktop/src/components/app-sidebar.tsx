import { Link } from '@tanstack/react-router';
import type { ComponentProps, ComponentType, ReactNode } from 'react';
import { cn } from '../utils';

type SidebarPath = NonNullable<ComponentProps<typeof Link>['to']>;

export function AppSidebar({
  header,
  navigation,
  navigationLabel,
  footer,
  footerLabel = '应用导航',
  className,
}: {
  header: ReactNode;
  navigation: ReactNode;
  navigationLabel: string;
  footer?: ReactNode;
  footerLabel?: string;
  className?: string;
}) {
  return (
    <aside
      className={cn(
        'flex w-43 shrink-0 flex-col border-r border-border bg-muted/40 px-3 pb-2 lg:w-56',
        className,
      )}
    >
      {header}
      <nav className="mt-3 flex flex-1 flex-col gap-1" aria-label={navigationLabel}>
        {navigation}
      </nav>
      {footer && (
        <nav className="border-t border-border pt-2" aria-label={footerLabel}>
          {footer}
        </nav>
      )}
    </aside>
  );
}

export function SidebarNavItem({
  to,
  label,
  icon: Icon,
  exact = false,
  disabled = false,
}: {
  to: SidebarPath;
  label: string;
  icon: ComponentType<{ className?: string }>;
  exact?: boolean;
  disabled?: boolean;
}) {
  const content = (
    <>
      <Icon className="size-4 shrink-0" />
      <span className="truncate">{label}</span>
    </>
  );

  if (disabled) {
    return (
      <span
        aria-disabled="true"
        className="flex h-9 cursor-not-allowed items-center gap-3 rounded-md px-3 text-sm text-muted-foreground opacity-50"
      >
        {content}
      </span>
    );
  }

  return (
    <Link
      to={to}
      activeOptions={{ exact }}
      className="flex h-9 items-center gap-3 rounded-md px-3 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      activeProps={{
        className: 'bg-accent font-medium text-accent-foreground',
      }}
    >
      {content}
    </Link>
  );
}
