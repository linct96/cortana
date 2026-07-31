import { ExternalLink, Eye, EyeOff, LoaderCircle } from 'lucide-react';
import type { FormEvent, ReactNode } from 'react';
import { productName } from '../../components/app-shell-context';
import { CopyButton } from '../../components/copy-button';
import { RefreshButton } from '../../components/refresh-button';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Field, FieldGroup, FieldLabel } from '../../components/ui/field';
import { Input } from '../../components/ui/input';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Switch } from '../../components/ui/switch';
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '../../components/ui/input-group';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../components/ui/tabs';
import { Textarea } from '../../components/ui/textarea';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import type { AccountProduct, AddMode, PendingConfirm, Profile } from './types';
import { type ModelProfilesStatus, uniqueModelsById } from '../models/types';

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
  modelStatus,
  customModelEnabled,
  modelProfileId,
  defaultModelId,
  oauthMessage,
  oauthUrl,
  callbackUrl,
  setAddMode,
  setAlias,
  setAuthJson,
  setRelayApiKey,
  setRelayApiBaseUrl,
  setShowRelayApiKey,
  setCustomModelEnabled,
  setModelProfileId,
  setDefaultModelId,
  setCallbackUrl,
  onGenerateOAuth,
  onOpenOAuth,
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
  modelStatus: ModelProfilesStatus | null;
  customModelEnabled: boolean;
  modelProfileId: string | null;
  defaultModelId: string | null;
  oauthMessage: string | null;
  oauthUrl: string;
  callbackUrl: string;
  setAddMode: Setter<AddMode>;
  setAlias: (value: string) => void;
  setAuthJson: Setter<string>;
  setRelayApiKey: Setter<string>;
  setRelayApiBaseUrl: Setter<string>;
  setShowRelayApiKey: Setter<boolean>;
  setCustomModelEnabled: Setter<boolean>;
  setModelProfileId: Setter<string | null>;
  setDefaultModelId: Setter<string | null>;
  setCallbackUrl: Setter<string>;
  onGenerateOAuth: () => Promise<void>;
  onOpenOAuth: () => void;
  onSubmit: (event: FormEvent) => void;
  onClose: () => void;
}) {
  const oauthSaving = busy === 'oauth' || busy?.startsWith('oauth:');
  const saving = oauthSaving || busy === 'auth-json' || busy === 'relay' || busy === 'import';
  if (product === 'antigravity') {
    return (
      <AppDialog title="添加账号" contentClassName="sm:max-w-xl" onClose={onClose}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="new-alias">别名</FieldLabel>
              <Input
                id="new-alias"
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
                placeholder="例如：工作账户"
              />
            </Field>
            <BrowserOAuthFields
              busy={busy}
              oauthMessage={oauthMessage}
              oauthUrl={oauthUrl}
              callbackUrl={callbackUrl}
              setCallbackUrl={setCallbackUrl}
              onGenerate={onGenerateOAuth}
              onOpen={onOpenOAuth}
            />
          </FieldGroup>
          <DialogFooter>
            <CancelButton />
            <Button type="submit" disabled={saving || !callbackUrl.trim()}>
              {busy === 'oauth:complete' && (
                <LoaderCircle data-icon="inline-start" className="animate-spin" />
              )}
              确认
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
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="new-alias">别名</FieldLabel>
              <Input
                id="new-alias"
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
                placeholder="例如：工作账户"
              />
            </Field>
            {product === 'codex' && (
              <TabsContent value="paste">
                <Field>
                  <FieldLabel htmlFor="auth-json">auth.json</FieldLabel>
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
              <Field>
                <FieldLabel htmlFor="relay-api-key">API Key</FieldLabel>
                <SecretInput
                  id="relay-api-key"
                  visible={showRelayApiKey}
                  value={relayApiKey}
                  onChange={setRelayApiKey}
                  onToggle={() => setShowRelayApiKey((visible) => !visible)}
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="relay-api-base-url">API 地址</FieldLabel>
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
              {(product === 'codex' || product === 'claude' || product === 'grok') && (
                <ModelProfileFields
                  status={modelStatus}
                  enabled={customModelEnabled}
                  profileId={modelProfileId}
                  defaultModelId={defaultModelId}
                  onEnabledChange={setCustomModelEnabled}
                  onProfileChange={(value) => {
                    setModelProfileId(value);
                    setDefaultModelId(
                      modelStatus?.profiles.find((profile) => profile.id === value)?.models[0]
                        ?.id ?? null,
                    );
                  }}
                  onDefaultModelChange={setDefaultModelId}
                />
              )}
            </TabsContent>
            <TabsContent value="browser" className="empty:hidden">
              <BrowserOAuthFields
                busy={busy}
                oauthMessage={oauthMessage}
                oauthUrl={oauthUrl}
                callbackUrl={callbackUrl}
                setCallbackUrl={setCallbackUrl}
                onGenerate={onGenerateOAuth}
                onOpen={onOpenOAuth}
                showCallback={product !== 'grok'}
              />
            </TabsContent>
          </FieldGroup>
          <DialogFooter>
            <CancelButton disabled={busy === 'auth-json' || busy === 'relay'} />
            {!(product === 'grok' && addMode === 'browser') && (
              <Button
                type="submit"
                disabled={saving || (addMode === 'browser' && !callbackUrl.trim())}
              >
                {saving && <LoaderCircle data-icon="inline-start" className="animate-spin" />}
                确认
              </Button>
            )}
          </DialogFooter>
        </Tabs>
      </form>
    </AppDialog>
  );
}

function BrowserOAuthFields({
  busy,
  oauthMessage,
  oauthUrl,
  callbackUrl,
  setCallbackUrl,
  onGenerate,
  onOpen,
  showCallback = true,
}: {
  busy: string | null;
  oauthMessage: string | null;
  oauthUrl: string;
  callbackUrl: string;
  setCallbackUrl: Setter<string>;
  onGenerate: () => Promise<void>;
  onOpen: () => void;
  showCallback?: boolean;
}) {
  const loading = busy === 'oauth:prepare';
  const completing = busy === 'oauth:complete';
  return (
    <div className="flex flex-col gap-4">
      {oauthUrl ? (
        <>
          <Field>
            <FieldLabel htmlFor="oauth-authorization-url">授权链接</FieldLabel>
            <InputGroup>
              <InputGroupInput
                id="oauth-authorization-url"
                value={oauthUrl}
                readOnly
                tabIndex={-1}
              />
              <InputGroupAddon align="inline-end">
                <OAuthIconButton
                  label="打开授权链接"
                  onClick={onOpen}
                  disabled={loading || completing}
                >
                  <ExternalLink />
                </OAuthIconButton>
                <CopyButton
                  value={oauthUrl}
                  label="复制授权链接"
                  disabled={loading || completing}
                />
                <RefreshButton
                  label="重新生成授权链接并打开"
                  onRefresh={onGenerate}
                  disabled={loading || completing}
                />
              </InputGroupAddon>
            </InputGroup>
          </Field>
          {showCallback && (
            <Field>
              <FieldLabel htmlFor="oauth-callback-url">回调链接</FieldLabel>
              <Input
                id="oauth-callback-url"
                value={callbackUrl}
                onChange={(event) => setCallbackUrl(event.target.value)}
                placeholder="粘贴浏览器最终跳转的链接"
                autoComplete="off"
                spellCheck={false}
                disabled={completing}
              />
            </Field>
          )}
        </>
      ) : (
        <Button
          type="button"
          variant="outline"
          className="w-fit"
          onClick={onGenerate}
          disabled={loading}
        >
          {loading && <LoaderCircle data-icon="inline-start" className="animate-spin" />}
          生成授权链接并打开
        </Button>
      )}
      {oauthMessage && (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          {completing && <LoaderCircle size={15} className="animate-spin" />} {oauthMessage}
        </p>
      )}
    </div>
  );
}

function OAuthIconButton({
  label,
  children,
  onClick,
  disabled = false,
}: {
  label: string;
  children: ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={label}
          onClick={onClick}
          disabled={disabled}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
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
  modelStatus,
  customModelEnabled,
  modelProfileId,
  defaultModelId,
  setAlias,
  setAuthJson,
  setRelayApiKey,
  setRelayApiBaseUrl,
  setShowRelayApiKey,
  setCustomModelEnabled,
  setModelProfileId,
  setDefaultModelId,
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
  modelStatus: ModelProfilesStatus | null;
  customModelEnabled: boolean;
  modelProfileId: string | null;
  defaultModelId: string | null;
  setAlias: Setter<string>;
  setAuthJson: Setter<string>;
  setRelayApiKey: Setter<string>;
  setRelayApiBaseUrl: Setter<string>;
  setShowRelayApiKey: Setter<boolean>;
  setCustomModelEnabled: Setter<boolean>;
  setModelProfileId: Setter<string | null>;
  setDefaultModelId: Setter<string | null>;
  onSubmit: (event: FormEvent) => void;
  onClose: () => void;
}) {
  return (
    <AppDialog title="编辑账户" contentClassName="sm:max-w-2xl" onClose={onClose}>
      <form className="flex flex-col gap-4" onSubmit={onSubmit}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="editing-alias">别名</FieldLabel>
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
              <Field>
                <FieldLabel htmlFor="editing-relay-api-key">API Key</FieldLabel>
                <SecretInput
                  id="editing-relay-api-key"
                  visible={showRelayApiKey}
                  value={relayApiKey}
                  onChange={setRelayApiKey}
                  onToggle={() => setShowRelayApiKey((visible) => !visible)}
                  placeholder="留空则保留现有 API Key"
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="editing-relay-api-base-url">API 地址</FieldLabel>
                <Input
                  id="editing-relay-api-base-url"
                  type="url"
                  value={relayApiBaseUrl}
                  onChange={(event) => setRelayApiBaseUrl(event.target.value)}
                  autoComplete="url"
                  required
                />
              </Field>
              {(editing.product === 'codex' ||
                editing.product === 'claude' ||
                editing.product === 'grok') && (
                <ModelProfileFields
                  status={modelStatus}
                  enabled={customModelEnabled}
                  profileId={modelProfileId}
                  defaultModelId={defaultModelId}
                  onEnabledChange={setCustomModelEnabled}
                  onProfileChange={(value) => {
                    setModelProfileId(value);
                    setDefaultModelId(
                      modelStatus?.profiles.find((profile) => profile.id === value)?.models[0]
                        ?.id ?? null,
                    );
                  }}
                  onDefaultModelChange={setDefaultModelId}
                />
              )}
            </>
          ) : editing.product === 'codex' || editing.product === 'grok' ? (
            <Field>
              <FieldLabel htmlFor="editing-auth-json">auth.json</FieldLabel>
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
          ) : null}
        </FieldGroup>
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
  busy,
  onConfirm,
  onClose,
}: {
  confirm: NonNullable<PendingConfirm>;
  busy: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const product = productName(confirm.profile.product);
  return (
    <AppDialog
      title={confirm.kind === 'delete' ? '移除账户档案' : '覆盖当前登录状态'}
      onClose={() => !busy && onClose()}
    >
      <div>
        <p className="text-sm leading-6 text-muted-foreground">
          {confirm.kind === 'delete'
            ? confirm.profile.product === 'claude' && confirm.profile.isActive
              ? confirm.profile.accountType === 'relay'
                ? `将移除“${confirm.profile.alias}”的本地认证档案，并从 settings.json 清除当前 Claude 中转站凭据。`
                : `将移除“${confirm.profile.alias}”的本地认证档案，并从 settings.json 清除当前 Claude OAuth Token。Keychain 登录保持不变。`
              : confirm.profile.product === 'grok' &&
                  confirm.profile.accountType === 'relay' &&
                  confirm.profile.isActive
                ? `将停用并移除“${confirm.profile.alias}”，同时重建 Grok 中转模型配置。`
                : confirm.profile.isActive
                  ? `将移除“${confirm.profile.alias}”的本地认证档案。当前 ${product} 登录保持不变，但不再由本应用管理。`
                  : `将移除“${confirm.profile.alias}”的本地认证档案。`
            : confirm.kind === 'force-grok-relay'
              ? `当前 Grok 登录、API 或模型配置在应用外被修改。继续会覆盖托管配置并${confirm.enabled ? '启用' : '停用'}“${confirm.profile.alias}”。`
              : confirm.kind === 'force-grok-edit'
                ? `当前 Grok API 或模型配置在应用外被修改。继续会保存“${confirm.profile.alias}”并覆盖托管配置。`
                : `当前 ${product} 登录、API 或模型配置在应用外被修改。继续会立即切换到“${confirm.profile.alias}”。`}
        </p>
        <DialogFooter>
          <CancelButton disabled={busy} />
          <Button
            variant={confirm.kind === 'delete' ? 'destructive' : 'default'}
            type="button"
            onClick={onConfirm}
            disabled={busy}
          >
            {busy && <LoaderCircle data-icon="inline-start" className="animate-spin" />}
            {confirm.kind === 'delete'
              ? '移除'
              : confirm.kind === 'force-grok-relay' || confirm.kind === 'force-grok-edit'
                ? '确认覆盖'
                : '仍要切换'}
          </Button>
        </DialogFooter>
      </div>
    </AppDialog>
  );
}

function ModelProfileFields({
  status,
  enabled,
  profileId,
  defaultModelId,
  onEnabledChange,
  onProfileChange,
  onDefaultModelChange,
}: {
  status: ModelProfilesStatus | null;
  enabled: boolean;
  profileId: string | null;
  defaultModelId: string | null;
  onEnabledChange: Setter<boolean>;
  onProfileChange: (value: string) => void;
  onDefaultModelChange: Setter<string | null>;
}) {
  const profiles = status?.profiles.filter((item) => item.models.length) ?? [];
  const profile = profiles.find((item) => item.id === profileId);
  const modelOptions = uniqueModelsById(profile?.models ?? []);
  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <label className="flex items-center justify-between gap-3 text-sm font-medium">
        自定义模型
        <Switch
          checked={enabled}
          onCheckedChange={(checked) => {
            onEnabledChange(checked);
            if (checked && !profileId) {
              const first = profiles[0];
              onProfileChange(first?.id ?? '');
            }
          }}
          disabled={!profiles.length}
        />
      </label>
      {!profiles.length && (
        <p className="text-xs text-muted-foreground">暂无可用模型方案，请先在模型管理中创建。</p>
      )}
      {enabled && profiles.length ? (
        <div className="grid grid-cols-2 gap-3">
          <Field>
            <FieldLabel>模型方案</FieldLabel>
            <Select value={profileId} onValueChange={(value) => value && onProfileChange(value)}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="选择方案">{profile?.name}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {profiles.map((item) => (
                    <SelectItem key={item.id} value={item.id}>
                      {item.name}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          {profile ? (
            <Field>
              <FieldLabel>默认模型</FieldLabel>
              <Select value={defaultModelId} onValueChange={onDefaultModelChange}>
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="选择默认模型" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {modelOptions.map((model) => (
                      <SelectItem key={model.id} value={model.id}>
                        {model.displayName}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </Field>
          ) : null}
        </div>
      ) : null}
    </div>
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
