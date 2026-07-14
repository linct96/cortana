export type ModelPricing = {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheWriteCostPerMillion: string;
};

export type ModelsDevPricing = ModelPricing & {
  provider: string;
  releaseDate: string;
};

export const emptyPricing: ModelPricing = {
  modelId: '',
  displayName: '',
  inputCostPerMillion: '0',
  outputCostPerMillion: '0',
  cacheReadCostPerMillion: '0',
  cacheWriteCostPerMillion: '0',
};

export type PricingErrors = Partial<Record<keyof ModelPricing, string>>;

export function validatePricing(pricing: ModelPricing): PricingErrors {
  const errors: PricingErrors = {};
  const modelId = pricing.modelId.trim();
  const displayName = pricing.displayName.trim();

  if (!modelId) errors.modelId = '模型 ID 不能为空。';
  else if (modelId.length > 200) errors.modelId = '模型 ID 不能超过 200 个字符。';

  if (!displayName) errors.displayName = '显示名称不能为空。';
  else if (displayName.length > 200) errors.displayName = '显示名称不能超过 200 个字符。';

  for (const [key, label] of [
    ['inputCostPerMillion', '输入'],
    ['outputCostPerMillion', '输出'],
    ['cacheReadCostPerMillion', '缓存读取'],
    ['cacheWriteCostPerMillion', '缓存写入'],
  ] as const) {
    const value = pricing[key].trim();
    if (value.length > 32 || !/^\d+(\.\d+)?$/.test(value) || !Number.isFinite(Number(value))) {
      errors[key] = `${label}价格必须是非负数。`;
    }
  }

  return errors;
}
