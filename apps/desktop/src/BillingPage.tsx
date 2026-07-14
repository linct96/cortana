import { invoke } from '@tauri-apps/api/core';
import { Download, LoaderCircle, Pencil, Plus, Trash2, TriangleAlert } from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from './components/page-shell';
import { Alert, AlertAction, AlertDescription, AlertTitle } from './components/ui/alert';
import { Button } from './components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './components/ui/dialog';
import { Field, FieldGroup, FieldLabel } from './components/ui/field';
import { Input } from './components/ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from './components/ui/tooltip';
import { ModelsDevDialog } from './features/billing/models-dev-dialog';
import { emptyPricing, type ModelPricing } from './features/billing/types';
import { appError } from './utils';

const priceFields = [
  ['inputCostPerMillion', '输入'],
  ['outputCostPerMillion', '输出'],
  ['cacheReadCostPerMillion', '缓存读取'],
  ['cacheWriteCostPerMillion', '缓存写入'],
] as const;

const priceNumber = new Intl.NumberFormat('en-US', {
  minimumFractionDigits: 0,
  maximumFractionDigits: 6,
});

export default function BillingPage() {
  const [pricing, setPricing] = useState<ModelPricing[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ model: ModelPricing; isNew: boolean } | null>(null);
  const [deleting, setDeleting] = useState<ModelPricing | null>(null);
  const [importing, setImporting] = useState(false);

  const loadPricing = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setPricing(await invoke<ModelPricing[]>('list_model_pricing'));
    } catch (caught) {
      setError(appError(caught));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadPricing();
  }, [loadPricing]);

  async function deletePricing() {
    if (!deleting) return;
    try {
      await invoke('delete_model_pricing', { modelId: deleting.modelId });
      setDeleting(null);
      await loadPricing();
      toast.success('模型定价已删除。');
    } catch (caught) {
      toast.error(appError(caught));
    }
  }

  return (
    <PageShell>
      <PageHeader
        title="计费"
        description="模型基础价格 · USD / 百万 Token"
        className="pb-7"
        actions={
          <>
            <Button variant="outline" type="button" onClick={() => setImporting(true)}>
              <Download data-icon="inline-start" /> 从 models.dev 导入
            </Button>
            <Button
              type="button"
              onClick={() => setEditing({ model: { ...emptyPricing }, isNew: true })}
            >
              <Plus data-icon="inline-start" /> 新增定价
            </Button>
          </>
        }
      />

      {loading ? (
        <div className="grid min-h-72 place-items-center text-muted-foreground">
          <LoaderCircle className="animate-spin" />
        </div>
      ) : error ? (
        <div className="px-4 sm:px-8 lg:px-12">
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertTitle>无法读取模型价格</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
            <AlertAction>
              <Button variant="outline" size="sm" onClick={() => void loadPricing()}>
                重试
              </Button>
            </AlertAction>
          </Alert>
        </div>
      ) : pricing.length ? (
        <PricingList
          pricing={pricing}
          onEdit={(model) => setEditing({ model, isNew: false })}
          onDelete={setDeleting}
        />
      ) : (
        <div className="grid min-h-60 place-items-center border-y text-center">
          <div className="flex flex-col items-center gap-3">
            <p className="text-sm text-muted-foreground">还没有模型定价</p>
            <Button variant="outline" type="button" onClick={() => setImporting(true)}>
              <Download data-icon="inline-start" /> 从 models.dev 导入
            </Button>
          </div>
        </div>
      )}

      {editing && (
        <PricingEditor
          key={`${editing.isNew ? 'new' : 'edit'}:${editing.model.modelId}`}
          model={editing.model}
          isNew={editing.isNew}
          onClose={() => setEditing(null)}
          onSaved={loadPricing}
        />
      )}
      {importing && (
        <ModelsDevDialog onClose={() => setImporting(false)} onImported={loadPricing} />
      )}
      {deleting && (
        <Dialog open onOpenChange={(open) => !open && setDeleting(null)}>
          <DialogContent initialFocus={false}>
            <DialogHeader>
              <DialogTitle>删除模型定价</DialogTitle>
              <DialogDescription>
                删除 {deleting.displayName} 后，相关用量会显示为未定价。
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="ghost" type="button" onClick={() => setDeleting(null)}>
                取消
              </Button>
              <Button variant="destructive" type="button" onClick={() => void deletePricing()}>
                删除
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </PageShell>
  );
}

function PricingList({
  pricing,
  onEdit,
  onDelete,
}: {
  pricing: ModelPricing[];
  onEdit: (model: ModelPricing) => void;
  onDelete: (model: ModelPricing) => void;
}) {
  const columns = 'grid-cols-[minmax(9rem,1fr)_repeat(4,4rem)_4.25rem]';
  return (
    <div className="border-y border-border px-4 sm:px-8 lg:px-12">
      <div
        className={`grid ${columns} gap-2 border-b py-2 text-xs font-medium text-muted-foreground`}
      >
        <span>模型</span>
        <span className="text-right">输入</span>
        <span className="text-right">输出</span>
        <span className="text-right">缓存读</span>
        <span className="text-right">缓存写</span>
        <span className="text-right">操作</span>
      </div>
      {pricing.map((model) => (
        <article
          key={model.modelId}
          className={`grid min-h-16 ${columns} items-center gap-2 border-b py-2 last:border-b-0`}
        >
          <div className="min-w-0">
            <strong className="block truncate text-sm font-medium" title={model.displayName}>
              {model.displayName}
            </strong>
            <span
              className="block truncate font-mono text-xs text-muted-foreground"
              title={model.modelId}
            >
              {model.modelId}
            </span>
          </div>
          <Price value={model.inputCostPerMillion} />
          <Price value={model.outputCostPerMillion} />
          <Price value={model.cacheReadCostPerMillion} />
          <Price value={model.cacheWriteCostPerMillion} />
          <div className="flex justify-end gap-1">
            <IconAction label="编辑" icon={Pencil} onClick={() => onEdit(model)} />
            <IconAction label="删除" icon={Trash2} onClick={() => onDelete(model)} />
          </div>
        </article>
      ))}
    </div>
  );
}

function PricingEditor({
  model,
  isNew,
  onClose,
  onSaved,
}: {
  model: ModelPricing;
  isNew: boolean;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [form, setForm] = useState(model);
  const [saving, setSaving] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    try {
      await invoke('save_model_pricing', { items: [form] });
      await onSaved();
      toast.success(isNew ? '模型定价已添加。' : '模型定价已更新。');
      onClose();
    } catch (caught) {
      toast.error(appError(caught));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && !saving && onClose()}>
      <DialogContent
        className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-lg"
        initialFocus={false}
      >
        <DialogHeader>
          <DialogTitle>{isNew ? '新增定价' : `编辑定价 · ${model.displayName}`}</DialogTitle>
          <DialogDescription>价格单位为 USD / 百万 Token</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="pricing-model-id">模型 ID</FieldLabel>
              <Input
                id="pricing-model-id"
                value={form.modelId}
                onChange={(event) => setForm({ ...form, modelId: event.target.value })}
                placeholder="gpt-5.4"
                disabled={!isNew || saving}
                required
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="pricing-display-name">显示名称</FieldLabel>
              <Input
                id="pricing-display-name"
                value={form.displayName}
                onChange={(event) => setForm({ ...form, displayName: event.target.value })}
                placeholder="GPT-5.4"
                disabled={saving}
                required
              />
            </Field>
            <div className="grid grid-cols-2 gap-4">
              {priceFields.map(([key, label]) => (
                <Field key={key}>
                  <FieldLabel htmlFor={`pricing-${key}`}>{label}</FieldLabel>
                  <Input
                    id={`pricing-${key}`}
                    type="number"
                    inputMode="decimal"
                    min="0"
                    step="any"
                    value={form[key]}
                    onChange={(event) => setForm({ ...form, [key]: event.target.value })}
                    disabled={saving}
                    required
                  />
                </Field>
              ))}
            </div>
          </FieldGroup>
          <DialogFooter>
            <Button variant="ghost" type="button" onClick={onClose} disabled={saving}>
              取消
            </Button>
            <Button type="submit" disabled={saving}>
              {saving && <LoaderCircle className="animate-spin" />}
              保存
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Price({ value }: { value: string }) {
  return (
    <span className="truncate text-right font-mono text-xs tabular-nums" title={`$${value}`}>
      ${priceNumber.format(Number(value))}
    </span>
  );
}

function IconAction({
  label,
  icon: Icon,
  onClick,
}: {
  label: string;
  icon: typeof Pencil;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <Button variant="ghost" size="icon-sm" type="button" onClick={onClick}>
          <Icon />
          <span className="sr-only">{label}</span>
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
