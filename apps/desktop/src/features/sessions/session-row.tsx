import {
  Archive,
  ArchiveRestore,
  LoaderCircle,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  Trash2,
} from 'lucide-react';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';

export type SessionSummary = {
  id: string;
  name: string | null;
  preview: string;
  cwd: string | null;
  source: string | null;
  createdAt: number | null;
  updatedAt: number;
};

export type SessionCapabilities = {
  supportsArchived: boolean;
  canRename: boolean;
  canArchive: boolean;
  canDelete: boolean;
};

export function SessionRow({
  session,
  capabilities,
  archived,
  busy,
  onRename,
  onMove,
  onDelete,
}: {
  session: SessionSummary;
  capabilities: SessionCapabilities;
  archived: boolean;
  busy: boolean;
  onRename: () => void;
  onMove: () => void;
  onDelete: () => void;
}) {
  const title = session.name?.trim() || session.preview.trim() || '未命名会话';
  const showPreview = Boolean(session.name && session.preview.trim() && session.preview !== title);
  const source = session.source
    ? ({ cli: 'CLI', vscode: 'Codex App / VS Code', appServer: 'App', unknown: '其他' }[
        session.source
      ] ?? '其他')
    : null;
  const hasActions = capabilities.canRename || capabilities.canArchive || capabilities.canDelete;
  const updatedAt = new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(session.updatedAt);

  return (
    <li className="flex min-h-20 items-center gap-3 border-b border-border px-4 py-3 sm:px-8 lg:px-12">
      <span className="grid size-9 shrink-0 place-items-center rounded-md bg-secondary text-secondary-foreground">
        <MessageSquare className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <strong className="truncate text-sm font-medium">{title}</strong>
          {source && <Badge variant="secondary">{source}</Badge>}
        </div>
        <p className="mt-1 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          {showPreview && <span className="max-w-52 truncate">{session.preview}</span>}
          {session.cwd && (
            <span className="truncate" title={session.cwd}>
              {session.cwd}
            </span>
          )}
        </p>
      </div>
      <time className="hidden shrink-0 text-xs text-muted-foreground sm:block">{updatedAt}</time>
      {hasActions && (
        <DropdownMenu>
          <DropdownMenuTrigger
            disabled={busy}
            render={<Button variant="ghost" size="icon-sm" aria-label="会话操作" />}
          >
            {busy ? <LoaderCircle className="animate-spin" /> : <MoreHorizontal />}
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {(capabilities.canRename || capabilities.canArchive) && (
              <DropdownMenuGroup>
                {capabilities.canRename && (
                  <DropdownMenuItem onClick={onRename}>
                    <Pencil /> 重命名
                  </DropdownMenuItem>
                )}
                {capabilities.canArchive && (
                  <DropdownMenuItem onClick={onMove}>
                    {archived ? <ArchiveRestore /> : <Archive />}
                    {archived ? '恢复' : '归档'}
                  </DropdownMenuItem>
                )}
              </DropdownMenuGroup>
            )}
            {capabilities.canDelete && (
              <>
                {(capabilities.canRename || capabilities.canArchive) && <DropdownMenuSeparator />}
                <DropdownMenuGroup>
                  <DropdownMenuItem variant="destructive" onClick={onDelete}>
                    <Trash2 /> 永久删除
                  </DropdownMenuItem>
                </DropdownMenuGroup>
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </li>
  );
}
