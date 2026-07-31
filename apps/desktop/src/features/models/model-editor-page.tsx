import { Link, useBlocker, useNavigate, useParams } from '@tanstack/react-router';
import {
  ArrowLeft,
  Download,
  ListPlus,
  LoaderCircle,
  Pencil,
  Plus,
  Save,
  Trash2,
} from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { invoke } from '../../backend';
import { useAppShell } from '../../components/app-shell-context';
import { CreatableCombobox } from '../../components/creatable-combobox';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Button } from '../../components/ui/button';
import { buttonVariants } from '../../components/ui/button-variants';
import { Card, CardAction, CardContent, CardHeader, CardTitle } from '../../components/ui/card';
import { Checkbox } from '../../components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Field, FieldLabel } from '../../components/ui/field';
import { Input } from '../../components/ui/input';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Separator } from '../../components/ui/separator';
import { appError, cn } from '../../utils';
import { ConfirmDialog } from '../prompts/prompt-dialogs';
import {
  CLAUDE_MODEL_SLOTS,
  fillRelayModels,
  modelFormError,
  type ClaudeModelSlot,
  type ModelAssignment,
  type ModelEntry,
  type ModelProfilesStatus,
  type RelayModelOption,
  removeModelAt,
  uniqueModelsById,
} from './types';

export function NewModelPage() {
  return <ModelEditorPage />;
}

export function EditModelPage() {
  const { profileId } = useParams({ from: '/main/models/edit/$profileId' });
  return <ModelEditorPage profileId={profileId} />;
}

