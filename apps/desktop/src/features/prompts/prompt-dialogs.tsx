import { LoaderCircle } from 'lucide-react';
import type { FormEvent } from 'react';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Field, FieldGroup, FieldLabel } from '../../components/ui/field';
import { Input } from '../../components/ui/input';

export function NameDialogForm({
  kind,
  name,
  busy,
  onNameChange,
  onClose,
  onSubmit,
}: {
  kind: 'create' | 'import' | null;
  name: string;
  busy: boolean;
  onNameChange: (name: string) => void;
  onClose: () => void;
  onSubmit: (event: FormEvent) => void;
}) {
  return (
    <Dialog open={Boolean(kind)} onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent initialFocus={false}>
        <form onSubmit={onSubmit} className="contents">
          <DialogHeader>
            <DialogTitle>{kind === 'import' ? '同步当前文件' : '新建方案'}</DialogTitle>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="agents-profile-name">方案名称</FieldLabel>
              <Input
                id="agents-profile-name"
                value={name}
                onChange={(event) => onNameChange(event.target.value)}
                disabled={busy}
              />
            </Field>
          </FieldGroup>
          <DialogFooter>
            <CancelButton disabled={busy} />
            <Button type="submit" disabled={busy || !name.trim()}>
              {busy && <LoaderCircle data-icon="inline-start" className="animate-spin" />}
              {kind === 'import' ? '同步' : '创建'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  destructive = false,
  busy = false,
  onClose,
  onConfirm,
}: {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  destructive?: boolean;
  busy?: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && !busy && onClose()}>
      <DialogContent initialFocus={false}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <CancelButton disabled={busy} />
          <Button
            variant={destructive ? 'destructive' : 'default'}
            onClick={onConfirm}
            disabled={busy}
          >
            {busy && <LoaderCircle data-icon="inline-start" className="animate-spin" />}
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CancelButton({ disabled }: { disabled: boolean }) {
  return (
    <DialogClose
      render={
        <Button
          variant="outline"
          disabled={disabled}
          onMouseDown={(event) => event.preventDefault()}
        />
      }
    >
      取消
    </DialogClose>
  );
}
