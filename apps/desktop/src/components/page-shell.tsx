import { Link, Outlet, useRouterState } from '@tanstack/react-router';
import {
  ArrowLeft,
  ChartNoAxesCombined,
  Check,
  ChevronDown,
  CreditCard,
  FileCog,
  Info,
  FileText,
  MessagesSquare,
  Settings,
  SlidersHorizontal,
  UsersRound,
} from 'lucide-react';
import { type ComponentType, type ReactNode, useRef } from 'react';
import chatGptIcon from '../assets/chatgpt.svg';
import claudeIcon from '../assets/claude.svg';
import { cn } from '../utils';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';
import { WindowTitleBar } from './window-title-bar';

export function AppLayout() {
  const isWindows = navigator.userAgent.includes('Windows');
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isSettings = pathname === '/settings' || pathname.startsWith('/settings/');
  const previousAppPath = useRef<MainPath>('/');

  if (!isSettings)
    previousAppPath.current = pathname.startsWith('/prompts/')
      ? '/prompts'
      : (pathname as MainPath);

  return (
    <div className="relative flex h-screen min-h-0 bg-background text-foreground">
      {isWindows ? (
        <WindowTitleBar />
      ) : (
        <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-40 h-11" />
      )}
      <aside
        className={cn(
          'flex w-[172px] shrink-0 flex-col border-r border-border bg-muted/40 px-3 pb-2 lg:w-56',
          isWindows ? 'pt-9' : 'pt-12',
        )}
      >
        {isSettings ? (
          <>
            <Link
              to={previousAppPath.current}
              className="flex h-9 items-center gap-2 rounded-md px-2 outline-none transition-colors hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
            >
              <ArrowLeft className="size-4 shrink-0" />
              <strong className="truncate text-base font-semibold">设置</strong>
            </Link>
            <nav className="mt-3 flex flex-1 flex-col gap-1" aria-label="设置导航">
              <NavItem to="/settings" label="常规" icon={SlidersHorizontal} exact />
              <NavItem to="/settings/billing" label="计费" icon={CreditCard} />
              <NavItem to="/settings/about" label="关于" icon={Info} />
            </nav>
          </>
        ) : (
          <>
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <button
                    type="button"
                    className="flex h-9 w-fit items-center gap-2 rounded-md px-2 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50 data-popup-open:bg-accent"
                  />
                }
              >
                <img src={chatGptIcon} alt="" className="size-5 shrink-0" />
                <strong className="truncate text-base font-semibold">Codex</strong>
                <ChevronDown className="ml-auto size-4 shrink-0 text-muted-foreground" />
              </DropdownMenuTrigger>
              <DropdownMenuContent sideOffset={4}>
                <DropdownMenuGroup>
                  <DropdownMenuItem>
                    <img src={chatGptIcon} alt="" className="size-4" />
                    Codex
                    <Check className="ml-auto" />
                  </DropdownMenuItem>
                  <DropdownMenuItem disabled>
                    <img src={claudeIcon} alt="" className="size-4" />
                    Claude
                  </DropdownMenuItem>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
            <nav className="mt-3 flex flex-1 flex-col gap-1" aria-label="主导航">
              <NavItem to="/" label="账号" icon={UsersRound} exact />
              <NavItem to="/analytics" label="统计分析" icon={ChartNoAxesCombined} />
              <NavItem to="/sessions" label="会话管理" icon={MessagesSquare} />
              <NavItem to="/prompts" label="提示词管理" icon={FileText} />
              <NavItem to="/config" label="Codex 配置" icon={FileCog} />
            </nav>
            <nav className="border-t border-border pt-2" aria-label="应用导航">
              <NavItem to="/settings" label="设置" icon={Settings} />
            </nav>
          </>
        )}
      </aside>
      <div className={cn('flex min-w-0 flex-1 flex-col', isWindows ? 'pt-8' : 'pt-11')}>
        <div className="min-h-0 flex-1">
          <Outlet />
        </div>
      </div>
    </div>
  );
}

type MainPath = '/' | '/sessions' | '/analytics' | '/prompts' | '/config';

type AppPath = MainPath | '/settings' | '/settings/billing' | '/settings/about';

function NavItem({
  to,
  label,
  icon: Icon,
  exact = false,
}: {
  to: AppPath;
  label: string;
  icon: ComponentType<{ className?: string }>;
  exact?: boolean;
}) {
  return (
    <Link
      to={to}
      activeOptions={{ exact }}
      className="flex h-9 items-center gap-3 rounded-md px-3 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      activeProps={{ className: 'bg-accent font-medium text-accent-foreground' }}
    >
      <Icon className="size-4 shrink-0" />
      <span className="truncate">{label}</span>
    </Link>
  );
}

export function PageShell({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <main className={cn('h-full overflow-y-auto bg-background text-foreground', className)}>
      {children}
    </main>
  );
}

export function PageHeader({
  title,
  description,
  actions,
  leading,
  className,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  leading?: ReactNode;
  className?: string;
}) {
  return (
    <header
      className={cn(
        'flex w-full items-center justify-between gap-4 px-4 sm:px-8 lg:px-12',
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-3">
        {leading}
        <div className="min-w-0">
          <h1 className="truncate text-lg font-semibold">{title}</h1>
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </div>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}