function ModelEditorPage({ profileId }: { profileId?: string }) {
  const navigate = useNavigate();
  const { activeProduct, setHasUnsavedChanges } = useAppShell();
  const modelProduct =
    activeProduct === 'claude' ? 'claude' : activeProduct === 'grok' ? 'grok' : 'codex';
  const [status, setStatus] = useState<ModelProfilesStatus | null>(null);
  const [name, setName] = useState('');
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [assignments, setAssignments] = useState<ModelAssignment[]>([]);
  const [saved, setSaved] = useState(JSON.stringify({ name: '', models: [], assignments: [] }));
  const [busy, setBusy] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [syncAccountId, setSyncAccountId] = useState<string | null>(null);
  const [relayModels, setRelayModels] = useState<RelayModelOption[]>([]);
  const [accountDialogOpen, setAccountDialogOpen] = useState(false);
  const [editingAccountId, setEditingAccountId] = useState<string | null>(null);
  const [pendingAccountId, setPendingAccountId] = useState<string | null>(null);
  const [pendingModelId, setPendingModelId] = useState<string | null>(null);
  const requestId = useRef(0);
  const syncRequestId = useRef(0);
  const snapshot = JSON.stringify({ name, models, assignments });
  const dirty = snapshot !== saved;
  const validationError = modelFormError(modelProduct, name, models, assignments);
  const modelOptions = uniqueModelsById(models);

  function linkedProfileName(accountId: string) {
    if (accountId === editingAccountId) return null;
    if (assignments.some((assignment) => assignment.accountId === accountId)) {
      return name.trim() || '当前方案';
    }
    return status?.profiles.find(
      (profile) =>
        profile.id !== profileId &&
        profile.assignments.some((assignment) => assignment.accountId === accountId),
    )?.name;
  }

  const availableAccounts =
    status?.relayAccounts.filter((account) => !linkedProfileName(account.accountId)) ?? [];
  const blocker = useBlocker({
    shouldBlockFn: useCallback(() => dirty, [dirty]),
    enableBeforeUnload: dirty,
    withResolver: true,
  });

  useEffect(() => {
    setHasUnsavedChanges(dirty);
    return () => setHasUnsavedChanges(false);
  }, [dirty, setHasUnsavedChanges]);

  useEffect(() => {
    if (activeProduct !== 'codex' && activeProduct !== 'claude' && activeProduct !== 'grok') {
      void navigate({ to: '/accounts' });
      return;
    }
    const currentRequest = ++requestId.current;
    syncRequestId.current += 1;
    setSyncAccountId(null);
    setRelayModels([]);
    setSyncing(false);
    setAccountDialogOpen(false);
    setEditingAccountId(null);
    setPendingAccountId(null);
    setPendingModelId(null);
    setBusy(true);
    invoke<ModelProfilesStatus>('get_model_profiles_status', { product: modelProduct })
      .then((nextStatus) => {
        if (currentRequest !== requestId.current) return;
        setStatus(nextStatus);
        const profile = profileId
          ? nextStatus.profiles.find((item) => item.id === profileId)
          : null;
        if (profileId && !profile) throw new Error('模型方案不存在。');
        const nextName = profile?.name ?? '';
        const nextModels = profile?.models ?? [emptyModel(modelProduct, [])];
        const nextAssignments = profile?.assignments ?? [];
        setName(nextName);
        setModels(nextModels);
        setAssignments(nextAssignments);
        setSaved(
          JSON.stringify({ name: nextName, models: nextModels, assignments: nextAssignments }),
        );
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => currentRequest === requestId.current && setBusy(false));
    return () => {
      requestId.current += 1;
      syncRequestId.current += 1;
    };
  }, [activeProduct, modelProduct, navigate, profileId]);

  function updateModel(index: number, patch: Partial<ModelEntry>) {
    setModels((current) =>
      current.map((model, modelIndex) => (modelIndex === index ? { ...model, ...patch } : model)),
    );
  }

  function removeModel(index: number) {
    const next = removeModelAt(models, assignments, index);
    setModels(next.models);
    setAssignments(next.assignments);
  }

  function saveAssignment() {
    const account = status?.relayAccounts.find((item) => item.accountId === pendingAccountId);
    if (!account || !pendingModelId) return;
    const nextAssignment = { ...account, defaultModelId: pendingModelId };
    setAssignments((current) =>
      editingAccountId
        ? current.map((item) => (item.accountId === editingAccountId ? nextAssignment : item))
        : [...current, nextAssignment],
    );
    setAccountDialogOpen(false);
    setEditingAccountId(null);
    setPendingAccountId(null);
    setPendingModelId(null);
  }

  async function syncRelayModels() {
    if (!syncAccountId) return;
    const currentRequest = ++syncRequestId.current;
    setSyncing(true);
    try {
      const result = await invoke<RelayModelOption[]>('fetch_relay_models', {
        product: modelProduct,
        accountId: syncAccountId,
      });
      if (currentRequest !== syncRequestId.current) return;
      setRelayModels(result);
      if (result.length) {
        toast.success(`已获取 ${result.length} 个模型。`);
      } else {
        toast.info('未获取到远端模型。');
      }
    } catch (error) {
      if (currentRequest === syncRequestId.current) toast.error(appError(error));
    } finally {
      if (currentRequest === syncRequestId.current) setSyncing(false);
    }
  }

  function setAllRelayModels() {
    setModels((current) => fillRelayModels(modelProduct, current, relayModels));
  }

  async function save(forceReassign = false, event?: FormEvent) {
    event?.preventDefault();
    const error = modelFormError(modelProduct, name, models, assignments);
    if (error) {
      toast.error(error);
      return;
    }
    setBusy(true);
    try {
      await invoke(profileId ? 'update_model_profile' : 'create_model_profile', {
        ...(profileId ? { profileId } : {}),
        product: modelProduct,
        name,
        models,
        assignments,
        forceReassign,
      });
      toast.success(profileId ? '模型方案已保存。' : '模型方案已创建。');
      await navigate({ to: '/models', ignoreBlocker: true });
    } catch (caught) {
      const message = appError(caught);
      if (
        !forceReassign &&
        message.includes('已关联其他模型方案') &&
        window.confirm(`${message}\n\n继续保存会将该账号改绑到当前方案。`)
      ) {
        await save(true);
      } else {
        toast.error(message);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageShell className="flex min-h-0 flex-col overflow-hidden">
      <PageHeader
        title={profileId ? '编辑模型方案' : '新建模型方案'}
        leading={
          <Link
            to="/models"
            aria-label="返回模型列表"
            className={buttonVariants({ variant: 'ghost', size: 'icon' })}
          >
            <ArrowLeft />
          </Link>
        }
      />
      <form
        className="min-h-0 flex-1 overflow-y-auto px-4 pb-8 sm:px-8 lg:px-12"
        onSubmit={(event) => void save(false, event)}
      >
        <section className="py-5">
          <Field className="max-w-xl">
            <FieldLabel htmlFor="model-profile-name">方案名称</FieldLabel>
            <Input
              id="model-profile-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={busy}
            />
          </Field>
        </section>
        <Separator />
        <section className="py-5">
          <div className="mb-3 flex items-center justify-between gap-3">
            <h2 className="text-sm font-semibold">模型列表</h2>
            <div className="flex gap-2">
              <Select
                value={syncAccountId}
                onValueChange={setSyncAccountId}
                disabled={busy || syncing}
              >
                <SelectTrigger className="w-36" size="sm" aria-label="选择远端同步账号">
                  <SelectValue className="min-w-0 truncate" placeholder="从账号导入">
                    {
                      status?.relayAccounts.find((account) => account.accountId === syncAccountId)
                        ?.accountAlias
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent className="w-auto min-w-(--anchor-width) max-w-80">
                  <SelectGroup>
                    {status?.relayAccounts.map((account) => (
                      <SelectItem key={account.accountId} value={account.accountId}>
                        <span className="block max-w-72 truncate" title={account.accountAlias}>
                          {account.accountAlias}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void syncRelayModels()}
                disabled={busy || syncing || !syncAccountId}
              >
                {syncing ? (
                  <LoaderCircle data-icon="inline-start" className="animate-spin" />
                ) : (
                  <Download data-icon="inline-start" />
                )}
                获取模型
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={setAllRelayModels}
                disabled={busy || syncing || !relayModels.length}
              >
                <ListPlus data-icon="inline-start" />
                一键设置
              </Button>
            </div>
          </div>
          <div className="overflow-hidden rounded-md border">
            <div
              className={cn(
                'grid gap-3 border-b bg-muted/50 px-3 py-2 text-xs font-medium text-muted-foreground',
                modelProduct === 'claude'
                  ? 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_108px_40px_36px]'
                  : modelProduct === 'codex'
                    ? 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_40px_36px]'
                    : 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_36px]',
              )}
            >
              <span>模型 ID</span>
              <span>显示名称</span>
              {modelProduct === 'claude' && <span>映射入口</span>}
              {modelProduct !== 'grok' && <span className="text-center">1M</span>}
              <span className="text-center">操作</span>
            </div>
            {models.length ? (
              models.map((model, index) => (
                <div
                  key={index}
                  className={cn(
                    'grid items-center gap-3 border-b p-3 last:border-b-0',
                    modelProduct === 'claude'
                      ? 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_108px_40px_36px]'
                      : modelProduct === 'codex'
                        ? 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_40px_36px]'
                        : 'grid-cols-[minmax(0,1fr)_minmax(0,1fr)_36px]',
                  )}
                >
                  {relayModels.length ? (
                    <CreatableCombobox
                      aria-label="模型 ID"
                      value={model.id}
                      items={relayModels}
                      itemToValue={(option) => option.id}
                      itemToLabel={(option) => option.displayName}
                      placeholder="glm-5.2"
                      onValueChange={(id) => updateModel(index, { id })}
                      onItemSelect={(option) =>
                        updateModel(index, {
                          id: option.id,
                          displayName: option.displayName,
                        })
                      }
                      renderItem={(option) => (
                        <div className="flex min-w-0 flex-1 flex-col items-start">
                          <span className="w-full truncate font-medium">{option.displayName}</span>
                          <span className="w-full truncate font-mono text-xs text-muted-foreground">
                            {option.id}
                          </span>
                        </div>
                      )}
                    />
                  ) : (
                    <Input
                      aria-label="模型 ID"
                      placeholder="glm-5.2"
                      value={model.id}
                      onChange={(event) => updateModel(index, { id: event.target.value })}
                    />
                  )}
                  <Input
                    aria-label="显示名称"
                    placeholder="glm-5.2"
                    value={model.displayName}
                    onChange={(event) => updateModel(index, { displayName: event.target.value })}
                  />
                  {modelProduct === 'claude' && (
                    <Select
                      value={model.claudeSlot ?? null}
                      onValueChange={(value) => {
                        if (!value) return;
                        const claudeSlot = value as ClaudeModelSlot;
                        updateModel(index, {
                          claudeSlot,
                          ...(claudeSlot === 'custom' ? { context1m: false } : {}),
                        });
                      }}
                    >
                      <SelectTrigger className="w-full" aria-label="映射入口">
                        <SelectValue placeholder="选择入口">
                          {
                            CLAUDE_MODEL_SLOTS.find((slot) => slot.value === model.claudeSlot)
                              ?.label
                          }
                        </SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        <SelectGroup>
                          {CLAUDE_MODEL_SLOTS.map((slot) => (
                            <SelectItem
                              key={slot.value}
                              value={slot.value}
                              disabled={models.some(
                                (item, itemIndex) =>
                                  itemIndex !== index && item.claudeSlot === slot.value,
                              )}
                            >
                              {slot.label}
                            </SelectItem>
                          ))}
                        </SelectGroup>
                      </SelectContent>
                    </Select>
                  )}
                  {modelProduct !== 'grok' && (
                    <div className="flex justify-center">
                      <Checkbox
                        aria-label={`${model.displayName || model.id || '模型'} 1M 上下文`}
                        title={
                          modelProduct === 'claude' && model.claudeSlot === 'custom'
                            ? 'Custom 不支持 1M 上下文'
                            : undefined
                        }
                        className={
                          modelProduct === 'claude' && model.claudeSlot === 'custom'
                            ? 'data-disabled:border-muted-foreground/20 data-disabled:bg-muted data-disabled:opacity-30'
                            : undefined
                        }
                        checked={
                          modelProduct === 'claude' && model.claudeSlot === 'custom'
                            ? false
                            : Boolean(model.context1m)
                        }
                        onCheckedChange={(checked) =>
                          updateModel(index, { context1m: checked === true })
                        }
                        disabled={
                          modelProduct === 'claude' &&
                          (!model.claudeSlot || model.claudeSlot === 'custom')
                        }
                      />
                    </div>
                  )}
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={`删除 ${model.displayName || model.id || '模型'}`}
                    onClick={() => removeModel(index)}
                    disabled={models.length === 1}
                  >
                    <Trash2 />
                  </Button>
                </div>
              ))
            ) : (
              <p className="px-3 py-7 text-center text-sm text-muted-foreground">
                模型方案至少需要一个模型。
              </p>
            )}
            <Button
              type="button"
              variant="ghost"
              className="h-10 w-full rounded-none"
              aria-label="添加模型"
              onClick={() =>
                setModels((current) => [...current, emptyModel(modelProduct, current)])
              }
              disabled={
                busy || (modelProduct === 'claude' && models.length >= CLAUDE_MODEL_SLOTS.length)
              }
            >
              <Plus />
            </Button>
          </div>
        </section>
        <Separator />
        <section className="py-5">
          <h2 className="mb-3 text-sm font-semibold">关联账号</h2>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(13rem,1fr))] gap-3">
            {assignments.map((assignment) => (
              <Card key={assignment.accountId} size="sm" className="min-h-24">
                <CardHeader>
                  <CardTitle className="truncate">{assignment.accountAlias}</CardTitle>
                  <CardAction className="flex gap-0.5">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`编辑 ${assignment.accountAlias}`}
                      onClick={() => {
                        setEditingAccountId(assignment.accountId);
                        setPendingAccountId(assignment.accountId);
                        setPendingModelId(assignment.defaultModelId);
                        setAccountDialogOpen(true);
                      }}
                    >
                      <Pencil />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`取消关联 ${assignment.accountAlias}`}
                      onClick={() =>
                        setAssignments((current) =>
                          current.filter((item) => item.accountId !== assignment.accountId),
                        )
                      }
                    >
                      <Trash2 />
                    </Button>
                  </CardAction>
                </CardHeader>
                <CardContent className="mt-auto flex flex-col gap-1">
                  <span className="text-xs text-muted-foreground">默认模型</span>
                  <span className="truncate text-sm">
                    {modelOptions.find((model) => model.id === assignment.defaultModelId)
                      ?.displayName ||
                      assignment.defaultModelId ||
                      '未设置'}
                  </span>
                </CardContent>
              </Card>
            ))}
            <Card size="sm" className="min-h-24">
              <CardContent className="flex flex-1">
                <Button
                  type="button"
                  variant="ghost"
                  className="min-h-full flex-1 flex-col text-muted-foreground"
                  onClick={() => {
                    setEditingAccountId(null);
                    setAccountDialogOpen(true);
                  }}
                  disabled={busy || !availableAccounts.length}
                >
                  <Plus data-icon="inline-start" />
                  添加账号
                </Button>
              </CardContent>
            </Card>
            <Dialog
              open={accountDialogOpen}
              onOpenChange={(open) => {
                setAccountDialogOpen(open);
                if (!open) {
                  setEditingAccountId(null);
                  setPendingAccountId(null);
                  setPendingModelId(null);
                }
              }}
            >
              <DialogContent initialFocus={false}>
                <DialogHeader>
                  <DialogTitle>{editingAccountId ? '编辑账号' : '添加账号'}</DialogTitle>
                </DialogHeader>
                <Field>
                  <FieldLabel>关联账号</FieldLabel>
                  <Select
                    value={pendingAccountId}
                    onValueChange={(value) => {
                      setPendingAccountId(value);
                      setPendingModelId(null);
                    }}
                  >
                    <SelectTrigger className="w-full" aria-label="选择关联账号">
                      <SelectValue placeholder="选择账号">
                        {
                          status?.relayAccounts.find(
                            (account) => account.accountId === pendingAccountId,
                          )?.accountAlias
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {status?.relayAccounts.map((account) => {
                          const linkedProfile = linkedProfileName(account.accountId);
                          return (
                            <SelectItem
                              key={account.accountId}
                              value={account.accountId}
                              disabled={Boolean(linkedProfile)}
                            >
                              {account.accountAlias}
                              {linkedProfile && `（已关联至${linkedProfile}）`}
                            </SelectItem>
                          );
                        })}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel>默认模型</FieldLabel>
                  <Select
                    value={pendingModelId}
                    onValueChange={setPendingModelId}
                    disabled={!pendingAccountId || !modelOptions.length}
                  >
                    <SelectTrigger className="w-full" aria-label="选择默认模型">
                      <SelectValue placeholder="选择默认模型" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {modelOptions.map((model) => (
                          <SelectItem key={model.id.trim()} value={model.id.trim()}>
                            {model.displayName || model.id}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </Field>
                <DialogFooter>
                  <Button
                    type="button"
                    size="sm"
                    onClick={saveAssignment}
                    disabled={!pendingAccountId || !pendingModelId}
                  >
                    {editingAccountId ? (
                      <Save data-icon="inline-start" />
                    ) : (
                      <Plus data-icon="inline-start" />
                    )}
                    {editingAccountId ? '保存修改' : '添加账号'}
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          </div>
        </section>
        <div className="flex justify-end">
          <Button type="submit" disabled={busy || !dirty || Boolean(validationError)}>
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
        open={blocker.status === 'blocked'}
        title="离开编辑页面"
        description="当前模型方案的修改尚未保存。"
        confirmLabel="放弃并离开"
        onClose={() => blocker.reset?.()}
        onConfirm={() => blocker.proceed?.()}
      />
    </PageShell>
  );
}

function emptyModel(product: 'codex' | 'claude' | 'grok', models: ModelEntry[]): ModelEntry {
  return {
    id: '',
    displayName: '',
    ...(product === 'claude'
      ? {
          claudeSlot: !models.some((model) => model.claudeSlot === 'custom')
            ? 'custom'
            : CLAUDE_MODEL_SLOTS.find(
                (slot) => !models.some((model) => model.claudeSlot === slot.value),
              )?.value,
        }
      : {}),
  };
}
