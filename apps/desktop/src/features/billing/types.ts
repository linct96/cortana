export type ModelPricing = {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheWriteCostPerMillion: string;
};

export type ModelsDevPricing = ModelPricing & {
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
