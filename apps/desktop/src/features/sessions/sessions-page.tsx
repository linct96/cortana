import { invoke } from '../../backend';
import { LoaderCircle, MessageSquare, RefreshCw, Search, TriangleAlert } from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Alert, AlertAction, AlertDescription, AlertTitle } from '../../components/ui/alert';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '../../components/ui/input-group';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../components/ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { appError } from '../../utils';
import { SessionRow, type CodexSession } from './session-row';

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
    async (
      targetArchived: boolean,
      targetSearchTerm: string,
      cursor: string | null = null,
      minimumDuration = 0,
    ) => {
      const append = cursor !== null;
      const currentRequest = ++requestId.current;
      const minimumWait = minimumDuration
        ? new Promise((resolve) => setTimeout(resolve, minimumDuration))
        : Promise.resolve();
      setLoading(append ? 'more' : 'list');
      if (!append) setError(null);
      try {
        const page = await invoke<CodexSessionPage>('list_codex_sessions', {
          cursor,
          archived: targetArchived,
          searchTerm: targetSearchTerm || null,
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
        await minimumWait;
        if (currentRequest === requestId.current) setLoading(null);
      }
    },
    [],
  );

  useEffect(() => {
    void loadSessions(false, '');
    return () => {
      requestId.current += 1;
    };
  }, [loadSessions]);

  function startQuery(targetArchived: boolean, targetSearchTerm: string) {
    setSessions([]);
    setNextCursor(null);
    void loadSessions(targetArchived, targetSearchTerm);
  }

  function changeArchived(next: boolean) {
    if (next === archived) return;
    setArchived(next);
    startQuery(next, searchTerm);
  }

  function submitSearch(event: FormEvent) {
    event.preventDefault();
    const next = query.trim();
    if (next !== searchTerm) setSearchTerm(next);
    startQuery(archived, next);
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
                onClick={() => void loadSessions(archived, searchTerm, null, 400)}
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
      <SessionsContent
        archived={archived}
        query={query}
        searchTerm={searchTerm}
        sessions={sessions}
        nextCursor={nextCursor}
        loading={loading}
        busyId={busyId}
        error={error}
        dialog={dialog}
        onArchivedChange={changeArchived}
        onQueryChange={setQuery}
        onSearch={submitSearch}
        onLoad={(cursor) => loadSessions(archived, searchTerm, cursor)}
        onMove={moveSession}
        onDialogChange={setDialog}
        onRename={renameSession}
        onDelete={deleteSession}
      />
    </PageShell>
  );
}

function SessionsContent({
  archived,
  query,
  searchTerm,
  sessions,
  nextCursor,
  loading,
  busyId,
  error,
  dialog,
  onArchivedChange,
  onQueryChange,
  onSearch,
  onLoad,
  onMove,
  onDialogChange,
  onRename,
  onDelete,
}: {
  archived: boolean;
  query: string;
  searchTerm: string;
  sessions: CodexSession[];
  nextCursor: string | null;
  loading: 'list' | 'more' | null;
  busyId: string | null;
  error: string | null;
  dialog: PendingDialog;
  onArchivedChange: (archived: boolean) => void;
  onQueryChange: (query: string) => void;
  onSearch: (event: FormEvent) => void;
  onLoad: (cursor?: string | null) => Promise<void>;
  onMove: (session: CodexSession) => Promise<void>;
  onDialogChange: (dialog: PendingDialog) => void;
  onRename: (event: FormEvent) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  return (
    <>
      <Tabs
        className="mt-5 min-h-0 flex-1 gap-0"
        value={archived ? 'archived' : 'active'}
        onValueChange={(value) => onArchivedChange(value === 'archived')}
      >
        <div className="flex items-center justify-between gap-4 border-b border-border px-4 pb-4 sm:px-8 lg:px-12">
          <TabsList>
            <TabsTrigger value="active">当前</TabsTrigger>
            <TabsTrigger value="archived">已归档</TabsTrigger>
          </TabsList>
          <form className="w-full max-w-72" onSubmit={onSearch}>
            <InputGroup>
              <InputGroupInput
                aria-label="搜索会话标题"
                value={query}
                onChange={(event) => onQueryChange(event.target.value)}
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
                  <Button variant="outline" size="sm" onClick={() => void onLoad()}>
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
                      onDialogChange({
                        kind: 'rename',
                        session,
                        name: session.name ?? session.preview,
                      })
                    }
                    onMove={() => void onMove(session)}
                    onDelete={() => onDialogChange({ kind: 'delete', session })}
                  />
                ))}
              </ul>
              {nextCursor && (
                <div className="flex justify-center px-4 py-5 sm:px-8 lg:px-12">
                  <Button
                    variant="outline"
                    onClick={() => void onLoad(nextCursor)}
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
        onChange={onDialogChange}
        onRename={onRename}
        onDelete={() => void onDelete()}
      />
    </>
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
                将永久删除“
                {dialog.session.name?.trim() || dialog.session.preview.trim() || '未命名会话'}
                ”，此操作无法撤销。
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
