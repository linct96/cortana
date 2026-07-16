import { Outlet, useRouterState } from '@tanstack/react-router';
import {
  ChartNoAxesCombined,
  Check,
  ChevronDown,
  ExternalLink,
  FileCog,
  FileText,
  MessagesSquare,
  RefreshCw,
  Settings,
  TriangleAlert,
  UsersRound,
} from 'lucide-react';
import {
  createContext,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react';
import { toast } from 'sonner';
import chatGptIcon from '../assets/chatgpt.svg';
import claudeIcon from '../assets/claude.svg';
import { invoke, isTauri } from '../backend';
import { appError, cn } from '../utils';
import { AppSidebar, SidebarNavItem } from './app-sidebar';
import { Alert, AlertDescription } from './ui/alert';
import { Button } from './ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';

export type MainPath = '/accounts' | '/sessions' | '/analytics' | '/prompts' | '/config';

type AppShellContextValue = {
  topPadding: string;
  previousMainPath: MainPath;
  codexCliAvailable: boolean | null;
  setCodexCliAvailable: Dispatch<SetStateAction<boolean | null>>;
};

const AppShellContext = createContext<AppShellContextValue | null>(null);

export function useAppShell() {
  const context = useContext(AppShellContext);
  if (!context) throw new Error('useAppShell must be used within AppShell');
  return context;
}

export function AppShell() {
  const isMacOS = isTauri && navigator.userAgent.includes('Mac');
  const topPadding = isMacOS ? 'pt-10' : 'pt-3';
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const previousMainPath = useRef<MainPath>('/accounts');
  const [codexCliAvailable, setCodexCliAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    const nextPath = mainPathFor(pathname);
    if (nextPath) previousMainPath.current = nextPath;
  }, [pathname]);

  return (
    <AppShellContext.Provider
      value={{
        topPadding,
        previousMainPath: previousMainPath.current,
        codexCliAvailable,
        setCodexCliAvailable,
      }}
    >
      <div className="relative flex h-screen min-h-0 bg-background text-foreground">
        {isMacOS && <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-40 h-10" />}
        <Outlet />
      </div>
    </AppShellContext.Provider>
  );
}

export function MainLayout() {
  const { topPadding, codexCliAvailable } = useAppShell();

  return (
    <>
      <AppSidebar
        className={topPadding}
        header={<ProductMenu />}
        navigationLabel="主导航"
        navigation={
          <>
            <SidebarNavItem to="/accounts" label="账号" icon={UsersRound} />
            <SidebarNavItem to="/analytics" label="统计分析" icon={ChartNoAxesCombined} />
            <SidebarNavItem
              to="/sessions"
              label="会话管理"
              icon={MessagesSquare}
              disabled={codexCliAvailable === false}
            />
            <SidebarNavItem to="/prompts" label="提示词管理" icon={FileText} />
            <SidebarNavItem to="/config" label="配置" icon={FileCog} />
          </>
        }
        footer={<SidebarNavItem to="/settings/general" label="设置" icon={Settings} />}
      />
      <AppContent>
        <Outlet />
      </AppContent>
    </>
  );
}

export function AppContent({ children }: { children: ReactNode }) {
  const { topPadding, codexCliAvailable, setCodexCliAvailable } = useAppShell();

  return (
    <div className={cn('flex min-w-0 flex-1 flex-col', topPadding)}>
      <CodexCliAlert available={codexCliAvailable} onAvailableChange={setCodexCliAvailable} />
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  );
}

function ProductMenu() {
  return (
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
    if (available === null) void check();
  }, [available, check]);

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

function mainPathFor(pathname: string): MainPath | null {
  if (pathname.startsWith('/prompts/')) return '/prompts';
  if (
    pathname === '/accounts' ||
    pathname === '/sessions' ||
    pathname === '/analytics' ||
    pathname === '/prompts' ||
    pathname === '/config'
  ) {
    return pathname;
  }
  return null;
}
