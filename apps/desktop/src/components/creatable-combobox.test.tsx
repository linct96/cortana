import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { CreatableCombobox } from './creatable-combobox';

describe('CreatableCombobox', () => {
  it('保留候选项之外的输入值', () => {
    const html = renderToStaticMarkup(
      <CreatableCombobox
        aria-label="模型 ID"
        items={[{ id: 'remote-model', name: '远端模型' }]}
        value="custom-model"
        onValueChange={vi.fn()}
        itemToValue={(item) => item.id}
        itemToLabel={(item) => item.name}
      />,
    );

    expect(html).toContain('value="custom-model"');
  });
});
