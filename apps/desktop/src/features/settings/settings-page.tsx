import { ExternalLink, FolderOpen, RefreshCw } from 'lucide-react';
import { type FormEvent, type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { invoke, isTauri } from '../../backend';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Card, CardAction, CardContent, CardHeader, CardTitle } from '../../components/ui/card';
import { Field, FieldError } from '../../components/ui/field';
import { Input } from '../../components/ui/input';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Switch } from '../../components/ui/switch';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { appError } from '../../utils';
import { type CliEnvironment, isCliUpgradeAvailable } from './cli-environment';

type AppStatus = {
  autostartEnabled: boolean;
  webAccess: WebAccessStatus;
};

type WebAccessStatus = {
  enabled: boolean;
  port: number;
  available: boolean;
  error: string | null;
};

type UsageRefreshSettings = {
  enabled: boolean;
  activeIntervalMinutes: number;
  inactiveIntervalMinutes: number;
};

type TerminalApp = 'terminal' | 'warp' | 'ghostty';

const terminalOptions = [
  { label: 'Terminal', value: 'terminal' },
  { label: 'Warp', value: 'warp' },
  { label: 'Ghostty', value: 'ghostty' },
];
const activeRefreshOptions = [1, 2, 5, 10].map((minutes) => ({
  label: `${minutes} 分钟`,
  value: String(minutes),
}));
const inactiveRefreshOptions = [5, 10, 30, 60].map((minutes) => ({
  label: `${minutes} 分钟`,
  value: String(minutes),
}));

export default function SettingsPage() {
  return (
    <PageShell>
      <PageHeader title="常规" />
      <SettingsContent />
    </PageShell>
  );
}

