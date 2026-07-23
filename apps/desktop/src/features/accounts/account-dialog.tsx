import { Eye, EyeOff, LoaderCircle } from 'lucide-react';
import type { FormEvent, ReactNode } from 'react';
import { productName } from '../../components/app-shell-context';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '../../components/ui/input-group';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../components/ui/tabs';
import { Textarea } from '../../components/ui/textarea';
import type { AccountProduct, AddMode, PendingConfirm, Profile } from './types';

type Setter<T> = (value: T | ((current: T) => T)) => void;

export function AddAccountDialog({
  product,
  busy,
  addMode,
  alias,
  authJson,
  relayApiKey,
  relayApiBaseUrl,
  showRelayApiKey,
  oauthMessage,
  setAddMode,
  setAlias,
  setAuthJson,
  setRelayApiKey,
  setRelayApiBaseUrl,
  setShowRelayApiKey,
  onSubmit,
  onClose,
}: {
  product: AccountProduct;
  busy: string | null;
  addMode: AddMode;
  alias: string;
  authJson: string;
  relayApiKey: string;
  relayApiBaseUrl: string;
  showRelayApiKey: boolean;
  oauthMessage: string | null;
  setAddMode: Setter<AddMode>;
  setAlias: Setter<string>;
  setAuthJson: Setter<string>;
  setRelayApiKey: Setter<string>;
  setRelayApiBaseUrl: Setter<string>;
  setShowRelayApiKey: Setter<boolean>;
  onSubmit: (event: FormEvent) => void;
  onClose: () => void;
}) {
  const saving = busy === 'oauth' || busy === 'auth-json' || busy === 'relay' || busy === 'import';
  if (product !== 'codex' && product !== 'claude') {
    return (
      <AppDialog title="添加账号" onClose={onClose}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <Field label="别名" htmlFor="new-alias">
            <Input
              id="new-alias"
              value={alias}
              onChange={(event) => setAlias(event.target.value)}
              placeholder="例如：工作账户"
            />
          </Field>
          {oauthMessage && (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              {busy === 'oauth' && <LoaderCircle size={15} className="animate-spin" />}{' '}
              {oauthMessage}
            </p>
          )}
          <DialogFooter>
            <CancelButton />
            <Button type="submit" disabled={saving}>
              {saving && <LoaderCircle size={16} className="animate-spin" />}
              在浏览器中授权
            </Button>
          </DialogFooter>
        </form>
      </AppDialog>
    );
  }
  return (
    <AppDialog title="添加账号" contentClassName="sm:max-w-xl" onClose={onClose}>
      <form onSubmit={onSubmit}>
        <Tabs
          className="gap-4"
          value={addMode}
          onValueChange={(value) => setAddMode(value as AddMode)}
        >
          <TabsList className="w-full">
            {(product === 'codex'
              ? [
                  ['browser', '浏览器授权'],
                  ['paste', '粘贴 auth.json'],
                  ['relay', '中转站'],
                ]
              : [
                  ['browser', '浏览器授权'],
                  ['relay', '中转站'],
                ]
            ).map(([value, label]) => (
              <TabsTrigger
                key={value}
                value={value}
                disabled={saving}
                onMouseDown={(event) => event.preventDefault()}
              >
                {label}
              </TabsTrigger>
            ))}
          </TabsList>
          <Field label="别名" htmlFor="new-alias">
            <Input
              id="new-alias"
              value={alias}
              onChange={(event) => setAlias(event.target.value)}
              placeholder="例如：工作账户"
            />
          </Field>
          {product === 'codex' && (
            <TabsContent value="paste">
              <Field label="auth.json" htmlFor="auth-json">
                <Textarea
                  id="auth-json"
                  className="min-h-0 resize-none field-sizing-fixed font-mono text-xs"
                  value={authJson}
                  onChange={(event) => setAuthJson(event.target.value)}
                  placeholder='{"tokens":{"refresh_token":"..."}}'
                  rows={2}
                  autoComplete="off"
                  spellCheck={false}
                  required
                />
              </Field>
            </TabsContent>
          )}
          <TabsContent value="relay" className="flex flex-col gap-4">
            <Field label="API Key" htmlFor="relay-api-key">
              <SecretInput
                id="relay-api-key"
                visible={showRelayApiKey}
                value={relayApiKey}
                onChange={setRelayApiKey}
                onToggle={() => setShowRelayApiKey((visible) => !visible)}
                required
              />
            </Field>
            <Field label="API 地址" htmlFor="relay-api-base-url">
              <Input
                id="relay-api-base-url"
                type="url"
                value={relayApiBaseUrl}
                onChange={(event) => setRelayApiBaseUrl(event.target.value)}
                placeholder="https://example.com/v1"
                autoComplete="url"
                required
              />
            </Field>
          </TabsContent>
          <TabsContent value="browser" className="empty:hidden">
            {oauthMessage && (
              <p className="flex items-center gap-2 text-sm text-muted-foreground">
                {busy === 'oauth' && <LoaderCircle size={15} className="animate-spin" />}{' '}
                {oauthMessage}
              </p>
            )}
          </TabsContent>
          <DialogFooter>
            <CancelButton disabled={busy === 'auth-json' || busy === 'relay'} />
            <Button type="submit" disabled={saving}>
              {saving && <LoaderCircle size={16} className="animate-spin" />}
              {addMode === 'browser' ? '在浏览器中授权' : '确认'}
            </Button>
          </DialogFooter>
        </Tabs>
      </form>
    </AppDialog>
  );
}

