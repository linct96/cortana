import { describe, expect, it } from 'vitest';
import { emptyPricing, validatePricing } from './types';

describe('validatePricing', () => {
  it('returns field errors without relying on native form validation', () => {
    expect(
      validatePricing({
        ...emptyPricing,
        modelId: ' ',
        displayName: '',
        inputCostPerMillion: '-1',
      }),
    ).toEqual({
      modelId: '模型 ID 不能为空。',
      displayName: '显示名称不能为空。',
      inputCostPerMillion: '输入价格必须是非负数。',
    });
  });
});
