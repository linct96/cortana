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
  const [webPort, setWebPort] = useState('11456');
  const [webPortError, setWebPortError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>('loading');

  useEffect(() => {
    invoke<AppStatus>('get_app_status')
      .then((next) => {
        setStatus(next);
        setWebPort(String(next.webAccess.port));
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(null));
  }, []);

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
          <div className="overflow-hidden rounded-lg border border-border">
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
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const requestId = useRef(0);

  const loadEnvironment = useCallback(async () => {
    const currentRequest = ++requestId.current;
    setLoading(true);
    try {
      const [codex, claude] = await Promise.all([
        invoke<CliEnvironment>('get_codex_cli_environment'),
        invoke<CliEnvironment>('get_claude_cli_environment'),
      ]);
      if (currentRequest === requestId.current) setEnvironments({ codex, claude });
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
        <section className="grid gap-3 lg:grid-cols-2">
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
          <EnvironmentRow label="安装方式">
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
