import { invoke } from '@tauri-apps/api/core';
import { Check, FolderOpen, LoaderCircle } from 'lucide-react';
import { type FormEvent, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Button } from '../../components/ui/button';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '../../components/ui/input-group';
import { Switch } from '../../components/ui/switch';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { appError, authFileParent } from '../../utils';

type AppStatus = {
  authPath: string;
  autostartEnabled: boolean;
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

  async function openCodexHome() {
    setBusy('open');
    try {
      const openedInVsCode = await invoke<boolean>('open_codex_home', { codexHome });
      if (!openedInVsCode) toast.info('未找到 VS Code，已在文件管理器中打开。');
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      <div className="w-full px-4 pt-6 pb-10 sm:px-8 lg:px-12">
        <section className="grid gap-4 pb-6 sm:grid-cols-[12rem_minmax(0,1fr)]">
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
                      type="button"
                      onClick={() => void openCodexHome()}
                      disabled={busy === 'open' || busy === 'loading'}
                    >
                      {busy === 'open' ? <LoaderCircle className="animate-spin" /> : <FolderOpen />}
                      <span className="sr-only">在 VS Code 中打开</span>
                    </InputGroupButton>
                  </TooltipTrigger>
                  <TooltipContent>在 VS Code 中打开</TooltipContent>
                </Tooltip>
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

        <section className="grid gap-4 border-t border-border pt-6 sm:grid-cols-[12rem_minmax(0,1fr)] sm:items-center">
          <div>
            <h2 className="text-sm font-medium">本地数据</h2>
            <p className="mt-1 text-xs text-muted-foreground">账户档案仅保存在本机</p>
          </div>
          <Button
            className="w-fit"
            variant="outline"
            type="button"
            onClick={() => void invoke('reveal_data_directory')}
          >
            <FolderOpen data-icon="inline-start" /> 打开数据目录
          </Button>
        </section>
      </div>
    </>
  );
}

export function AboutPage() {
  return (
    <PageShell>
      <PageHeader title="关于" />
      <AboutContent />
    </PageShell>
  );
}

function AboutContent() {
  return null;
}
