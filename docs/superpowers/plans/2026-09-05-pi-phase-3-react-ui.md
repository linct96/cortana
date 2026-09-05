# Pi Phase 3 React Product Integration and UI Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the Phase 2 Pi account backend in Cortana's React UI with a focused account-switching experience and centralize product capability decisions so Pi does not create a new layer of scattered product conditionals.

**Architecture:** Add a small `products.ts` metadata/capability registry for product label, icon, supported routes, and account actions. Existing products retain their current UI behavior; Pi exposes only Accounts plus global Settings. Reuse `useAccountManager` for status/import/switch/reorder/delete while gating OAuth, relay, usage, models, gateway, and raw credential editing through capabilities.

**Tech Stack:** React 19, TypeScript 7, Tauri invoke bridge, TanStack Router, Base UI/shadcn components, lucide-react, Vitest, oxlint, oxfmt, Vite.

**Spec:** `docs/superpowers/specs/2026-09-05-pi-account-switching-design.md`

**Depends on:**
- `docs/superpowers/plans/2026-09-05-pi-phase-1-database-rust-adapter.md`
- `docs/superpowers/plans/2026-09-05-pi-phase-2-auth-sync-switching.md`

## Global Constraints

- Pi is an independent product named `Pi`.
- Pi Phase 3 must not parse or receive access/refresh tokens in React.
- Pi UI supports only: status, sync current account, switch, reorder, edit alias, delete.
- Pi UI does not support: browser OAuth, pasted credentials, relay, usage refresh, reset credits, quotas, gateway, models, sessions, analytics, prompts, config, or open-CLI-from-profile.
- Pi current account and Codex current account are independent; UI copy must say `当前 Pi 账号` where ambiguity matters.
- Pi `authPath` is visible in the status card.
- Missing-state guidance tells the user to run Pi `/login` and choose OpenAI (ChatGPT Plus/Pro).
- Existing Codex, Claude, Antigravity, and Grok visible behavior must not regress.
- Do not add React Testing Library solely for this work; test pure product policy helpers with the already-installed Vitest and verify component integration through TypeScript/Vite build.

---

## File Structure

**Create**

- `apps/desktop/src/products.ts` — central product metadata/capability policy.
- `apps/desktop/src/products.test.ts` — capability and route policy tests.
- `apps/desktop/src/assets/pi.svg` — monochrome Pi product glyph.

**Modify**

- `apps/desktop/src/components/app-shell-context.ts` — consume/re-export the centralized `AccountProduct` and `productName` definitions.
- `apps/desktop/src/components/app-shell.tsx` — Pi product menu entry, capability-driven navigation, unsupported-route redirect, Pi CLI-alert suppression.
- `apps/desktop/src/features/accounts/types.ts` — add `Profile.providerKey`.
- `apps/desktop/src/features/accounts/use-account-manager.ts` — capability-gate Pi-inapplicable workflows and support alias-only editing.
- `apps/desktop/src/features/accounts/accounts-page.tsx` — Pi header/status/empty-state actions and capability-driven sections.
- `apps/desktop/src/features/accounts/account-list.tsx` — Pi icon/meta/actions; hide usage/refresh/reset/model/CLI affordances.
- `apps/desktop/src/features/accounts/account-dialog.tsx` — Pi alias-only edit mode; prevent Pi from using existing add/raw-auth/relay forms.

**Do not modify**

- Pi auth file format or lock implementation from Phase 2.
- Codex OAuth flow.
- Gateway, Models, Sessions, Analytics, Prompts, Config backend implementations.

---

### Task 1: Centralize product metadata and capabilities

**Files:**
- Create: `apps/desktop/src/products.ts`
- Create: `apps/desktop/src/products.test.ts`
- Create: `apps/desktop/src/assets/pi.svg`
- Modify: `apps/desktop/src/components/app-shell-context.ts`

**Interfaces:**
- Produces: `AccountProduct = 'codex' | 'claude' | 'antigravity' | 'grok' | 'pi'`.
- Produces: `productMeta(product)`.
- Produces: `productName(product)`.
- Produces: `productSupportsPath(product, path)`.
- Produces: `ProductCapabilities` consumed by shell/accounts UI.

