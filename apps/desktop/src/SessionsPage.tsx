import { invoke } from '@tauri-apps/api/core';
import {
  Archive,
  ArchiveRestore,
  LoaderCircle,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  RefreshCw,
  Search,
  Trash2,
  TriangleAlert,
} from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from './components/page-shell';
import { Alert, AlertAction, AlertDescription, AlertTitle } from './components/ui/alert';
import { Badge } from './components/ui/badge';
import { Button } from './components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './components/ui/dropdown-menu';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from './components/ui/input-group';
import { Tabs, TabsContent, TabsList, TabsTrigger } from './components/ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from './components/ui/tooltip';
import { appError } from './utils';

type CodexSession = {
  id: string;
  name: string | null;
  preview: string;
  cwd: string;
  source: string;
  createdAt: number;
  updatedAt: number;
};

type CodexSessionPage = {
  sessions: CodexSession[];
  nextCursor: string | null;
};

type PendingDialog =
  | { kind: 'rename'; session: CodexSession; name: string }
  | { kind: 'delete'; session: CodexSession }
  | null;

export default function SessionsPage() {
  const [archived, setArchived] = useState(false);
  const [query, setQuery] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [sessions, setSessions] = useState<CodexSession[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState<'list' | 'more' | null>('list');
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<PendingDialog>(null);
  const requestId = useRef(0);

  const loadSessions = useCallback(
    async (cursor: string | null = null) => {
      const append = cursor !== null;
      const currentRequest = ++requestId.current;
      setLoading(append ? 'more' : 'list');
      if (!append) setError(null);
      try {
        const page = await invoke<CodexSessionPage>('list_codex_sessions', {
          cursor,
          archived,
          searchTerm: searchTerm || null,
        });
        if (currentRequest !== requestId.current) return;
        setSessions((current) => (append ? [...current, ...page.sessions] : page.sessions));
        setNextCursor(page.nextCursor);
      } catch (caught) {
        if (currentRequest !== requestId.current) return;
        const message = appError(caught);
        setError(message);
        toast.error(message);
      } finally {
        if (currentRequest === requestId.current) setLoading(null);
      }
    },
    [archived, searchTerm],
  );

  useEffect(() => {
    setSessions([]);
    setNextCursor(null);
    void loadSessions();
  }, [loadSessions]);

  function submitSearch(event: FormEvent) {
    event.preventDefault();
    const next = query.trim();
    if (next === searchTerm) void loadSessions();
    else setSearchTerm(next);
  }

  async function moveSession(session: CodexSession) {
    const command = archived ? 'restore_codex_session' : 'archive_codex_session';
    setBusyId(session.id);
    try {
      await invoke(command, { sessionId: session.id });
      setSessions((current) => current.filter((item) => item.id !== session.id));
      toast.success(archived ? '会话已恢复。' : '会话已归档。');
    } catch (caught) {
      toast.error(appError(caught));
    } finally {
      setBusyId(null);
    }
  }

  async function renameSession(event: FormEvent) {
    event.preventDefault();
    if (dialog?.kind !== 'rename') return;
    const name = dialog.name.trim();
    if (!name) return;
    setBusyId(dialog.session.id);
    try {
      await invoke('rename_codex_session', { sessionId: dialog.session.id, name });
      setSessions((current) =>
        current.map((session) =>
          session.id === dialog.session.id ? { ...session, name } : session,
        ),
      );
      setDialog(null);
      toast.success('会话已重命名。');
    } catch (caught) {
      toast.error(appError(caught));
    } finally {
      setBusyId(null);
    }
  }

  async function deleteSession() {
    if (dialog?.kind !== 'delete') return;
    setBusyId(dialog.session.id);
    try {
      await invoke('delete_codex_session', { sessionId: dialog.session.id });
      setSessions((current) => current.filter((session) => session.id !== dialog.session.id));
      setDialog(null);
      toast.success('会话已永久删除。');
    } catch (caught) {
      toast.error(appError(caught));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title="会话"
        actions={
          <Tooltip>
            <TooltipTrigger render={<span className="inline-flex" />}>
              <Button
                variant="ghost"
                size="icon"
                type="button"
                onClick={() => void loadSessions()}
                disabled={loading === 'list'}
              >
                <RefreshCw className={loading === 'list' ? 'animate-spin' : ''} />
                <span className="sr-only">刷新会话</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>刷新会话</TooltipContent>
          </Tooltip>
        }
      />

      <Tabs
        className="mt-5 min-h-0 flex-1 gap-0"
        value={archived ? 'archived' : 'active'}
        onValueChange={(value) => setArchived(value === 'archived')}
      >
        <div className="flex items-center justify-between gap-4 border-b border-border px-4 pb-4 sm:px-8 lg:px-12">
          <TabsList>
            <TabsTrigger value="active">当前</TabsTrigger>
            <TabsTrigger value="archived">已归档</TabsTrigger>
          </TabsList>
          <form className="w-full max-w-72" onSubmit={submitSearch}>
            <InputGroup>
              <InputGroupInput
                aria-label="搜索会话标题"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="搜索会话标题"
                spellCheck={false}
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton size="icon-xs" type="submit" aria-label="搜索">
                  <Search />
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
          </form>
        </div>

        <TabsContent value={archived ? 'archived' : 'active'} className="min-h-0 overflow-y-auto">
          {error && !sessions.length ? (
            <div className="px-4 py-6 sm:px-8 lg:px-12">
              <Alert variant="destructive">
                <TriangleAlert />
                <AlertTitle>无法读取会话</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
                <AlertAction>
                  <Button variant="outline" size="sm" onClick={() => void loadSessions()}>
                    重试
                  </Button>
                </AlertAction>
              </Alert>
            </div>
          ) : loading === 'list' && !sessions.length ? (
            <EmptyState loading />
          ) : sessions.length ? (
            <>
              <ul className="w-full">
                {sessions.map((session) => (
                  <SessionRow
                    key={session.id}
                    session={session}
                    archived={archived}
                    busy={busyId === session.id}
                    onRename={() =>
                      setDialog({
                        kind: 'rename',
                        session,
                        name: session.name ?? session.preview,
                      })
                    }
                    onMove={() => void moveSession(session)}
                    onDelete={() => setDialog({ kind: 'delete', session })}
                  />
                ))}
              </ul>
              {nextCursor && (
                <div className="flex justify-center px-4 py-5 sm:px-8 lg:px-12">
                  <Button
                    variant="outline"
                    onClick={() => void loadSessions(nextCursor)}
                    disabled={loading === 'more'}
                  >
                    {loading === 'more' && <LoaderCircle className="animate-spin" />}
                    加载更多
                  </Button>
                </div>
              )}
            </>
          ) : (
            <EmptyState archived={archived} searching={Boolean(searchTerm)} />
          )}
        </TabsContent>
      </Tabs>

      <SessionDialog
        dialog={dialog}
        busy={dialog ? busyId === dialog.session.id : false}
        onChange={setDialog}
        onRename={renameSession}
        onDelete={() => void deleteSession()}
      />
    </PageShell>
  );
}

function SessionRow({
  session,
  archived,
  busy,
  onRename,
  onMove,
  onDelete,
}: {
  session: CodexSession;
  archived: boolean;
  busy: boolean;
  onRename: () => void;
  onMove: () => void;
  onDelete: () => void;
}) {
  const title = sessionTitle(session);
  const showPreview = Boolean(session.name && session.preview.trim() && session.preview !== title);
  return (
    <li className="flex min-h-20 items-center gap-3 border-b border-border px-4 py-3 sm:px-8 lg:px-12">
      <span className="grid size-9 shrink-0 place-items-center rounded-md bg-secondary text-secondary-foreground">
        <MessageSquare className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <strong className="truncate text-sm font-medium">{title}</strong>
          <Badge variant="secondary">{sourceLabel(session.source)}</Badge>
        </div>
        <p className="mt-1 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          {showPreview && <span className="max-w-52 truncate">{session.preview}</span>}
          <span className="truncate" title={session.cwd}>
            {session.cwd}
          </span>
        </p>
      </div>
      <time className="hidden shrink-0 text-xs text-muted-foreground sm:block">
        {formatSessionTime(session.updatedAt)}
      </time>
      <DropdownMenu>
        <DropdownMenuTrigger
          disabled={busy}
          render={<Button variant="ghost" size="icon-sm" aria-label="会话操作" />}
        >
          {busy ? <LoaderCircle className="animate-spin" /> : <MoreHorizontal />}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuGroup>
            <DropdownMenuItem onClick={onRename}>
              <Pencil /> 重命名
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onMove}>
              {archived ? <ArchiveRestore /> : <Archive />}
              {archived ? '恢复' : '归档'}
            </DropdownMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuItem variant="destructive" onClick={onDelete}>
              <Trash2 /> 永久删除
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </li>
  );
}

function SessionDialog({
  dialog,
  busy,
  onChange,
  onRename,
  onDelete,
}: {
  dialog: PendingDialog;
  busy: boolean;
  onChange: (dialog: PendingDialog) => void;
  onRename: (event: FormEvent) => void;
  onDelete: () => void;
}) {
  return (
    <Dialog open={dialog !== null} onOpenChange={(open) => !open && !busy && onChange(null)}>
      <DialogContent initialFocus={false}>
        {dialog?.kind === 'rename' ? (
          <form onSubmit={onRename}>
            <DialogHeader>
              <DialogTitle>重命名会话</DialogTitle>
            </DialogHeader>
            <InputGroup className="mt-4">
              <InputGroupInput
                aria-label="会话名称"
                value={dialog.name}
                onChange={(event) => onChange({ ...dialog, name: event.target.value })}
                disabled={busy}
                required
              />
            </InputGroup>
            <DialogFooter className="mt-4">
              <CancelButton disabled={busy} />
              <Button type="submit" disabled={busy || !dialog.name.trim()}>
                {busy && <LoaderCircle className="animate-spin" />}
                保存
              </Button>
            </DialogFooter>
          </form>
        ) : dialog?.kind === 'delete' ? (
          <div>
            <DialogHeader>
              <DialogTitle>永久删除会话</DialogTitle>
              <DialogDescription>
                将永久删除“{sessionTitle(dialog.session)}”，此操作无法撤销。
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="mt-4">
              <CancelButton disabled={busy} />
              <Button variant="destructive" onClick={onDelete} disabled={busy}>
                {busy && <LoaderCircle className="animate-spin" />}
                永久删除
              </Button>
            </DialogFooter>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function CancelButton({ disabled }: { disabled: boolean }) {
  return (
    <DialogClose
      disabled={disabled}
      render={
        <Button variant="ghost" type="button" onMouseDown={(event) => event.preventDefault()} />
      }
    >
      取消
    </DialogClose>
  );
}

function EmptyState({
  loading = false,
  archived = false,
  searching = false,
}: {
  loading?: boolean;
  archived?: boolean;
  searching?: boolean;
}) {
  return (
    <div className="flex min-h-52 flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
      {loading ? <LoaderCircle className="animate-spin" /> : <MessageSquare />}
      {loading
        ? '正在读取会话'
        : searching
          ? '没有匹配的会话'
          : archived
            ? '暂无已归档会话'
            : '暂无会话'}
    </div>
  );
}

function sourceLabel(source: string) {
  return (
    {
      cli: 'CLI',
      vscode: 'VS Code',
      appServer: 'App',
      unknown: '其他',
    }[source] ?? '其他'
  );
}

function sessionTitle(session: CodexSession) {
  return session.name?.trim() || session.preview.trim() || '未命名会话';
}

function formatSessionTime(value: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(value);
}