function SettingsContent() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [usageRefresh, setUsageRefresh] = useState<UsageRefreshSettings | null>(null);
  const [terminalApp, setTerminalApp] = useState<TerminalApp | null>(null);
  const [webPort, setWebPort] = useState('11456');
  const [webPortError, setWebPortError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>('loading');

  useEffect(() => {
    Promise.all([
      invoke<AppStatus>('get_app_status'),
      invoke<UsageRefreshSettings>('get_usage_refresh_settings'),
      invoke<TerminalApp>('get_terminal_app'),
    ])
      .then(([nextStatus, nextUsageRefresh, nextTerminalApp]) => {
        setStatus(nextStatus);
        setUsageRefresh(nextUsageRefresh);
        setTerminalApp(nextTerminalApp);
        setWebPort(String(nextStatus.webAccess.port));
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(null));
  }, []);

  async function saveUsageRefresh(next: UsageRefreshSettings) {
    setBusy('usage-refresh');
    try {
      setUsageRefresh(await invoke<UsageRefreshSettings>('set_usage_refresh_settings', next));
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function saveTerminalApp(next: TerminalApp) {
    setBusy('terminal-app');
    try {
      setTerminalApp(await invoke<TerminalApp>('set_terminal_app', { terminalApp: next }));
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function toggleAutostart(enabled: boolean) {
    setBusy('autostart');
    try {
      await invoke('set_autostart', { enabled });
      setStatus((current) => (current ? { ...current, autostartEnabled: enabled } : current));
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function saveWebAccess(enabled: boolean, showSuccess = false) {
    const port =
      !enabled && !showSuccess ? (status?.webAccess.port ?? Number(webPort)) : Number(webPort);
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      setWebPortError('端口必须是 1024 到 65535 之间的整数。');
      return;
    }
    setWebPortError(null);
    setBusy('web');
    try {
      const webAccess = await invoke<WebAccessStatus>('set_web_access_settings', {
        enabled,
        port,
      });
      setStatus((current) => (current ? { ...current, webAccess } : current));
      setWebPort(String(webAccess.port));
      if (showSuccess) toast.success('Web 端口已更新。');
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  function saveWebPort(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    event.currentTarget.querySelector('input')?.blur();
  }

  function commitWebPort() {
    if (webPort === String(status?.webAccess.port)) return;
    void saveWebAccess(true, true);
  }

  async function openWebAccess() {
    try {
      await invoke('open_web_access');
    } catch (error) {
      toast.error(appError(error));
    }
  }

  return (
    <div className="w-full px-4 pt-6 pb-10 sm:px-8 lg:px-12">
      <div className="flex flex-col gap-6">
        <section aria-labelledby="app-settings-heading" className="flex flex-col gap-2">
          <h2 id="app-settings-heading" className="px-1 text-sm font-medium text-foreground">
            应用设置
          </h2>
          <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
              <div className="flex min-w-0 flex-col gap-1">
                <h3 className="text-sm font-medium">启动</h3>
                <p className="text-sm text-muted-foreground">系统登录后自动运行 Cortana</p>
              </div>
              <Switch
                aria-label="登录系统时启动"
                className="justify-self-end"
                checked={status?.autostartEnabled ?? false}
                onCheckedChange={(enabled) => void toggleAutostart(enabled)}
                disabled={busy === 'autostart' || busy === 'loading'}
              />
            </div>
            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
              <div className="flex min-w-0 flex-col gap-1">
                <h3 className="text-sm font-medium">终端应用</h3>
                <p className="text-sm text-muted-foreground">打开账号终端时使用</p>
              </div>
              <Select
                items={terminalOptions}
                value={terminalApp}
                onValueChange={(next) => next && void saveTerminalApp(next as TerminalApp)}
                disabled={!terminalApp || busy === 'terminal-app' || busy === 'loading'}
              >
                <SelectTrigger className="w-32" aria-label="终端应用">
                  <SelectValue>
                    {terminalApp && (
                      <>
                        <TerminalAppIcon app={terminalApp} />
                        {terminalOptions.find((option) => option.value === terminalApp)?.label}
                      </>
                    )}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {terminalOptions.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        <TerminalAppIcon app={option.value as TerminalApp} />
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
          </div>
        </section>

        <section aria-labelledby="usage-refresh-heading" className="flex flex-col gap-2">
          <h2 id="usage-refresh-heading" className="px-1 text-sm font-medium text-foreground">
            额度刷新
          </h2>
          <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
              <div className="flex min-w-0 flex-col gap-1">
                <h3 className="text-sm font-medium">自动刷新</h3>
                <p className="text-sm text-muted-foreground">定期更新 Codex 账户额度</p>
              </div>
              <Switch
                aria-label="自动刷新账户额度"
                className="justify-self-end"
                checked={usageRefresh?.enabled ?? false}
                onCheckedChange={(enabled) =>
                  usageRefresh && void saveUsageRefresh({ ...usageRefresh, enabled })
                }
                disabled={!usageRefresh || busy === 'usage-refresh' || busy === 'loading'}
              />
            </div>

            <UsageRefreshIntervalRow
              title="启用账号刷新间隔"
              options={activeRefreshOptions}
              value={usageRefresh?.activeIntervalMinutes}
              disabled={!usageRefresh?.enabled || busy === 'usage-refresh' || busy === 'loading'}
              onChange={(activeIntervalMinutes) =>
                usageRefresh && void saveUsageRefresh({ ...usageRefresh, activeIntervalMinutes })
              }
            />

            <UsageRefreshIntervalRow
              title="未启用账号刷新间隔"
              options={inactiveRefreshOptions}
              value={usageRefresh?.inactiveIntervalMinutes}
              disabled={!usageRefresh?.enabled || busy === 'usage-refresh' || busy === 'loading'}
              onChange={(inactiveIntervalMinutes) =>
                usageRefresh && void saveUsageRefresh({ ...usageRefresh, inactiveIntervalMinutes })
              }
            />
          </div>
        </section>

        <section aria-labelledby="access-data-heading" className="flex flex-col gap-2">
          <h2 id="access-data-heading" className="px-1 text-sm font-medium text-foreground">
            访问与数据
          </h2>
          <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
              <div className="flex min-w-0 flex-col gap-1">
                <div className="flex items-center gap-2">
                  <h3 className="text-sm font-medium">Web 访问</h3>
                  {status?.webAccess.available && (
                    <Tooltip>
                      <TooltipTrigger render={<span className="inline-flex" />}>
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          type="button"
                          onClick={() => void openWebAccess()}
                        >
                          <ExternalLink />
                          <span className="sr-only">在浏览器中打开</span>
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>在浏览器中打开</TooltipContent>
                    </Tooltip>
                  )}
                  <Badge
                    variant={
                      status?.webAccess.enabled
                        ? status.webAccess.available
                          ? 'default'
                          : 'destructive'
                        : 'secondary'
                    }
                  >
                    {status?.webAccess.enabled
                      ? status.webAccess.available
                        ? '运行中'
                        : '启动失败'
                      : '已关闭'}
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground">允许本机浏览器控制 Cortana</p>
                {!isTauri && <p className="text-xs text-muted-foreground">仅可在桌面应用中修改</p>}
                {status?.webAccess.error && (
                  <p className="text-xs text-destructive">{status.webAccess.error}</p>
                )}
              </div>
              <Switch
                aria-label="启用 Web 访问"
                className="justify-self-end"
                checked={status?.webAccess.enabled ?? false}
                onCheckedChange={(enabled) => void saveWebAccess(enabled)}
                disabled={!isTauri || busy === 'web' || busy === 'loading'}
              />
            </div>

            {status?.webAccess.enabled && (
              <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
                <div className="flex min-w-0 flex-col gap-1">
                  <h3 className="text-sm font-medium">Web 端口</h3>
                  <p className="text-sm text-muted-foreground">本机 Web 服务监听端口</p>
                </div>
                <form className="w-28" onSubmit={saveWebPort}>
                  <Field data-invalid={Boolean(webPortError)}>
                    <Input
                      aria-label="Web 端口"
                      aria-invalid={Boolean(webPortError)}
                      inputMode="numeric"
                      type="text"
                      value={webPort}
                      onChange={(event) => {
                        setWebPort(event.target.value);
                        setWebPortError(null);
                      }}
                      onBlur={commitWebPort}
                      disabled={!isTauri || busy === 'web' || busy === 'loading'}
                    />
                    {webPortError && <FieldError match>{webPortError}</FieldError>}
                  </Field>
                </form>
              </div>
            )}

            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
              <div className="flex min-w-0 flex-col gap-1">
                <h3 className="text-sm font-medium">本地数据</h3>
                <p className="text-sm text-muted-foreground">账户档案仅保存在本机</p>
              </div>
              <Button
                className="w-fit justify-self-end"
                variant="outline"
                type="button"
                onClick={() => void invoke('reveal_data_directory')}
              >
                <FolderOpen data-icon="inline-start" /> 打开数据目录
              </Button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}

function TerminalAppIcon({ app }: { app: TerminalApp }) {
  if (app === 'terminal') {
    return (
      <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect width="18" height="18" x="3" y="3" rx="2" />
        <path d="m7 8 4 4-4 4M13 16h4" />
      </svg>
    );
  }

  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" fill="currentColor">
      <path
        d={
          app === 'warp'
            ? 'M12.035 2.723h9.253A2.712 2.712 0 0 1 24 5.435v10.529a2.712 2.712 0 0 1-2.712 2.713H8.047Zm-1.681 2.6L6.766 19.677h5.598l-.399 1.6H2.712A2.712 2.712 0 0 1 0 18.565V8.036a2.712 2.712 0 0 1 2.712-2.712Z'
            : 'M12 0C6.7 0 2.4 4.3 2.4 9.6v11.146c0 1.772 1.45 3.267 3.222 3.254a3.18 3.18 0 0 0 1.955-.686 1.96 1.96 0 0 1 2.444 0 3.18 3.18 0 0 0 1.976.686c.75 0 1.436-.257 1.98-.686.715-.563 1.71-.587 2.419-.018.59.476 1.355.743 2.182.699 1.705-.094 3.022-1.537 3.022-3.244V9.601C21.6 4.3 17.302 0 12 0M6.069 6.562a1 1 0 0 1 .46.131l3.578 2.065v.002a.974.974 0 0 1 0 1.687L6.53 12.512a.975.975 0 0 1-.976-1.687L7.67 9.602 5.553 8.38a.975.975 0 0 1 .515-1.818m7.438 2.063h4.7a.975.975 0 1 1 0 1.95h-4.7a.975.975 0 0 1 0-1.95'
        }
      />
    </svg>
  );
}

function UsageRefreshIntervalRow({
  title,
  options,
  value,
  disabled,
  onChange,
}: {
  title: string;
  options: { label: string; value: string }[];
  value?: number;
  disabled: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4">
      <h3 className="min-w-0 text-sm font-medium">{title}</h3>
      <Select
        items={options}
        value={value === undefined ? null : String(value)}
        onValueChange={(next) => next && onChange(Number(next))}
        disabled={disabled}
      >
        <SelectTrigger className="w-32" aria-label={title}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            {options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </div>
  );
}

export function AboutPage() {
  return (
    <PageShell>
      <AboutContent />
    </PageShell>
  );
}

function AboutContent() {
  const [environments, setEnvironments] = useState<{
    codex: CliEnvironment;
    claude: CliEnvironment;
    antigravity: CliEnvironment;
    grok: CliEnvironment;
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const requestId = useRef(0);

  const loadEnvironment = useCallback(async () => {
    const currentRequest = ++requestId.current;
    setLoading(true);
    try {
      const [codex, claude, antigravity, grok] = await Promise.all([
        invoke<CliEnvironment>('get_codex_cli_environment'),
        invoke<CliEnvironment>('get_claude_cli_environment'),
        invoke<CliEnvironment>('get_antigravity_cli_environment'),
        invoke<CliEnvironment>('get_grok_cli_environment'),
      ]);
      if (currentRequest === requestId.current) {
        setEnvironments({ codex, claude, antigravity, grok });
      }
    } catch (error) {
      if (currentRequest === requestId.current) toast.error(appError(error));
    } finally {
      if (currentRequest === requestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadEnvironment();
    return () => {
      requestId.current += 1;
    };
  }, [loadEnvironment]);

  return (
    <>
      <PageHeader
        title="关于"
        actions={
          <Button
            variant="ghost"
            size="icon"
            type="button"
            onClick={() => void loadEnvironment()}
            disabled={loading}
          >
            <RefreshCw className={loading ? 'animate-spin' : ''} />
            <span className="sr-only">刷新本地环境</span>
          </Button>
        }
      />
      <div className="w-full px-4 pt-6 pb-10 sm:px-8 lg:px-12">
        <section className="grid gap-3 md:grid-cols-2">
          <CliEnvironmentCard
            title="Codex CLI"
            environment={environments?.codex}
            loading={loading}
          />
          <CliEnvironmentCard
            title="Claude CLI"
            environment={environments?.claude}
            loading={loading}
          />
          <CliEnvironmentCard
            title="Antigravity CLI"
            environment={environments?.antigravity}
            loading={loading}
          />
          <CliEnvironmentCard title="Grok CLI" environment={environments?.grok} loading={loading} />
        </section>
      </div>
    </>
  );
}

function CliEnvironmentCard({
  title,
  environment,
  loading,
}: {
  title: string;
  environment?: CliEnvironment;
  loading: boolean;
}) {
  const upgradeAvailable = isCliUpgradeAvailable(environment);
  const status =
    loading && !environment
      ? '检测中'
      : !environment
        ? '检测失败'
        : upgradeAvailable
          ? '可升级'
          : environment.installed
            ? '已安装'
            : '未安装';

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardAction>
          <Badge
            variant={
              upgradeAvailable ? 'warning' : environment?.installed ? 'default' : 'secondary'
            }
          >
            {status}
          </Badge>
        </CardAction>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-2">
          <EnvironmentRow label="当前版本" mono>
            {loading && !environment
              ? '检测中...'
              : environment?.installed
                ? (environment.installedVersion ?? '未知')
                : '未安装'}
          </EnvironmentRow>
          <EnvironmentRow label="最新版本" mono>
            {loading && !environment ? '检测中...' : (environment?.latestVersion ?? '获取失败')}
          </EnvironmentRow>
          <EnvironmentRow label="安装方式" mono>
            {environment?.installed ? installMethodLabel(environment.installMethod) : '未知'}
          </EnvironmentRow>
        </dl>
      </CardContent>
    </Card>
  );
}

function EnvironmentRow({
  label,
  mono,
  children,
}: {
  label: string;
  mono?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="grid min-h-6 grid-cols-[6rem_minmax(0,1fr)] items-center gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={mono ? 'font-mono text-xs' : undefined}>{children}</dd>
    </div>
  );
}

function installMethodLabel(method: CliEnvironment['installMethod']) {
  return {
    brew: 'Homebrew',
    npm: 'npm',
    pnpm: 'pnpm',
    bun: 'Bun',
    standalone: '官方安装器',
    unknown: '未知',
  }[method];
}
