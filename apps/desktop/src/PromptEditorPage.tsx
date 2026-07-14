import { Link, useBlocker, useNavigate, useParams } from '@tanstack/react-router';
import { invoke } from '@tauri-apps/api/core';
import CodeMirror, { EditorView } from '@uiw/react-codemirror';
import { ArrowLeft, LoaderCircle, Save } from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from './components/page-shell';
import { Badge } from './components/ui/badge';
import { Button, buttonVariants } from './components/ui/button';
import { Field, FieldGroup, FieldLabel } from './components/ui/field';
import { Input } from './components/ui/input';
import { ConfirmDialog } from './features/prompts/prompt-dialogs';
import type { AgentsProfile, AgentsStatus } from './features/prompts/types';
import { appError } from './utils';

const editorExtensions = [
  EditorView.lineWrapping,
  EditorView.contentAttributes.of({ 'aria-label': 'AGENTS.md 内容', spellcheck: 'false' }),
  EditorView.theme({
    '&': { backgroundColor: 'transparent', color: 'var(--foreground)', fontSize: '12px' },
    '&.cm-focused': { outline: 'none' },
    '.cm-content': {
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
      padding: '8px 0',
    },
    '.cm-gutters': {
      backgroundColor: 'var(--muted)',
      borderRight: '1px solid var(--border)',
      color: 'var(--muted-foreground)',
    },
    '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--accent)' },
  }),
];

export function NewPromptPage() {
  return <PromptEditorPage />;
}

export function EditPromptPage() {
  const { profileId } = useParams({ from: '/prompts/edit/$profileId' });
  return <PromptEditorPage profileId={profileId} />;
}

function PromptEditorPage({ profileId }: { profileId?: string }) {
  const navigate = useNavigate();
  const [profile, setProfile] = useState<AgentsProfile | null>(null);
  const [name, setName] = useState('');
  const [content, setContent] = useState('');
  const [savedName, setSavedName] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [busy, setBusy] = useState(Boolean(profileId));
  const [forceSave, setForceSave] = useState(false);
  const dirty = name !== savedName || content !== savedContent;
  const blocker = useBlocker({
    shouldBlockFn: useCallback(() => dirty, [dirty]),
    enableBeforeUnload: dirty,
    withResolver: true,
  });

  useEffect(() => {
    setBusy(true);
    invoke<AgentsStatus>('get_agents_status')
      .then((status) => {
        if (!profileId) return;
        const next = status.profiles.find((item) => item.id === profileId);
        if (!next) throw new Error('提示词方案不存在。');
        setProfile(next);
        setName(next.name);
        setContent(next.content);
        setSavedName(next.name);
        setSavedContent(next.content);
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(false));
  }, [profileId]);

  async function save(event?: FormEvent, force = false) {
    event?.preventDefault();
    setBusy(true);
    try {
      if (profileId) {
        await invoke('update_agents_profile', { profileId, name, content, force });
        setSavedName(name.trim());
        setName(name.trim());
        setSavedContent(content);
        setForceSave(false);
        toast.success('提示词已保存。');
      } else {
        await invoke('create_agents_profile', { name, content });
        toast.success('提示词已创建。');
        await navigate({ to: '/prompts', ignoreBlocker: true });
      }
    } catch (error) {
      const message = appError(error);
      if (!force && message.includes('工具外')) setForceSave(true);
      else toast.error(message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title={profileId ? '编辑提示词' : '新建提示词'}
        leading={
          <Link
            to="/prompts"
            aria-label="返回提示词列表"
            className={buttonVariants({ variant: 'ghost', size: 'icon' })}
          >
            <ArrowLeft />
          </Link>
        }
        actions={profile?.isActive ? <Badge>使用中</Badge> : undefined}
      />
      <PromptEditorContent
        name={name}
        content={content}
        busy={busy}
        dirty={dirty}
        forceSave={forceSave}
        navigationBlocked={blocker.status === 'blocked'}
        onNameChange={setName}
        onContentChange={setContent}
        onSave={save}
        onCloseForce={() => setForceSave(false)}
        onResetNavigation={() => blocker.reset?.()}
        onProceedNavigation={() => blocker.proceed?.()}
      />
    </PageShell>
  );
}

function PromptEditorContent({
  name,
  content,
  busy,
  dirty,
  forceSave,
  navigationBlocked,
  onNameChange,
  onContentChange,
  onSave,
  onCloseForce,
  onResetNavigation,
  onProceedNavigation,
}: {
  name: string;
  content: string;
  busy: boolean;
  dirty: boolean;
  forceSave: boolean;
  navigationBlocked: boolean;
  onNameChange: (name: string) => void;
  onContentChange: (content: string) => void;
  onSave: (event?: FormEvent, force?: boolean) => Promise<void>;
  onCloseForce: () => void;
  onResetNavigation: () => void;
  onProceedNavigation: () => void;
}) {
  return (
    <>
      <form
        className="flex min-h-0 flex-1 flex-col px-4 pb-6 sm:px-8 sm:pb-7 lg:px-12"
        onSubmit={(event) => void onSave(event)}
      >
        <FieldGroup className="min-h-0 flex-1 gap-4">
          <Field className="shrink-0">
            <FieldLabel htmlFor="prompt-name">名称</FieldLabel>
            <Input
              id="prompt-name"
              value={name}
              onChange={(event) => onNameChange(event.target.value)}
              disabled={busy}
            />
          </Field>
          <Field className="min-h-0 flex-1">
            <FieldLabel>AGENTS.md</FieldLabel>
            <CodeMirror
              className="min-h-0 flex-1 overflow-hidden rounded-lg border border-input focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50"
              height="100%"
              value={content}
              onChange={onContentChange}
              extensions={editorExtensions}
              editable={!busy}
              readOnly={busy}
              theme="none"
            />
          </Field>
        </FieldGroup>
        <div className="mt-4 flex shrink-0 justify-end">
          <Button type="submit" disabled={busy || !dirty || !name.trim()}>
            {busy ? (
              <LoaderCircle data-icon="inline-start" className="animate-spin" />
            ) : (
              <Save data-icon="inline-start" />
            )}
            保存
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={forceSave}
        title="覆盖工具外修改"
        description="继续保存会用当前内容覆盖 AGENTS.md。"
        confirmLabel="强制覆盖"
        busy={busy}
        onClose={onCloseForce}
        onConfirm={() => void onSave(undefined, true)}
      />
      <ConfirmDialog
        open={navigationBlocked}
        title="离开编辑页面"
        description="当前提示词的修改尚未保存。"
        confirmLabel="放弃并离开"
        onClose={onResetNavigation}
        onConfirm={onProceedNavigation}
      />
    </>
  );
}
