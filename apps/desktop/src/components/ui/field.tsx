import { Field as FieldPrimitive } from '@base-ui/react/field';
import type { ComponentProps } from 'react';
import { cn } from '../../utils';

function FieldGroup({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div data-slot="field-group" className={cn('flex flex-col gap-4', className)} {...props} />
  );
}

function Field({ className, ...props }: FieldPrimitive.Root.Props) {
  return (
    <FieldPrimitive.Root
      data-slot="field"
      className={cn('group/field flex flex-col gap-2', className)}
      {...props}
    />
  );
}

function FieldLabel({ className, ...props }: FieldPrimitive.Label.Props) {
  return (
    <FieldPrimitive.Label
      data-slot="field-label"
      className={cn(
        'text-sm font-medium group-data-[invalid=true]/field:text-destructive group-data-disabled/field:opacity-50',
        className,
      )}
      {...props}
    />
  );
}

function FieldError({ className, ...props }: FieldPrimitive.Error.Props) {
  return (
    <FieldPrimitive.Error
      data-slot="field-error"
      className={cn('text-xs text-destructive', className)}
      {...props}
    />
  );
}

export { Field, FieldError, FieldGroup, FieldLabel };
