import { invoke } from '@tauri-apps/api/core';
import { LoaderCircle, Search, TriangleAlert } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { Alert, AlertAction, AlertDescription, AlertTitle } from '../../components/ui/alert';
import { Button } from '../../components/ui/button';
import { Checkbox } from '../../components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { InputGroup, InputGroupAddon, InputGroupInput } from '../../components/ui/input-group';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { appError } from '../../utils';
import type { ModelsDevPricing } from './types';

const priceNumber = new Intl.NumberFormat('en-US', {
  minimumFractionDigits: 0,
  maximumFractionDigits: 6,
});

export function ModelsDevDialog({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported: () => Promise<void>;
}) {
  const [models, setModels] = useState<ModelsDevPricing[]>([]);
  const [provider, setProvider] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<ModelsDevPricing[]>('fetch_models_dev_pricing')
      .then((result) => {
        if (!cancelled) setModels(result);
      })
      .catch((caught) => {
        if (!cancelled) setError(appError(caught));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadKey]);

  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    return models.filter(
      (model) =>
        (!provider || model.provider === provider) &&
        (!query ||
          model.modelId.toLowerCase().includes(query) ||
          model.displayName.toLowerCase().includes(query)),
    );
  }, [models, provider, search]);
  const providers = [
    { label: '全部供应商', value: null },
    ...[...new Set(models.map((model) => model.provider))].map((name) => ({
      label: name,
      value: name,
    })),
  ];
  const allFilteredSelected =
    filtered.length > 0 && filtered.every((model) => selected.has(model.modelId));

  function toggle(modelId: string, checked: boolean) {
    setSelected((current) => {
      const next = new Set(current);
      if (checked) next.add(modelId);
      else next.delete(modelId);
      return next;
    });
  }

  function toggleFiltered(checked: boolean) {
    setSelected((current) => {
      const next = new Set(current);
      for (const model of filtered) {
        if (checked) next.add(model.modelId);
        else next.delete(model.modelId);
      }
      return next;
    });
  }

  async function importSelected() {
    const items = models.filter((model) => selected.has(model.modelId));
    if (!items.length) return;
    setSaving(true);
    try {
      await invoke('save_model_pricing', { items });
      await onImported();
      toast.success(`已导入 ${items.length} 个模型定价。`);
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
        className="h-[min(38rem,calc(100vh-2rem))] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-3xl"
        initialFocus={false}
      >
        <DialogHeader>
          <DialogTitle>导入模型价格</DialogTitle>
        </DialogHeader>

        <div className="flex min-h-0 flex-col gap-3">
          <div className="flex gap-2">
            <Select items={providers} value={provider} onValueChange={setProvider}>
              <SelectTrigger className="w-36" aria-label="按供应商筛选模型">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {providers.map((item) => (
                    <SelectItem key={item.value ?? 'all'} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
            <InputGroup>
              <InputGroupAddon>
                <Search />
              </InputGroupAddon>
              <InputGroupInput
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="搜索模型"
                aria-label="搜索 models.dev 模型"
              />
            </InputGroup>
          </div>

          {loading ? (
            <div className="grid min-h-0 flex-1 place-items-center text-muted-foreground">
              <LoaderCircle className="animate-spin" />
            </div>
          ) : error ? (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>无法加载模型价格</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
              <AlertAction>
                <Button variant="outline" size="sm" onClick={() => setReloadKey((key) => key + 1)}>
                  重试
                </Button>
              </AlertAction>
            </Alert>
          ) : (
            <div className="min-h-0 flex-1 overflow-y-auto border-y border-border">
              <label className="sticky top-0 z-10 flex h-9 items-center gap-3 border-b bg-background px-3 text-xs font-medium text-muted-foreground">
                <Checkbox
                  checked={allFilteredSelected}
                  onCheckedChange={(checked) => toggleFiltered(checked)}
                  aria-label="选择全部搜索结果"
                />
                <span className="flex-1">模型</span>
                <span className="w-16 text-right">输入</span>
                <span className="w-16 text-right">输出</span>
                <span className="w-16 text-right">缓存读</span>
                <span className="w-16 text-right">缓存写</span>
              </label>
              {filtered.length ? (
                filtered.map((model) => (
                  <label
                    key={model.modelId}
                    className="flex min-h-14 cursor-pointer items-center gap-3 border-b px-3 py-2 last:border-b-0 hover:bg-muted/40"
                  >
                    <Checkbox
                      checked={selected.has(model.modelId)}
                      onCheckedChange={(checked) => toggle(model.modelId, checked)}
                      aria-label={`选择 ${model.displayName}`}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline gap-2">
                        <strong className="truncate text-sm font-medium">
                          {model.displayName}
                        </strong>
                        {model.releaseDate && (
                          <span className="shrink-0 text-xs text-muted-foreground">
                            {model.releaseDate}
                          </span>
                        )}
                      </span>
                      <span className="block truncate font-mono text-xs text-muted-foreground">
                        {model.modelId}
                      </span>
                    </span>
                    <Price value={model.inputCostPerMillion} />
                    <Price value={model.outputCostPerMillion} />
                    <Price value={model.cacheReadCostPerMillion} />
                    <Price value={model.cacheWriteCostPerMillion} />
                  </label>
                ))
              ) : (
                <div className="grid min-h-40 place-items-center text-sm text-muted-foreground">
                  没有匹配的模型
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="ghost" type="button" onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button
            type="button"
            onClick={() => void importSelected()}
            disabled={!selected.size || saving}
          >
            {saving && <LoaderCircle className="animate-spin" />}
            导入{selected.size ? ` ${selected.size} 个模型` : ''}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Price({ value }: { value: string }) {
  return (
    <span className="w-16 shrink-0 text-right font-mono text-xs tabular-nums">
      ${priceNumber.format(Number(value))}
    </span>
  );
}