- [ ] **Step 1: Write failing product policy tests**

Create `apps/desktop/src/products.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { productMeta, productSupportsPath } from './products';

describe('Pi product capabilities', () => {
  it('exposes only account management in the main product navigation', () => {
    const pi = productMeta('pi');
    expect(pi.name).toBe('Pi');
    expect(pi.capabilities.models).toBe(false);
    expect(pi.capabilities.sessions).toBe(false);
    expect(pi.capabilities.analytics).toBe(false);
    expect(pi.capabilities.prompts).toBe(false);
    expect(pi.capabilities.config).toBe(false);
    expect(pi.capabilities.gateway).toBe(false);
    expect(pi.capabilities.usage).toBe(false);
    expect(pi.capabilities.relay).toBe(false);
    expect(pi.capabilities.browserOAuth).toBe(false);
    expect(pi.capabilities.importCurrent).toBe(true);
    expect(pi.capabilities.showAuthPath).toBe(true);
    expect(pi.capabilities.showCliAlert).toBe(false);
  });

  it('redirects unsupported Pi product routes to accounts', () => {
    expect(productSupportsPath('pi', '/accounts')).toBe(true);
    expect(productSupportsPath('pi', '/models')).toBe(false);
    expect(productSupportsPath('pi', '/sessions')).toBe(false);
    expect(productSupportsPath('pi', '/config')).toBe(false);
  });
});
```

Add regression assertions that current products preserve current model-nav behavior:

```ts
expect(productMeta('codex').capabilities.models).toBe(true);
expect(productMeta('claude').capabilities.models).toBe(true);
expect(productMeta('grok').capabilities.models).toBe(true);
expect(productMeta('antigravity').capabilities.models).toBe(false);
```

- [ ] **Step 2: Run tests and verify they fail**

```bash
pnpm --filter @cortana/desktop exec vitest run src/products.test.ts
```

Expected: FAIL because `products.ts` does not exist.

- [ ] **Step 3: Add the Pi SVG asset**

Create `apps/desktop/src/assets/pi.svg` as a simple monochrome stroke icon that works with the same compact product-menu sizing:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">
  <path d="M4 6h16M8.5 6c-.2 5.6-1.6 9.7-4.5 12M15.2 6v8.6c0 2.3 1.2 3.4 3.6 3.4H20" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
```

If the existing image pipeline does not propagate `currentColor` for imported SVG image URLs, use a fixed neutral foreground-compatible fill/stroke consistent with the repository's existing product assets; do not introduce colored Pi branding in this phase.

- [ ] **Step 4: Implement `products.ts`**

Use these exact capability keys:

```ts
export type AccountProduct = 'codex' | 'claude' | 'antigravity' | 'grok' | 'pi';

