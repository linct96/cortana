import { describe, expect, it } from 'vitest';
import { fillRelayModels, modelFormError, removeModelAt, uniqueModelsById } from './types';

const remoteModels = [
  { id: 'first', displayName: 'First' },
  { id: 'second', displayName: 'Second' },
  { id: 'third', displayName: 'Third' },
  { id: 'fourth', displayName: 'Fourth' },
  { id: 'fifth', displayName: 'Fifth' },
];

describe('uniqueModelsById', () => {
  it('重复 ID 只保留第一个模型', () => {
    expect(
      uniqueModelsById([
        { id: 'shared', displayName: 'First' },
        { id: 'shared', displayName: 'Second' },
      ]),
    ).toEqual([{ id: 'shared', displayName: 'First' }]);
  });
});

describe('removeModelAt', () => {
  it('仅在最后一个同 ID 模型被删除时清空账号默认模型', () => {
    const assignments = [{ accountId: 'account', accountAlias: '账号', defaultModelId: 'shared' }];
    const first = removeModelAt(
      [
        { id: 'shared', displayName: 'First' },
        { id: 'shared', displayName: 'Second' },
      ],
      assignments,
      0,
    );
    expect(first.assignments).toEqual(assignments);

    expect(
      removeModelAt(first.models, first.assignments, 0).assignments[0].defaultModelId,
    ).toBeNull();
  });
});

describe('modelFormError', () => {
  it('拒绝空方案', () => {
    expect(modelFormError('codex', '官方模型', [], [])).toContain('至少需要一个模型');
  });

  it('允许重复模型 ID', () => {
    expect(
      modelFormError(
        'codex',
        '中转模型',
        [
          { id: 'deepseek-chat', displayName: 'Chat' },
          { id: 'deepseek-chat', displayName: 'Chat 2' },
        ],
        [],
      ),
    ).toBeNull();
  });

  it('要求关联账号选择方案内模型', () => {
    expect(
      modelFormError(
        'codex',
        '中转模型',
        [{ id: 'deepseek-chat', displayName: 'Chat' }],
        [{ accountId: 'account', accountAlias: '账号', defaultModelId: 'missing' }],
      ),
    ).toContain('默认模型');
  });

  it('限制 Claude 映射入口数量和唯一性', () => {
    expect(
      modelFormError(
        'claude',
        'Claude 模型',
        [
          { id: 'first', displayName: 'First', claudeSlot: 'opus' },
          { id: 'second', displayName: 'Second', claudeSlot: 'opus' },
        ],
        [],
      ),
    ).toContain('入口重复');
  });

  it('禁止 Custom 配置 1M 上下文', () => {
    expect(
      modelFormError(
        'claude',
        'Claude 模型',
        [{ id: 'custom', displayName: 'Custom', claudeSlot: 'custom', context1m: true }],
        [],
      ),
    ).toContain('不支持 1M');
  });

  it('保留已有模型并补充远端模型', () => {
    expect(
      fillRelayModels('codex', [{ id: 'first', displayName: 'Existing' }], remoteModels),
    ).toEqual([
      { id: 'first', displayName: 'Existing' },
      { id: 'second', displayName: 'Second' },
      { id: 'third', displayName: 'Third' },
      { id: 'fourth', displayName: 'Fourth' },
      { id: 'fifth', displayName: 'Fifth' },
    ]);
  });

  it('Grok 按普通模型方案补充远端模型', () => {
    expect(fillRelayModels('grok', [], remoteModels)).toEqual(remoteModels);
  });

  it('Claude 依次填充到五个映射入口', () => {
    expect(fillRelayModels('claude', [], remoteModels)).toEqual([
      { id: 'first', displayName: 'First', claudeSlot: 'custom' },
      { id: 'second', displayName: 'Second', claudeSlot: 'fable' },
      { id: 'third', displayName: 'Third', claudeSlot: 'opus' },
      { id: 'fourth', displayName: 'Fourth', claudeSlot: 'sonnet' },
      { id: 'fifth', displayName: 'Fifth', claudeSlot: 'haiku' },
    ]);
  });
});
