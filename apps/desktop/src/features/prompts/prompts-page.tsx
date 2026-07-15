import { Link } from '@tanstack/react-router';
import { invoke } from '@tauri-apps/api/core';
import {
  Ellipsis,
  FileText,
  LoaderCircle,
  Pencil,
  Plus,
  Power,
  Trash2,
  TriangleAlert,
  Upload,
} from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Alert, AlertAction, AlertDescription, AlertTitle } from '../../components/ui/alert';
import { Badge } from '../../components/ui/badge';
import { Button, buttonVariants } from '../../components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import { appError, cn } from '../../utils';
import { ConfirmDialog, NameDialogForm } from './prompt-dialogs';
import type { AgentsProfile, AgentsStatus } from './types';

export default function PromptsPage() {
  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title="提示词管理"
        actions={
          <Link to="/prompts/new" className={buttonVariants()}>
            <Plus data-icon="inline-start" /> 新建提示词
          </Link>
        }
      />
      <PromptsContent />
    </PageShell>
  );
}

function PromptsContent() {
  const [status, setStatus] = useState<AgentsStatus | null>(null);
  const [busy, setBusy] = useState<string | null>('load');
  const [importing, setImporting] = useState(false);
  const [importName, setImportName] = useState('当前方案');
  const [deleting, setDeleting] = useState<AgentsProfile | null>(null);
  const [forceProfile, setForceProfile] = useState<AgentsProfile | null>(null);

  const load = useCallback(async () => {
    setStatus(await invoke<AgentsStatus>('get_agents_status'));
  }, []);

  useEffect(() => {
    load()
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(null));
  }, [load]);

  async function importCurrent(event: FormEvent) {
    event.preventDefault();
    setBusy('import');
    try {
      await invoke('import_current_agents', { name: importName });
      setImporting(false);
      toast.success('当前文件已同步。');
      await load();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function activateProfile(profile: AgentsProfile, force = false) {
    setBusy(`activate:${profile.id}`);
    try {
      await invoke('activate_agents_profile', { profileId: profile.id, force });
      setForceProfile(null);
      toast.success(`已启用 ${profile.name}。`);
      await load();
    } catch (error) {
      const message = appError(error);
      if (!force && message.includes('未纳管')) setForceProfile(profile);
      else toast.error(message);
    } finally {
      setBusy(null);
    }
  }

  async function deleteProfile() {
    if (!deleting) return;
    setBusy(`delete:${deleting.id}`);
    try {
      await invoke('delete_agents_profile', { profileId: deleting.id });
      toast.success(`已删除 ${deleting.name}。`);
      setDeleting(null);
      await load();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      {status && status.fileState !== 'managed' && status.fileState !== 'missing' && (
        <div className="shrink-0 px-4 pb-5 sm:px-8 lg:px-12">
          <Alert className="pr-40">
            <TriangleAlert />
            <AlertTitle>AGENTS.md 尚未纳管</AlertTitle>
            <AlertDescription className="truncate" title={status.path}>
              {status.path}
            </AlertDescription>
            <AlertAction>
              <Button variant="outline" size="sm" onClick={() => setImporting(true)}>
                <Upload data-icon="inline-start" /> 同步当前文件
              </Button>
            </AlertAction>
          </Alert>
        </div>
      )}

      <section className="min-h-0 flex-1 overflow-y-auto">
        {busy === 'load' ? (
          <div className="grid min-h-52 place-items-center border-y text-sm text-muted-foreground">
            <span className="flex items-center gap-2">
              <LoaderCircle className="animate-spin" /> 正在读取提示词
            </span>
          </div>
        ) : status?.profiles.length ? (
          <div className="flex w-full flex-col gap-3 px-4 pt-6 pb-10 sm:px-8 lg:px-12">
            {status.profiles.map((profile) => (
              <PromptRow
                key={profile.id}
                profile={profile}
                busy={busy === `activate:${profile.id}`}
                onActivate={() => void activateProfile(profile)}
                onDelete={() => setDeleting(profile)}
              />
            ))}
          </div>
        ) : (
          <div className="flex min-h-52 flex-col items-center justify-center gap-3 border-y text-sm text-muted-foreground">
            <div className="grid size-11 place-items-center rounded-full bg-secondary text-primary">
              <FileText size={22} />
            </div>
            <strong className="text-sm font-medium text-foreground">还没有提示词方案</strong>
            <Link to="/prompts/new" className={buttonVariants({ variant: 'secondary' })}>
              <Plus data-icon="inline-start" /> 新建提示词
            </Link>
          </div>
        )}
      </section>

      <NameDialogForm
        kind={importing ? 'import' : null}
        name={importName}
        busy={busy === 'import'}
        onNameChange={setImportName}
        onClose={() => setImporting(false)}
        onSubmit={importCurrent}
      />
      <ConfirmDialog
        open={Boolean(deleting)}
        title="删除提示词"
        description={
          deleting?.isActive
            ? '方案将从 Cortana 删除，当前 AGENTS.md 会保留为未纳管状态。'
            : `确定删除“${deleting?.name ?? ''}”吗？`
        }
        confirmLabel="删除"
        destructive
        busy={busy === `delete:${deleting?.id}`}
        onClose={() => setDeleting(null)}
        onConfirm={() => void deleteProfile()}
      />
      <ConfirmDialog
        open={Boolean(forceProfile)}
        title="覆盖未纳管文件"
        description="继续启用会用所选方案覆盖当前 AGENTS.md。"
        confirmLabel="强制覆盖"
        busy={Boolean(busy)}
        onClose={() => setForceProfile(null)}
        onConfirm={() => forceProfile && void activateProfile(forceProfile, true)}
      />
    </>
  );
}

function PromptRow({
  profile,
  busy,
  onActivate,
  onDelete,
}: {
  profile: AgentsProfile;
  busy: boolean;
  onActivate: () => void;
  onDelete: () => void;
}) {
  const preview = profile.content
    .split('\n')
    .map((line) => line.trim())
    .find(Boolean);

  return (
    <article
      className={cn(
        'group flex h-[68px] items-center gap-3 rounded-md border border-border bg-card px-3 py-3 transition-[border-color,background-color] hover:border-primary/30',
        profile.isActive && 'bg-primary/5',
      )}
    >
      <div className="grid size-9 shrink-0 place-items-center rounded-full bg-secondary text-primary">
        <FileText size={18} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-0.5 flex items-center gap-2">
          <strong className="truncate text-sm font-medium">{profile.name}</strong>
          {profile.isActive && <Badge>使用中</Badge>}
        </div>
        <p className="truncate text-xs text-muted-foreground">{preview || '空白提示词'}</p>
      </div>
      <div className="pointer-events-none flex min-w-18 justify-end gap-1 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100">
        {!profile.isActive && (
          <Button size="icon" type="button" onClick={onActivate} disabled={busy}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Power />}
            <span className="sr-only">启用</span>
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon" type="button" />}>
            <Ellipsis />
            <span className="sr-only">更多</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              <DropdownMenuItem
                render={<Link to="/prompts/edit/$profileId" params={{ profileId: profile.id }} />}
              >
                <Pencil /> 编辑
              </DropdownMenuItem>
              <DropdownMenuItem variant="destructive" onClick={onDelete}>
                <Trash2 /> 删除
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </article>
  );
}
