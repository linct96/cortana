import { invoke } from '@tauri-apps/api/core';
import { Check, Database, FolderOpen, LoaderCircle } from 'lucide-react';
import { type FormEvent, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from './components/page-shell';
import { Button } from './components/ui/button';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from './components/ui/input-group';
import { Switch } from './components/ui/switch';
import { Tooltip, TooltipContent, TooltipTrigger } from './components/ui/tooltip';
import { appError, authFileParent } from './utils';

type AppStatus = {
  authPath: string;
  autostartEnabled: boolean;
};

export default function SettingsPage() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [codexHome, setCodexHome] = useState('');
  const [busy, setBusy] = useState<string | null>('loading');

  useEffect(() => {
    invoke<AppStatus>('get_app_status')
      .then((next) => {
        setStatus(next);
        setCodexHome(authFileParent(next.authPath));
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(null));
  }, []);

  async function saveCodexHome(event: FormEvent) {
    event.preventDefault();
    setBusy('path');
    try {
      const next = await invoke<AppStatus>('set_codex_home', { codexHome });
      setStatus(next);
      setCodexHome(authFileParent(next.authPath));
      toast.success('Codex 目录已更新。');
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

  return (
    <PageShell>
      <PageHeader title="常规" className="pb-7" />

      <div className="w-full border-y border-border px-4 sm:px-8 lg:px-12">
        <section className="grid gap-4 py-6 sm:grid-cols-[12rem_minmax(0,1fr)]">
          <div>
            <h2 className="text-sm font-medium">Codex 主目录</h2>
            <p className="mt-1 text-xs text-muted-foreground">认证文件所在目录</p>
          </div>
          <form onSubmit={saveCodexHome}>
            <InputGroup>
              <InputGroupInput
                aria-label="Codex 主目录"
                className="font-mono text-xs"
                value={codexHome}
                onChange={(event) => setCodexHome(event.target.value)}
                disabled={busy === 'loading'}
                spellCheck={false}
              />
              <InputGroupAddon align="inline-end">
                <Tooltip>
                  <TooltipTrigger render={<span className="inline-flex" />}>
                    <InputGroupButton
                      size="icon-sm"
                      type="submit"
                      disabled={busy === 'path' || busy === 'loading'}
                    >
                      {busy === 'path' ? <LoaderCircle className="animate-spin" /> : <Check />}
                      <span className="sr-only">保存 Codex 目录</span>
                    </InputGroupButton>
                  </TooltipTrigger>
                  <TooltipContent>保存 Codex 目录</TooltipContent>
                </Tooltip>
              </InputGroupAddon>
            </InputGroup>
          </form>
        </section>

        <section className="grid gap-4 border-t border-border py-6 sm:grid-cols-[12rem_minmax(0,1fr)] sm:items-center">
          <div>
            <h2 className="text-sm font-medium">启动</h2>
            <p className="mt-1 text-xs text-muted-foreground">系统登录后自动运行</p>
          </div>
          <label className="flex w-fit items-center gap-3 text-sm">
            <Switch
              checked={status?.autostartEnabled ?? false}
              onCheckedChange={(enabled) => void toggleAutostart(enabled)}
              disabled={busy === 'autostart' || busy === 'loading'}
            />
            登录系统时启动
          </label>
        </section>

        <section className="grid gap-4 border-t border-border py-6 sm:grid-cols-[12rem_minmax(0,1fr)] sm:items-center">
          <div>
            <h2 className="text-sm font-medium">本地数据</h2>
            <p className="mt-1 text-xs text-muted-foreground">账户档案仅保存在本机</p>
          </div>
          <div className="flex items-center gap-3">
            <Button
              variant="outline"
              type="button"
              onClick={() => void invoke('reveal_data_directory')}
            >
              <FolderOpen data-icon="inline-start" /> 打开数据目录
            </Button>
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Database /> SQLite
            </span>
          </div>
        </section>
      </div>
    </PageShell>
  );
}

export function BillingPage() {
  return (
    <PageShell>
      <PageHeader title="计费" />
    </PageShell>
  );
}

export function AboutPage() {
  return (
    <PageShell>
      <PageHeader title="关于" />
    </PageShell>
  );
}
