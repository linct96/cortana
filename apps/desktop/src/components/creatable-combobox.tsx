import { type ReactNode, useState } from 'react';
import {
  Combobox,
  ComboboxContent,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from './ui/combobox';

type CreatableComboboxProps<Item> = {
  'aria-label': string;
  items: readonly Item[];
  value: string;
  onValueChange: (value: string) => void;
  onItemSelect?: (item: Item) => void;
  itemToValue: (item: Item) => string;
  itemToLabel: (item: Item) => string;
  renderItem?: (item: Item) => ReactNode;
  placeholder?: string;
  disabled?: boolean;
};

export function CreatableCombobox<Item>({
  'aria-label': ariaLabel,
  items,
  value,
  onValueChange,
  onItemSelect,
  itemToValue,
  itemToLabel,
  renderItem = itemToLabel,
  placeholder,
  disabled,
}: CreatableComboboxProps<Item>) {
  const [openValue, setOpenValue] = useState<string | null>(null);
  const itemsByValue = new Map(items.map((item) => [itemToValue(item), item]));
  const itemValues = [...itemsByValue.keys()];

  return (
    <Combobox
      items={itemValues}
      value={value || null}
      inputValue={value}
      onInputValueChange={onValueChange}
      onOpenChange={(open) => setOpenValue(open ? value : null)}
      onValueChange={(itemValue: string | null) => {
        if (!itemValue) return;
        const item = itemsByValue.get(itemValue);
        onValueChange(itemValue);
        if (!item) return;
        onItemSelect?.(item);
      }}
      filter={(itemValue: string, query) => {
        const item = itemsByValue.get(itemValue);
        if (!item) return false;
        const normalized = query.trim().toLocaleLowerCase();
        return (
          query === openValue ||
          itemValue.toLocaleLowerCase().includes(normalized) ||
          itemToLabel(item).toLocaleLowerCase().includes(normalized)
        );
      }}
      openOnInputClick
      disabled={disabled}
    >
      <ComboboxInput
        aria-label={ariaLabel}
        placeholder={placeholder}
        className="w-full"
        disabled={disabled}
      />
      <ComboboxContent>
        <ComboboxList>
          {(itemValue: string) => {
            const item = itemsByValue.get(itemValue);
            if (!item) return null;
            return (
              <ComboboxItem key={itemValue} value={itemValue}>
                {renderItem(item)}
              </ComboboxItem>
            );
          }}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}
