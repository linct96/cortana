import type { AccountProduct } from '../../components/app-shell-context';

export type ClaudeModelSlot = 'custom' | 'fable' | 'opus' | 'sonnet' | 'haiku';

export type ModelEntry = {
  id: string;
  displayName: string;
  claudeSlot?: ClaudeModelSlot;
  context1m?: boolean;
};

export type ModelAssignment = {
  accountId: string;
  accountAlias: string;
  defaultModelId: string | null;
};

export type ModelProfile = {
  id: string;
  name: string;
  models: ModelEntry[];
  assignments: ModelAssignment[];
};

export type ModelProfilesStatus = {
  profiles: ModelProfile[];
  relayAccounts: ModelAssignment[];
};

export type RelayModelOption = ModelEntry;

export const CLAUDE_MODEL_SLOTS: { value: ClaudeModelSlot; label: string }[] = [
  { value: 'custom', label: 'Custom' },
  { value: 'fable', label: 'Fable' },
  { value: 'opus', label: 'Opus' },
  { value: 'sonnet', label: 'Sonnet' },
  { value: 'haiku', label: 'Haiku' },
];

export function uniqueModelsById(models: ModelEntry[]) {
  const ids = new Set<string>();
  return models.filter((model) => {
    const id = model.id.trim();
    if (!id || ids.has(id)) return false;
    ids.add(id);
    return true;
  });
}

export function removeModelAt(models: ModelEntry[], assignments: ModelAssignment[], index: number) {
  const removedId = models[index]?.id;
  const nextModels = models.filter((_, modelIndex) => modelIndex !== index);
  return {
    models: nextModels,
    assignments: nextModels.some((model) => model.id === removedId)
      ? assignments
      : assignments.map((assignment) =>
          assignment.defaultModelId === removedId
            ? { ...assignment, defaultModelId: null }
            : assignment,
        ),
  };
}

export function fillRelayModels(
  product: 'codex' | 'claude' | 'grok',
  current: ModelEntry[],
  remote: RelayModelOption[],
) {
  const kept = current.filter((model) => model.id.trim() || model.displayName.trim());
  const ids = new Set(kept.map((model) => model.id.trim()).filter(Boolean));
  const additions = remote.filter((model) => {
    const id = model.id.trim();
    if (ids.has(id)) return false;
    ids.add(id);
    return true;
  });
  if (product !== 'claude') return [...kept, ...additions];

  const slots = new Set(kept.map((model) => model.claudeSlot).filter(Boolean));
  const slotOrder: ClaudeModelSlot[] = ['custom', 'fable', 'opus', 'sonnet', 'haiku'];
  return [
    ...kept,
    ...additions.slice(0, Math.max(0, CLAUDE_MODEL_SLOTS.length - kept.length)).map((model) => {
      const claudeSlot = slotOrder.find((slot) => !slots.has(slot));
      if (claudeSlot) slots.add(claudeSlot);
      return { ...model, claudeSlot };
    }),
  ];
}

export function modelFormError(
  product: AccountProduct,
  name: string,
  models: ModelEntry[],
  assignments: ModelAssignment[],
) {
  if (!name.trim()) return '请输入方案名称。';
  if (!models.length) return '模型方案至少需要一个模型。';
  if (product === 'claude' && models.length > CLAUDE_MODEL_SLOTS.length) {
    return 'Claude 模型方案最多支持 5 个模型。';
  }
  const ids = new Set<string>();
  const slots = new Set<ClaudeModelSlot>();
  for (const model of models) {
    if (!model.id.trim() || !model.displayName.trim()) return '模型 ID 和显示名称不能为空。';
    ids.add(model.id.trim());
    if (product === 'claude') {
      if (!model.claudeSlot) return 'Claude 模型必须选择映射入口。';
      if (model.claudeSlot === 'custom' && model.context1m) {
        return 'Custom 模型不支持 1M 上下文配置。';
      }
      if (slots.has(model.claudeSlot)) return 'Claude 模型映射入口重复。';
      slots.add(model.claudeSlot);
    }
  }
  if (
    assignments.some(
      (assignment) => !assignment.defaultModelId || !ids.has(assignment.defaultModelId),
    )
  ) {
    return '每个关联账号都必须选择方案内的默认模型。';
  }
  return null;
}