export function EditAccountDialog({
  editing,
  busy,
  alias,
  authJson,
  relayApiKey,
  relayApiBaseUrl,
  showRelayApiKey,
  setAlias,
  setAuthJson,
  setRelayApiKey,
  setRelayApiBaseUrl,
  setShowRelayApiKey,
  onSubmit,
  onClose,
}: {
  editing: Profile;
  busy: string | null;
  alias: string;
  authJson: string;
  relayApiKey: string;
  relayApiBaseUrl: string;
  showRelayApiKey: boolean;
  setAlias: Setter<string>;
  setAuthJson: Setter<string>;
  setRelayApiKey: Setter<string>;
  setRelayApiBaseUrl: Setter<string>;
  setShowRelayApiKey: Setter<boolean>;
  onSubmit: (event: FormEvent) => void;
  onClose: () => void;
}) {
  return (
    <AppDialog title="编辑账户" contentClassName="sm:max-w-2xl" onClose={onClose}>
      <form className="flex flex-col gap-4" onSubmit={onSubmit}>
        <Field label="别名" htmlFor="editing-alias">
          <Input
            id="editing-alias"
            value={alias}
            onChange={(event) => setAlias(event.target.value)}
            placeholder={
              editing.accountType === 'relay'
                ? '留空则使用 API 主机名'
                : editing.product !== 'codex'
                  ? '例如：工作账户'
                  : '留空则使用邮箱账号'
            }
            required={editing.product !== 'codex' && editing.accountType !== 'relay'}
          />
        </Field>
        {editing.accountType === 'relay' ? (
          <>
            <Field label="API Key" htmlFor="editing-relay-api-key">
              <SecretInput
                id="editing-relay-api-key"
                visible={showRelayApiKey}
                value={relayApiKey}
                onChange={setRelayApiKey}
                onToggle={() => setShowRelayApiKey((visible) => !visible)}
                placeholder="留空则保留现有 API Key"
              />
            </Field>
            <Field label="API 地址" htmlFor="editing-relay-api-base-url">
              <Input
                id="editing-relay-api-base-url"
                type="url"
                value={relayApiBaseUrl}
                onChange={(event) => setRelayApiBaseUrl(event.target.value)}
                autoComplete="url"
                required
              />
            </Field>
          </>
        ) : (
          <Field label="auth.json" htmlFor="editing-auth-json">
            <Textarea
              id="editing-auth-json"
              className="h-80 resize-y field-sizing-fixed font-mono text-xs"
              value={authJson}
              onChange={(event) => setAuthJson(event.target.value)}
              autoComplete="off"
              spellCheck={false}
              required
            />
          </Field>
        )}
        <DialogFooter>
          <CancelButton />
          <Button type="submit" disabled={busy === `edit:${editing.id}`}>
            {busy === `edit:${editing.id}` && <LoaderCircle className="animate-spin" />}
            保存
          </Button>
        </DialogFooter>
      </form>
    </AppDialog>
  );
}

export function ConfirmAccountDialog({
  confirm,
  onConfirm,
  onClose,
}: {
  confirm: NonNullable<PendingConfirm>;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const product = productName(confirm.profile.product);
  return (
    <AppDialog
      title={confirm.kind === 'delete' ? '移除账户档案' : '覆盖当前登录状态'}
      onClose={onClose}
    >
      <div>
        <p className="text-sm leading-6 text-muted-foreground">
          {confirm.kind === 'delete'
            ? confirm.profile.product === 'claude' && confirm.profile.isActive
              ? confirm.profile.accountType === 'relay'
                ? `将移除“${confirm.profile.alias}”的本地认证档案，并从 settings.json 清除当前 Claude 中转站凭据。`
                : `将移除“${confirm.profile.alias}”的本地认证档案，并从 settings.json 清除当前 Claude OAuth Token。Keychain 登录保持不变。`
              : confirm.profile.isActive
                ? `将移除“${confirm.profile.alias}”的本地认证档案。当前 ${product} 登录保持不变，但不再由本应用管理。`
                : `将移除“${confirm.profile.alias}”的本地认证档案。`
            : `当前 ${product} 登录或 API 配置在应用外被修改。继续会立即切换到“${confirm.profile.alias}”。`}
        </p>
        <DialogFooter>
          <CancelButton />
          <Button
            variant={confirm.kind === 'delete' ? 'destructive' : 'default'}
            type="button"
            onClick={onConfirm}
          >
            {confirm.kind === 'delete' ? '移除' : '仍要切换'}
          </Button>
        </DialogFooter>
      </div>
    </AppDialog>
  );
}

function SecretInput({
  id,
  visible,
  value,
  onChange,
  onToggle,
  placeholder,
  required,
}: {
  id: string;
  visible: boolean;
  value: string;
  onChange: Setter<string>;
  onToggle: () => void;
  placeholder?: string;
  required?: boolean;
}) {
  return (
    <InputGroup>
      <InputGroupInput
        id={id}
        type={visible ? 'text' : 'password'}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        autoComplete="off"
        required={required}
      />
      <InputGroupAddon align="inline-end">
        <InputGroupButton
          size="icon-xs"
          aria-label={visible ? '隐藏 API Key' : '显示 API Key'}
          onClick={onToggle}
        >
          {visible ? <EyeOff /> : <Eye />}
        </InputGroupButton>
      </InputGroupAddon>
    </InputGroup>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
    </div>
  );
}

function CancelButton({ disabled = false }: { disabled?: boolean }) {
  return (
    <DialogClose
      disabled={disabled}
      render={
        <Button variant="ghost" type="button" onMouseDown={(event) => event.preventDefault()} />
      }
    >
      取消
    </DialogClose>
  );
}

function AppDialog({
  title,
  children,
  onClose,
  contentClassName,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  contentClassName?: string;
}) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className={contentClassName} initialFocus={false}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        {children}
      </DialogContent>
    </Dialog>
  );
}
