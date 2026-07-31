import { Outlet, useNavigate, useRouterState } from '@tanstack/react-router';
import {
  ChartNoAxesCombined,
  Check,
  ChevronDown,
  ExternalLink,
  FileCog,
  FileText,
  MessagesSquare,
  Boxes,
  RefreshCw,
  Settings,
  TriangleAlert,
  UsersRound,
} from 'lucide-react';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import antigravityIcon from '../assets/antigravity.svg';
import chatGptIcon from '../assets/chatgpt.svg';
import claudeIcon from '../assets/claude.svg';
import grokIcon from '../assets/grok.svg';
import { invoke, isTauri } from '../backend';
import { appError, cn } from '../utils';
import { AppSidebar, SidebarNavItem } from './app-sidebar';
import {
  AppShellContext,
  type AccountProduct,
  type MainPath,
  productName,
  useAppShell,
} from './app-shell-context';
import { Alert, AlertDescription } from './ui/alert';
import { Button } from './ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';

export function AppShell() {
  const isMacOS = isTauri && navigator.userAgent.includes('Mac');
  const topPadding = isMacOS ? 'pt-10' : 'pt-3';
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const previousMainPath = useRef<MainPath>('/accounts');
  const [activeProduct, setActiveProduct] = useState<AccountProduct>('codex');
  const [cliAvailable, setCliAvailable] = useState<boolean | null>(null);
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);

  useEffect(() => {
    const nextPath = mainPathFor(pathname);
    if (activeProduct === 'codex' && nextPath) previousMainPath.current = nextPath;
  }, [activeProduct, pathname]);

  useEffect(() => {
    invoke<AccountProduct>('get_active_product')
      .then(setActiveProduct)
      .catch((error) => toast.error(appError(error)));
  }, []);

  return (
    <AppShellContext.Provider
      value={{
        topPadding,
        previousMainPath: previousMainPath.current,
        activeProduct,
        setActiveProduct,
        cliAvailable,
        setCliAvailable,
        hasUnsavedChanges,
        setHasUnsavedChanges,
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
  const { topPadding, activeProduct } = useAppShell();

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
            <SidebarNavItem to="/sessions" label="会话管理" icon={MessagesSquare} />
            <SidebarNavItem to="/prompts" label="提示词管理" icon={FileText} />
            {(activeProduct === 'codex' ||
              activeProduct === 'claude' ||
              activeProduct === 'grok') && (
              <SidebarNavItem to="/models" label="自定义模型" icon={Boxes} />
            )}
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
  const { topPadding, activeProduct, cliAvailable, setCliAvailable } = useAppShell();

  return (
    <div className={cn('flex min-w-0 flex-1 flex-col', topPadding)}>
      <CliAlert
        key={activeProduct}
        product={activeProduct}
        available={cliAvailable}
        onAvailableChange={setCliAvailable}
      />
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  );
}

function ProductMenu() {
  const navigate = useNavigate();
  const { activeProduct, setActiveProduct, setCliAvailable, hasUnsavedChanges } = useAppShell();
  const claude = activeProduct === 'claude';
  const antigravity = activeProduct === 'antigravity';
  const grok = activeProduct === 'grok';

  async function selectProduct(product: AccountProduct) {
    if (product === activeProduct) return;
    if (hasUnsavedChanges && !window.confirm('当前修改尚未保存，确定放弃并切换产品吗？')) {
      return;
    }
    try {
      await invoke('set_active_product', { product });
      setActiveProduct(product);
      setCliAvailable(null);
      await navigate({ to: '/accounts', ignoreBlocker: true });
    } catch (error) {
      toast.error(appError(error));
    }
  }

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
        <img
          src={claude ? claudeIcon : antigravity ? antigravityIcon : grok ? grokIcon : chatGptIcon}
          alt=""
          className="size-5 shrink-0"
        />
        <strong className="truncate text-base font-semibold">
          {claude ? 'Claude' : antigravity ? 'Antigravity' : grok ? 'Grok' : 'Codex'}
        </strong>
        <ChevronDown className="ml-auto size-4 shrink-0 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent sideOffset={4}>
        <DropdownMenuGroup>
          <DropdownMenuItem onClick={() => void selectProduct('antigravity')}>
            <img src={antigravityIcon} alt="" className="size-4" />
            Antigravity
            {antigravity && <Check className="ml-auto" />}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => void selectProduct('claude')}>
            <img src={claudeIcon} alt="" className="size-4" />
            Claude
            {claude && <Check className="ml-auto" />}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => void selectProduct('codex')}>
            <img src={chatGptIcon} alt="" className="size-4" />
            Codex
            {activeProduct === 'codex' && <Check className="ml-auto" />}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => void selectProduct('grok')}>
            <img src={grokIcon} alt="" className="size-4" />
            Grok
            {grok && <Check className="ml-auto" />}
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CliAlert({
  product,
  available,
  onAvailableChange,
}: {
  product: AccountProduct;
  available: boolean | null;
  onAvailableChange: (available: boolean) => void;
}) {
  const [checking, setChecking] = useState(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const check = useCallback(
    async (minimumDuration = 0) => {
      setChecking(true);
      try {
        const [isAvailable] = await Promise.all([
          invoke<boolean>(`is_${product}_cli_available`),
          new Promise((resolve) => setTimeout(resolve, minimumDuration)),
        ]);
        if (mounted.current) onAvailableChange(isAvailable);
      } catch (error) {
        if (mounted.current) toast.error(appError(error));
      } finally {
        if (mounted.current) setChecking(false);
      }
    },
    [onAvailableChange, product],
  );

  useEffect(() => {
    if (available === null) void check();
  }, [available, check]);

  async function openInstallPage() {
    try {
      await invoke(`open_${product}_cli_install_page`);
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
          <span>未检测到 {productName(product)} CLI，部分功能将不可用。</span>
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
            安装 {productName(product)} CLI
          </Button>
        </span>
      </AlertDescription>
    </Alert>
  );
}

function mainPathFor(pathname: string): MainPath | null {
  if (pathname.startsWith('/prompts/')) return '/prompts';
  if (pathname.startsWith('/models/')) return '/models';
  if (
    pathname === '/accounts' ||
    pathname === '/sessions' ||
    pathname === '/analytics' ||
    pathname === '/prompts' ||
    pathname === '/models' ||
    pathname === '/config'
  ) {
    return pathname;
  }
  return null;
}