export type ProductCapabilities = {
  models: boolean;
  sessions: boolean;
  analytics: boolean;
  prompts: boolean;
  config: boolean;
  gateway: boolean;
  usage: boolean;
  relay: boolean;
  browserOAuth: boolean;
  pasteCredential: boolean;
  importCurrent: boolean;
  refreshAccount: boolean;
  refreshAllAccounts: boolean;
  openCliFromAccount: boolean;
  showAuthPath: boolean;
  showCliAlert: boolean;
};
```

Define a `PRODUCTS: Record<AccountProduct, ProductMeta>` mapping. Preserve existing behavior for old products; for Pi use:

```ts
pi: {
  name: 'Pi',
  icon: piIcon,
  capabilities: {
    models: false,
    sessions: false,
    analytics: false,
    prompts: false,
    config: false,
    gateway: false,
    usage: false,
    relay: false,
    browserOAuth: false,
    pasteCredential: false,
    importCurrent: true,
    refreshAccount: false,
    refreshAllAccounts: false,
    openCliFromAccount: false,
    showAuthPath: true,
    showCliAlert: false,
  },
},
```

`productSupportsPath` must return true for `/accounts` for every product and gate the other product routes from the capability fields.

Global `/settings/**` routes are not product routes and must remain accessible.

- [ ] **Step 5: Make app-shell context use the centralized type**

Replace the local `AccountProduct` union and `productName` ternary in `app-shell-context.ts` with imports/re-exports:

```ts
import { productName, type AccountProduct } from '../products';
export { productName, type AccountProduct } from '../products';
```

Keep `AppShellContextValue` otherwise unchanged.

- [ ] **Step 6: Run product policy tests**

```bash
pnpm --filter @cortana/desktop exec vitest run src/products.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src/products.ts apps/desktop/src/products.test.ts apps/desktop/src/assets/pi.svg apps/desktop/src/components/app-shell-context.ts
git commit -m "feat: add Pi product capabilities"
```

---

### Task 2: Add Pi to ProductMenu and capability-drive the main navigation

**Files:**
- Modify: `apps/desktop/src/components/app-shell.tsx`
- Modify: `apps/desktop/src/products.test.ts`

**Interfaces:**
- Consumes: `productMeta`, `productSupportsPath`.
- Produces: Pi selectable from ProductMenu.
- Produces: Pi exposes only Accounts in product navigation.
- Produces: unsupported Pi deep links redirect to `/accounts`.

- [ ] **Step 1: Extend route-policy tests**

Add:

```ts
for (const path of ['/analytics', '/sessions', '/prompts', '/models', '/config'] as const) {
  expect(productSupportsPath('pi', path)).toBe(false);
}
```

Also assert existing products continue supporting the routes that were previously unconditionally shown.

- [ ] **Step 2: Replace product-menu boolean chains with metadata**

In `ProductMenu`, remove the `claude` / `antigravity` / `grok` icon/name ternaries. Use:

```ts
const activeMeta = productMeta(activeProduct);
```

Render trigger icon/name from `activeMeta`.

Render menu items by iterating the stable product order:

```ts
const PRODUCT_ORDER: AccountProduct[] = ['antigravity', 'claude', 'codex', 'grok', 'pi'];
```

Each item uses `productMeta(product).icon`, `.name`, and the existing `selectProduct(product)` flow.

- [ ] **Step 3: Capability-drive sidebar items**

Keep Accounts always visible.

Replace current product checks with capability checks:

```tsx
const capabilities = productMeta(activeProduct).capabilities;

{capabilities.analytics && <SidebarNavItem ... />}
{capabilities.sessions && <SidebarNavItem ... />}
{capabilities.prompts && <SidebarNavItem ... />}
{capabilities.models && <SidebarNavItem ... />}
{capabilities.config && <SidebarNavItem ... />}
```

Settings footer remains visible for Pi.

- [ ] **Step 4: Redirect unsupported Pi product routes**

In `MainLayout`, observe the current pathname and add an effect:

```ts
useEffect(() => {
  const path = mainPathFor(pathname);
  if (path && !productSupportsPath(activeProduct, path)) {
    void navigate({ to: '/accounts', replace: true, ignoreBlocker: true });
  }
}, [activeProduct, navigate, pathname]);
```

This prevents old bookmarks such as `/models` from invoking unsupported Pi views.

- [ ] **Step 5: Suppress the generic CLI alert for Pi**

In `AppContent`, if `productMeta(activeProduct).capabilities.showCliAlert` is false, do not render `CliAlert` and do not invoke `is_pi_cli_available`.

Do not add a fake `is_pi_cli_available` Tauri command just to satisfy the old UI path.

- [ ] **Step 6: Run tests and build**

```bash
pnpm --filter @cortana/desktop exec vitest run src/products.test.ts
pnpm --filter @cortana/desktop build:web
```

Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src/components/app-shell.tsx apps/desktop/src/products.test.ts
git commit -m "feat: expose Pi product navigation"
```

---

### Task 3: Add `providerKey` to the frontend account contract

**Files:**
- Modify: `apps/desktop/src/features/accounts/types.ts`

**Interfaces:**
- Consumes: Phase 1 backend `providerKey` JSON field.
- Produces: `Profile.providerKey: string`.

- [ ] **Step 1: Update the TypeScript contract**

Add immediately after `product`:

```ts
providerKey: string;
```

The field is intentionally not optional. Existing products receive `''`; Pi `openai-codex` profiles receive `'openai-codex'`.

- [ ] **Step 2: Run TypeScript build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0, or compile failures that identify mocked/test profile objects needing `providerKey: ''`.

- [ ] **Step 3: Fix only compile-reported profile fixtures**

For existing-product fixtures use:

```ts
providerKey: '',
```

For Pi fixtures use:

```ts
providerKey: 'openai-codex',
```

Do not make the field optional to silence fixtures.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/features/accounts/types.ts apps/desktop/src
git commit -m "feat: expose account provider keys in desktop UI"
```

---

### Task 4: Make `useAccountManager` Pi-safe and alias-only

**Files:**
- Modify: `apps/desktop/src/features/accounts/use-account-manager.ts`
- Modify: `apps/desktop/src/products.ts`

**Interfaces:**
- Consumes: `productMeta(product).capabilities`.
- Reuses: existing `refresh`, `switchTo`, `importCurrent`, `reorderProfiles`, `deleteProfile`.
- Produces: Pi `openEditor` and `saveProfile` path that never fetches or submits raw credential JSON.

- [ ] **Step 1: Compute capabilities once**

Near the hook start:

```ts
const capabilities = productMeta(product).capabilities;
```

Return `capabilities` from the hook so account components do not recompute product policy independently.

- [ ] **Step 2: Stop automatic unsupported refreshes**

Where the hook currently refreshes Gateway, Models, or usage-related state based on product branches, gate with capabilities:

```ts
if (capabilities.gateway) { ... }
if (capabilities.models) { ... }
```

For Pi, `refresh()` must only invoke:

```ts
invoke<AppStatus>('get_app_status', { product: 'pi' })
```

No `get_codex_gateway_mode`, model status, or account usage call may run.

- [ ] **Step 3: Keep `importCurrent` generic**

Pi uses the existing command:

```ts
invoke<Profile>('import_current_profile', {
  product,
  alias: undefined,
})
```

After success, call `refresh()` and use product-neutral success copy based on `productName(product)`.

Do not open AddAccountDialog for this path.

- [ ] **Step 4: Make editor opening capability-aware**

For Pi OAuth profiles, do not call `get_profile_auth`.

Use:

```ts
if (product === 'pi') {
  setEditing(profile);
  setEditingAlias(profile.alias);
  setEditingAuthJson('');
  return;
}
```

The product check here is acceptable because it selects a backend contract, not a visual capability; do not replicate it across components.

- [ ] **Step 5: Make Pi save alias-only**

When saving Pi:

```ts
await invoke<Profile>('update_profile', {
  profileId: editing.id,
  alias: editingAlias.trim(),
  authJson: null,
  product,
});
```

Do not synthesize or send the profile credential.

- [ ] **Step 6: Guard unsupported public hook actions**

Functions such as `generateOAuthLink`, `submitAdd`, `refreshAccount`, relay editing, and model assignment should either never be exposed for Pi or fail immediately with a UI-internal guard if invoked accidentally.

Use stable guard helpers such as:

```ts
if (!capabilities.refreshAccount) return;
```

Do not call unsupported Tauri commands and rely on backend errors for normal UI flow.

- [ ] **Step 7: Run TypeScript build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0.

- [ ] **Step 8: Commit**

```bash
git add apps/desktop/src/features/accounts/use-account-manager.ts apps/desktop/src/products.ts
git commit -m "feat: make account manager capability aware"
```

---

### Task 5: Build the Pi account page status and sync flow

**Files:**
- Modify: `apps/desktop/src/features/accounts/accounts-page.tsx`

**Interfaces:**
- Consumes: `account.capabilities`, `status.authState`, `status.detectedProfile`, `status.authPath`.
- Produces: Pi-specific account UX without a separate route/component tree.

- [ ] **Step 1: Capability-drive header actions**

For Pi:

- Hide Gateway switch.
- Hide Refresh All button.
- Hide Add Account button.
- Show one primary action: `同步当前账号`.

The button calls:

```tsx
onClick={() => void account.importCurrent()}
```

Disable it when:

```text
busy === 'import'
authState.kind === 'missing'
```

For existing products, preserve current header behavior.

- [ ] **Step 2: Use Pi-specific missing guidance**

When `product === 'pi'` and `authState.kind === 'missing'`, render:

```text
尚未检测到 OpenAI Codex 登录
请先在 Pi 中运行 /login 并登录 OpenAI (ChatGPT Plus/Pro)。
```

Do not say simply `未检测到 auth.json`, because the file may exist for other Providers.

- [ ] **Step 3: Make the current-account label unambiguous**

For Pi use:

```text
当前 Pi 账号
```

For other products keep the existing label/copy.

For unmanaged Pi state, show the detected temporary profile and a `同步当前账号` button instead of presenting it as a saved active profile.

- [ ] **Step 4: Show Pi auth path**

Use `capabilities.showAuthPath` instead of the current Claude/Antigravity-only condition:

```tsx
{account.capabilities.showAuthPath && account.status.authPath && (
  <span className="...font-mono...">{account.status.authPath}</span>
)}
```

- [ ] **Step 5: Hide Pi usage and relay side panels**

The current account balance panel must require:

```ts
account.capabilities.usage
```

The current relay panel must require:

```ts
account.capabilities.relay
```

This prevents Pi OAuth profiles from displaying `额度未查询` or irrelevant relay information.

- [ ] **Step 6: Add Pi empty state**

Change `EmptyState` to receive `product` and `onImportCurrent` in addition to existing add behavior.

Pi copy:

```text
尚未保存 Pi 账号
先在 Pi 中使用 /login 登录 OpenAI (ChatGPT Plus/Pro)，然后回到 Cortana 同步当前账号。
```

Pi button:

```text
同步当前账号
```

Other products retain `添加账号` behavior.

- [ ] **Step 7: Do not mount AddAccountDialog for Pi**

Require an add capability before mounting the existing add dialog. Since capabilities are split by OAuth/paste/relay, define add availability as:

```ts
const canAddAccount =
  account.capabilities.browserOAuth ||
  account.capabilities.pasteCredential ||
  account.capabilities.relay;
```

Then:

```tsx
{canAddAccount && account.addOpen && <AddAccountDialog ... />}
```

- [ ] **Step 8: Build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0.

- [ ] **Step 9: Commit**

```bash
git add apps/desktop/src/features/accounts/accounts-page.tsx
git commit -m "feat: add Pi account status and sync UI"
```

---

### Task 6: Make account rows Provider-aware and remove unsupported Pi actions

**Files:**
- Modify: `apps/desktop/src/features/accounts/account-list.tsx`
- Modify: `apps/desktop/src/products.ts`

**Interfaces:**
- Consumes: profile product metadata and capabilities.
- Produces: Pi row meta `OpenAI Codex · <short accountId>`.
- Produces: only Switch/Edit/Delete actions for Pi.

- [ ] **Step 1: Use product metadata for OAuth icons**

Replace the nested product-icon ternary with:

```ts
const meta = productMeta(profile.product);
```

Then use `meta.icon` for OAuth profile rows.

Existing relay rows retain their initial-letter behavior.

- [ ] **Step 2: Add a short account-id helper**

Add a pure helper:

```ts
export function shortAccountId(accountId: string) {
  const value = accountId.trim();
  return value.length <= 8 ? value : `…${value.slice(-8)}`;
}
```

Add Vitest coverage in `products.test.ts` or a nearby pure helper test file:

```ts
expect(shortAccountId('1234567890abcdef')).toBe('…90abcdef');
```

- [ ] **Step 3: Render Pi metadata without usage fallback**

At the start of `AccountMeta`:

```tsx
if (profile.product === 'pi') {
  return (
    <div className="mt-1.5 flex min-w-0 items-center text-xs text-muted-foreground">
      <span>OpenAI Codex</span>
      {profile.accountId && <MetaSeparatorItem label={shortAccountId(profile.accountId)} />}
    </div>
  );
}
```

Do not fall through to the generic OAuth branch, which currently adds `额度未查询`.

- [ ] **Step 4: Capability-gate row actions**

For Pi:

- Keep Switch when not active.
- Keep Edit.
- Keep Delete.
- Hide Refresh OAuth action.
- Hide Open Terminal.
- Hide quota/reset-credit actions.
- Model-profile badges will naturally be absent because Phase 3 does not load model status for Pi.

Use `productMeta(profile.product).capabilities` for these checks rather than adding repeated Pi conditions.

- [ ] **Step 5: Run tests/build**

```bash
pnpm --filter @cortana/desktop exec vitest run
pnpm --filter @cortana/desktop build:web
```

Expected: both exit 0.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/features/accounts/account-list.tsx apps/desktop/src/products.ts apps/desktop/src/products.test.ts
git commit -m "feat: tailor Pi account list actions"
```

---

### Task 7: Add Pi alias-only edit dialog behavior

**Files:**
- Modify: `apps/desktop/src/features/accounts/account-dialog.tsx`

**Interfaces:**
- Consumes: `editing.product === 'pi'` and alias-only `saveProfile` behavior from Task 4.
- Produces: Pi edit form that cannot display/edit credentials.

- [ ] **Step 1: Add a dedicated early Pi branch in `EditAccountDialog`**

Before the existing credential/relay editor paths, render only:

```tsx
if (editing.product === 'pi') {
  return (
    <AppDialog title="编辑账号" contentClassName="sm:max-w-md" onClose={onClose}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <Field>
          <FieldLabel htmlFor="edit-pi-alias">别名</FieldLabel>
          <Input
            id="edit-pi-alias"
            value={alias}
            onChange={(event) => setAlias(event.target.value)}
            placeholder="例如：工作账户"
            required
          />
        </Field>
        <DialogFooter>
          <CancelButton />
          <Button type="submit" disabled={Boolean(busy)}>
            {busy === `edit:${editing.id}` && (
              <LoaderCircle data-icon="inline-start" className="animate-spin" />
            )}
            保存
          </Button>
        </DialogFooter>
      </form>
    </AppDialog>
  );
}
```

Adapt the busy expression to the hook's actual existing edit busy key; do not invent a second state machine.

The important invariant is that no auth JSON textarea, relay key, model selection, or OAuth action is rendered for Pi.

- [ ] **Step 2: Ensure AddAccountDialog never needs a Pi branch**

Do not add Pi tabs to `AddAccountDialog`. Task 5 prevents the dialog from mounting for Pi.

If defensive typing requires a guard, fail closed:

```ts
if (product === 'pi') return null;
```

Do not expose browser OAuth or relay tabs.

- [ ] **Step 3: Build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/features/accounts/account-dialog.tsx
git commit -m "feat: add Pi alias-only account editor"
```

---

### Task 8: Verify force-switch and delete UX with the existing confirmation system

**Files:**
- Modify only if needed: `apps/desktop/src/features/accounts/accounts-page.tsx`
- Modify only if needed: `apps/desktop/src/features/accounts/types.ts`
- Modify only if needed: `apps/desktop/src/features/accounts/use-account-manager.ts`

**Interfaces:**
- Reuses: existing `PendingConfirm` `force-switch` and `delete` variants.
- No new Pi-specific confirm variant should be added.

- [ ] **Step 1: Exercise the existing force-switch path in code**

Confirm `switchTo(profile)` catches the backend external-change error and sets:

```ts
{ kind: 'force-switch', profile }
```

If the current implementation already does this generically, make no change.

The Confirm dialog must eventually call:

```ts
account.switchTo(account.confirm.profile, true)
```

for Pi exactly as it does for other force-switch cases.

- [ ] **Step 2: Verify delete does not imply logout in copy**

Pi delete confirmation must describe removing the saved Cortana profile, not logging out of Pi. If the current generic text says only `删除账号`, it is acceptable. If it explicitly promises logout/revocation, add Pi-safe copy.

- [ ] **Step 3: Build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0.

- [ ] **Step 4: Commit only if code changed**

```bash
git add apps/desktop/src/features/accounts
git commit -m "fix: align Pi switch confirmation UX"
```

Skip this commit if the existing confirmation path already satisfies the design.

---

### Task 9: Run the full frontend and Rust regression gates

**Files:**
- No source changes unless a gate identifies a defect.

- [ ] **Step 1: Formatting**

```bash
pnpm --filter @cortana/desktop exec oxfmt --check .
```

Expected: exit 0.

- [ ] **Step 2: Lint**

```bash
pnpm --filter @cortana/desktop exec oxlint src
```

Expected: exit 0.

- [ ] **Step 3: Frontend tests**

```bash
pnpm --filter @cortana/desktop exec vitest run
```

Expected: all tests pass.

- [ ] **Step 4: Frontend build**

```bash
pnpm --filter @cortana/desktop build:web
```

Expected: exit 0.

- [ ] **Step 5: Rust format/lint/tests**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all commands exit 0.

- [ ] **Step 6: Manual Tauri smoke test**

Run:

```bash
pnpm dev
```

Use a disposable Pi agent directory, not the developer's real credential file:

```bash
export PI_CODING_AGENT_DIR="$(mktemp -d)/agent"
mkdir -p "$PI_CODING_AGENT_DIR"
```

Seed a fixture `auth.json` with fake credentials before starting Cortana. Verify visually:

1. Product menu contains Pi.
2. Selecting Pi lands on Accounts.
3. Only Accounts + global Settings are visible.
4. No generic Pi CLI warning appears.
5. Missing Pi credential shows `/login` guidance.
6. Unmanaged credential shows `同步当前账号`.
7. Managed profile rows show `OpenAI Codex · …<accountId>` and no usage text.
8. Row actions are only Switch/Edit/Delete.
9. Pi edit dialog contains only alias.
10. Direct navigation to `/models`, `/sessions`, `/analytics`, `/prompts`, `/config` while Pi is active redirects to `/accounts`.

Do not use real OpenAI access/refresh tokens in screenshots, logs, or test fixtures.

- [ ] **Step 7: Commit only validation fixes**

```bash
git add -A
git commit -m "fix: polish Pi account switching UI"
```

Skip if no files changed.

---

## Phase 3 Exit Criteria

- [ ] Pi is selectable as a product.
- [ ] Product menu uses centralized metadata rather than a new Pi ternary chain.
- [ ] Pi product navigation contains Accounts only; global Settings remains available.
- [ ] Unsupported Pi product routes redirect to `/accounts`.
- [ ] Pi does not invoke `is_pi_cli_available` or display the generic CLI warning.
- [ ] Pi account page does not display Gateway, Models, Usage, Relay, OAuth, pasted-auth, reset-credit, quota, or open-terminal controls.
- [ ] Missing state explicitly directs the user to Pi `/login` + OpenAI (ChatGPT Plus/Pro).
- [ ] Unmanaged state can synchronize the current account.
- [ ] Managed state displays `当前 Pi 账号`, alias, Provider, short accountId, and auth path.
- [ ] Pi account rows expose only Switch/Edit/Delete.
- [ ] Pi edit is alias-only and never fetches/sends raw credential JSON.
- [ ] Delete copy does not imply Pi logout.
- [ ] Existing force-switch confirmation is reused for external login changes.
- [ ] Existing Codex/Claude/Antigravity/Grok navigation and account actions remain unchanged.
- [ ] oxfmt, oxlint, Vitest, frontend build, cargo fmt, clippy, and cargo test all pass.

## Final Feature Acceptance

After all three phases, validate the design-level scenario end to end with disposable Pi credentials:

```text
Pi login A
-> Cortana sync A
-> Pi login B
-> Cortana sync B
-> switch A
-> switch B
-> Pi changes A credential tokens while accountId stays A
-> Cortana observes/synchronizes A
-> switch B
-> switch A
```

At every switch, verify unrelated Provider entries in Pi `auth.json` remain semantically identical.

The implementation is complete only when the Phase 1, Phase 2, and Phase 3 exit criteria all hold simultaneously.