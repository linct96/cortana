import { Link, useNavigate } from '@tanstack/react-router';
import { Box, Ellipsis, LoaderCircle, Pencil, Plus, Trash2, TriangleAlert } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { invoke } from '../../backend';
import { useAppShell } from '../../components/app-shell-context';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Alert, AlertAction, AlertDescription, AlertTitle } from '../../components/ui/alert';
import { Button } from '../../components/ui/button';
import { buttonVariants } from '../../components/ui/button-variants';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '../../components/ui/empty';
import { appError } from '../../utils';
import { ConfirmDialog } from '../prompts/prompt-dialogs';
import type { ModelProfile, ModelProfilesStatus } from './types';

export default function ModelsPage() {
  const navigate = useNavigate();
  const { activeProduct } = useAppShell();
  const [status, setStatus] = useState<ModelProfilesStatus | null>(null);
  const [busy, setBusy] = useState('load');
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<ModelProfile | null>(null);

  const load = useCallback(async () => {
    if (activeProduct !== 'codex' && activeProduct !== 'claude' && activeProduct !== 'grok') {
      await navigate({ to: '/accounts' });
      return;
    }
    setError(null);
    try {
      setStatus(
        await invoke<ModelProfilesStatus>('get_model_profiles_status', {
          product: activeProduct,
        }),
      );
    } catch (caught) {
      setError(appError(caught));
    } finally {
      setBusy('');
    }
  }, [activeProduct, navigate]);

  useEffect(() => void load(), [load]);

  async function deleteProfile() {
    if (!deleting) return;
    setBusy(`delete:${deleting.id}`);
    try {
      await invoke('delete_model_profile', { product: activeProduct, profileId: deleting.id });
      toast.success(`已删除 ${deleting.name}。`);
      setDeleting(null);
      await load();
    } catch (caught) {
      toast.error(appError(caught));
    } finally {
      setBusy('');
    }
  }

  const hasRows = Boolean(status?.profiles.length);

  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title="自定义模型"
        actions={
          <Link to="/models/new" className={buttonVariants()}>
            <Plus data-icon="inline-start" /> 新建方案
          </Link>
        }
      />
      <section className="min-h-0 flex-1 overflow-y-auto">
        {busy === 'load' ? (
          <ModelEmpty title="正在读取模型方案" loading />
        ) : error && !status ? (
          <div className="px-4 py-6 sm:px-8 lg:px-12">
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>无法读取模型方案</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
              <AlertAction>
                <Button variant="outline" size="sm" onClick={() => void load()}>
                  重试
                </Button>
              </AlertAction>
            </Alert>
          </div>
        ) : hasRows ? (
          <div className="flex flex-col gap-3 px-4 pt-6 pb-10 sm:px-8 lg:px-12">
            {status?.profiles.map((profile) => (
              <ModelRow
                key={profile.id}
                name={profile.name}
                description={`${profile.models.length} 个模型 · ${profile.assignments.length} 个账号`}
                profile={profile}
                onDelete={() => setDeleting(profile)}
              />
            ))}
          </div>
        ) : (
          <ModelEmpty title="还没有模型方案">
            <Link to="/models/new" className={buttonVariants({ variant: 'secondary' })}>
              <Plus data-icon="inline-start" /> 新建方案
            </Link>
          </ModelEmpty>
        )}
      </section>
      <ConfirmDialog
        open={Boolean(deleting)}
        title="删除模型方案"
        description={`确定删除“${deleting?.name ?? ''}”吗？`}
        confirmLabel="删除"
        destructive
        busy={busy === `delete:${deleting?.id}`}
        onClose={() => setDeleting(null)}
        onConfirm={() => void deleteProfile()}
      />
    </PageShell>
  );
}

function ModelRow({
  name,
  description,
  profile,
  onDelete,
}: {
  name: string;
  description: string;
  profile?: ModelProfile;
  onDelete?: () => void;
}) {
  return (
    <article className="group flex h-[68px] items-center gap-3 rounded-md border bg-card px-3 py-3 transition-colors hover:border-primary/30">
      <div className="grid size-9 shrink-0 place-items-center rounded-full bg-secondary text-primary">
        <Box size={18} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-0.5 flex items-center gap-2">
          <strong className="truncate text-sm font-medium">{name}</strong>
        </div>
        <p className="truncate text-xs text-muted-foreground">{description}</p>
      </div>
      <div className="pointer-events-none flex justify-end gap-1 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100">
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon" type="button" />}>
            <Ellipsis />
            <span className="sr-only">更多</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              <DropdownMenuItem
                render={
                  <Link to="/models/edit/$profileId" params={{ profileId: profile?.id ?? '' }} />
                }
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

function ModelEmpty({
  title,
  loading,
  children,
}: {
  title: string;
  loading?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <Empty className="min-h-52 rounded-none border-y">
      <EmptyHeader>
        <EmptyMedia variant={loading ? undefined : 'icon'}>
          {loading ? <LoaderCircle className="animate-spin" /> : <Box />}
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </Empty>
  );
}
