import { invoke } from '@tauri-apps/api/core';
import { Link, Outlet, useRouterState } from '@tanstack/react-router';
import {
  ArrowLeft,
  ChartNoAxesCombined,
  Check,
  ChevronDown,
  CreditCard,
  ExternalLink,
  FileCog,
  Info,
  FileText,
  MessagesSquare,
  RefreshCw,
  Settings,
  SlidersHorizontal,
  TriangleAlert,
  UsersRound,
} from 'lucide-react';
import {
  type ComponentType,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';
import { toast } from 'sonner';
import chatGptIcon from '../assets/chatgpt.svg';
import claudeIcon from '../assets/claude.svg';
import { appError, cn } from '../utils';
import { Alert, AlertDescription } from './ui/alert';
import { Button } from './ui/button';
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
  const [codexCliAvailable, setCodexCliAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    if (!isSettings)
      previousAppPath.current = pathname.startsWith('/prompts/')
        ? '/prompts'
        : (pathname as MainPath);
  }, [isSettings, pathname]);

  return (
    <div className="relative flex h-screen min-h-0 bg-background text-foreground">
      {isWindows ? (
        <WindowTitleBar />
      ) : (
        <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-40 h-10" />
      )}
      <aside
        className={cn(
          'flex w-[172px] shrink-0 flex-col border-r border-border bg-muted/40 px-3 pb-2 lg:w-56',
          isWindows ? 'pt-9' : 'pt-10',
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
              <NavItem
                to="/sessions"
                label="会话管理"
                icon={MessagesSquare}
                disabled={codexCliAvailable === false}
              />
              <NavItem to="/prompts" label="提示词管理" icon={FileText} />
              <NavItem to="/config" label="配置" icon={FileCog} />
            </nav>
            <nav className="border-t border-border pt-2" aria-label="应用导航">
              <NavItem to="/settings" label="设置" icon={Settings} />
            </nav>
          </>
        )}
      </aside>
      <div className={cn('flex min-w-0 flex-1 flex-col', isWindows ? 'pt-8' : 'pt-10')}>
        <CodexCliAlert available={codexCliAvailable} onAvailableChange={setCodexCliAvailable} />
        <div className="min-h-0 flex-1">
          <Outlet />
        </div>
      </div>
    </div>
  );
}

function CodexCliAlert({
  available,
  onAvailableChange,
}: {
  available: boolean | null;
  onAvailableChange: (available: boolean) => void;
}) {
  const [checking, setChecking] = useState(false);

  const check = useCallback(
    async (minimumDuration = 0) => {
      setChecking(true);
      try {
        const [isAvailable] = await Promise.all([
          invoke<boolean>('is_codex_cli_available'),
          new Promise((resolve) => setTimeout(resolve, minimumDuration)),
        ]);
        onAvailableChange(isAvailable);
      } catch (error) {
        toast.error(appError(error));
      } finally {
        setChecking(false);
      }
    },
    [onAvailableChange],
  );

  useEffect(() => {
    void check();
  }, [check]);

  async function openInstallPage() {
    try {
      await invoke('open_codex_cli_install_page');
    } catch (error) {
      toast.error(appError(error));
    }
  }

  if (available !== false) return null;

  return (
    <Alert variant="warning" className="mb-4 rounded-none border-0 px-4 sm:px-8 lg:px-12">
      <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2">
          <TriangleAlert className="size-4 shrink-0" />
          <span>未检测到 Codex CLI，部分功能将不可用。</span>
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            className="text-warning-foreground hover:bg-warning/15 focus-visible:border-warning/40 focus-visible:ring-warning/20"
            onClick={() => void check(400)}
            disabled={checking}
          >
            <RefreshCw data-icon="inline-start" className={cn(checking && 'animate-spin')} />
            {checking ? '检测中...' : '重新检测'}
          </Button>
          <Button variant="link" size="sm" onClick={() => void openInstallPage()}>
            <ExternalLink data-icon="inline-start" />
            安装 Codex CLI
          </Button>
        </span>
      </AlertDescription>
    </Alert>
  );
}

type MainPath = '/' | '/sessions' | '/analytics' | '/prompts' | '/config';

type AppPath = MainPath | '/settings' | '/settings/billing' | '/settings/about';

function NavItem({
  to,
  label,
  icon: Icon,
  exact = false,
  disabled = false,
}: {
  to: AppPath;
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
      activeProps={{ className: 'bg-accent font-medium text-accent-foreground' }}
    >
      {content}
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
