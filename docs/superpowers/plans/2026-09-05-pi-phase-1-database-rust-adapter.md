# Pi Phase 1 Database and Rust Product Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Pi as a first-class backend product, introduce Provider-scoped account storage, and establish the Rust Pi adapter boundary without implementing account switching yet.

**Architecture:** Keep Cortana's current `AccountProduct` model and add `Pi`. Extend the shared `accounts` table with `provider_key`, defaulting to `''` for all existing products and using `openai-codex` for Pi. Add a small `products::pi` adapter that owns Pi path/provider constants and credential shape validation; Phase 2 will build synchronization and switching transactions on top of these interfaces.

**Tech Stack:** Rust 2021, Tauri 2, rusqlite 0.40, serde/serde_json, existing SQLite migration style, cargo test/clippy/rustfmt.

**Spec:** `docs/superpowers/specs/2026-09-05-pi-account-switching-design.md`

## Global Constraints

- Cortana design baseline: `cf6b191ea3216eeea3b4cae9244f4293482efcf6`.
- Pi reference baseline: `da840b6216578c2a571d0374ac6a2091a83f9d91`.
- Pi is an independent `AccountProduct`; do not model it as a Codex mode.
- Pi profiles are Provider credential profiles, not full `auth.json` snapshots.
- Phase 1 supports only Provider key `openai-codex` and OAuth credentials.
- Existing Codex, Claude, Antigravity, and Grok rows must retain identical behavior with `provider_key = ''`.
- Do not add Pi OAuth, usage refresh, relay support, model support, CLI launching, or frontend Pi UI in this phase.
- Do not share Codex and Pi refresh tokens or auth snapshots.
- Keep database migrations idempotent because Cortana migrates at application startup.
- Every Rust match over `AccountProduct` must explicitly handle `Pi`; unsupported operations must return an explicit error instead of silently falling back to Codex.

---

## File Structure

**Modify**

- `src-tauri/src/platform/db.rs` — add/migrate `provider_key`, rebuild affected unique indexes, and add migration tests.
- `src-tauri/src/platform/state.rs` — add `AccountProduct::Pi` and `ProfileSummary.provider_key`.
- `src-tauri/src/features/accounts/store.rs` — include `provider_key` in generic account queries and profile summaries.
- `src-tauri/src/features/accounts/commands.rs` — make product matches exhaustive and persist/read `active_product = pi`.
- `src-tauri/src/products/mod.rs` — register the Pi product module.

**Create**

- `src-tauri/src/products/pi/mod.rs` — public Pi adapter boundary.
- `src-tauri/src/products/pi/auth.rs` — Pi path/provider constants and read-only credential validation helpers used by later phases.

**Do not modify in Phase 1**

- `apps/desktop/**`
- Codex OAuth implementation.
- Gateway, Models, Sessions, Prompts, Analytics implementations.

---

### Task 1: Add `provider_key` to the SQLite account schema

**Files:**
- Modify: `src-tauri/src/platform/db.rs`

**Interfaces:**
- Produces: `accounts.provider_key TEXT NOT NULL DEFAULT ''`.
- Produces: idempotent `migrate_account_provider_schema(connection: &Connection) -> Result<(), String>`.
- Consumed by: generic account store changes in Task 3 and Pi profile creation in Phase 2.

- [ ] **Step 1: Add failing migration tests**

Extend the existing `#[cfg(test)]` module in `src-tauri/src/platform/db.rs` with tests that create an old-style `accounts` table, run the provider migration twice, and verify the new column/index definitions.

Use this shape:

```rust
#[test]
fn migrates_account_provider_schema_idempotently() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE accounts (
              id TEXT PRIMARY KEY NOT NULL,
              product TEXT NOT NULL,
              account_type TEXT NOT NULL,
              api_base_url TEXT,
              account_id TEXT NOT NULL DEFAULT '',
              email TEXT NOT NULL DEFAULT '',
              upstream_protocol TEXT NOT NULL DEFAULT 'openaiResponses',
              upstream_auth_mode TEXT NOT NULL DEFAULT 'bearer'
            );
            CREATE UNIQUE INDEX accounts_oauth_account_identity_uq
              ON accounts(product, account_id)
              WHERE product <> 'codex' AND account_type = 'oauth' AND account_id <> '';
            CREATE UNIQUE INDEX accounts_oauth_email_identity_uq
              ON accounts(product, email COLLATE NOCASE)
              WHERE product <> 'codex' AND account_type = 'oauth' AND email <> '';
            CREATE UNIQUE INDEX accounts_relay_identity_uq
              ON accounts(product, api_base_url, account_id, upstream_protocol, upstream_auth_mode)
              WHERE account_type = 'relay' AND api_base_url IS NOT NULL AND account_id <> '';
            ",
        )
        .unwrap();

    migrate_account_provider_schema(&connection).unwrap();
    migrate_account_provider_schema(&connection).unwrap();

    assert!(account_column_exists(&connection, "provider_key").unwrap());
    let default: String = connection
        .query_row(
            "SELECT dflt_value FROM pragma_table_info('accounts') WHERE name = 'provider_key'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(default, "''");
}
```

Also add a uniqueness test proving these rows can coexist:

```text
(product=pi, provider_key=openai-codex, account_id=A)
(product=pi, provider_key=another-provider, account_id=A)
```

and that two rows with the same `(pi, openai-codex, A)` are rejected.

- [ ] **Step 2: Run the targeted test and verify it fails**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::db::tests::migrates_account_provider_schema_idempotently
```

Expected: FAIL because `migrate_account_provider_schema` and/or `provider_key` do not exist yet.

- [ ] **Step 3: Implement the migration**

Add:

```rust
fn migrate_account_provider_schema(connection: &Connection) -> Result<(), String> {
    if !account_column_exists(connection, "provider_key")? {
        connection
            .execute_batch(
                "ALTER TABLE accounts ADD COLUMN provider_key TEXT NOT NULL DEFAULT '';",
            )
            .map_err(database_error)?;
    }

    connection
        .execute_batch(
            "
            DROP INDEX IF EXISTS accounts_oauth_account_identity_uq;
            DROP INDEX IF EXISTS accounts_oauth_email_identity_uq;
            DROP INDEX IF EXISTS accounts_relay_identity_uq;

            CREATE UNIQUE INDEX accounts_oauth_account_identity_uq
              ON accounts(product, provider_key, account_id)
              WHERE product <> 'codex' AND account_type = 'oauth' AND account_id <> '';

            CREATE UNIQUE INDEX accounts_oauth_email_identity_uq
              ON accounts(product, provider_key, email COLLATE NOCASE)
              WHERE product <> 'codex' AND account_type = 'oauth' AND email <> '';

            CREATE UNIQUE INDEX accounts_relay_identity_uq
              ON accounts(product, provider_key, api_base_url, account_id, upstream_protocol, upstream_auth_mode)
              WHERE account_type = 'relay' AND api_base_url IS NOT NULL AND account_id <> '';
            ",
        )
        .map_err(database_error)
}
```

Add `provider_key TEXT NOT NULL DEFAULT ''` to the fresh `CREATE TABLE IF NOT EXISTS accounts` definition.

Call the migrations in this order from `initialize_database`:

```rust
migrate_account_gateway_schema(&connection)?;
migrate_account_provider_schema(&connection)?;
```

The provider migration owns the final relay index definition so it runs after the gateway migration.

- [ ] **Step 4: Run DB tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::db::tests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/platform/db.rs
git commit -m "feat: add provider key account schema"
```

---

### Task 2: Add `AccountProduct::Pi` and expose `provider_key` in profile summaries

**Files:**
- Modify: `src-tauri/src/platform/state.rs`

**Interfaces:**
- Produces: `AccountProduct::Pi` serialized as `"pi"`.
- Produces: `ProfileSummary.provider_key: String`, serialized to frontend as `providerKey`.
- Consumed by: account store Task 3, Pi adapter Task 4, Phase 3 TypeScript types.

- [ ] **Step 1: Add failing enum serialization tests**

Add a small test module to `state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::AccountProduct;

    #[test]
    fn pi_product_uses_stable_storage_name() {
        assert_eq!(AccountProduct::Pi.as_str(), "pi");
        assert_eq!(AccountProduct::Pi.display_name(), "Pi");
        assert_eq!(serde_json::to_string(&AccountProduct::Pi).unwrap(), "\"pi\"");
    }
}
```

- [ ] **Step 2: Run the targeted test and verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::state::tests::pi_product_uses_stable_storage_name
```

Expected: FAIL because the `Pi` variant does not exist.

- [ ] **Step 3: Add the product variant and provider field**

Update the enum:

```rust
pub(crate) enum AccountProduct {
    Codex,
    Claude,
    Antigravity,
    Grok,
    Pi,
}
```

Update both match helpers:

```rust
Self::Pi => "pi"
```

and:

```rust
Self::Pi => "Pi"
```

Add to `ProfileSummary` immediately after `product`:

```rust
pub(crate) provider_key: String,
```

Do not make it `Option<String>`; old products use `String::new()`.

- [ ] **Step 4: Run the state test**

```bash
cargo test --manifest-path src-tauri/Cargo.toml platform::state::tests::pi_product_uses_stable_storage_name
```

Expected: PASS. Compilation may now reveal every exhaustive product match and every `ProfileSummary` initializer that must be updated in the next task; do not suppress those errors with wildcard matches.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/platform/state.rs
git commit -m "feat: add Pi account product"
```

---

### Task 3: Thread `provider_key` through the shared account store

**Files:**
- Modify: `src-tauri/src/features/accounts/store.rs`
- Modify: any Rust product file reported by the compiler that directly constructs `ProfileSummary`

**Interfaces:**
- Consumes: `ProfileSummary.provider_key` from Task 2.
- Produces: all DB-loaded profiles expose the persisted provider key.
- Produces: existing products continue exposing `provider_key == ""`.

- [ ] **Step 1: Add a failing generic store test**

In the existing accounts store test module, insert a minimal Pi row with `provider_key = 'openai-codex'` and assert:

```rust
let profile = get_profile_summary_for_product(
    &connection,
    AccountProduct::Pi,
    "pi-a",
    None,
)
.unwrap();
assert_eq!(profile.provider_key, "openai-codex");
```

If the current store tests use a shared schema fixture, update that fixture with `provider_key TEXT NOT NULL DEFAULT ''` rather than constructing a second incompatible schema.

- [ ] **Step 2: Run the targeted store test and verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml features::accounts::store
```

Expected: FAIL until query columns and row mapping include `provider_key`.

- [ ] **Step 3: Update SELECT lists and row mapping**

For both `list_profiles_for_product` and `get_profile_summary_for_product`, select `provider_key` adjacent to `account_type`:

```sql
SELECT id, account_type, provider_key, api_base_url, account_id, ...
```

Update `profile_summary_from_row` indexes consistently and set:

```rust
provider_key: row.get(2)?,
```

Shift subsequent column indexes once, in one edit, rather than mixing old/new positions.

For manually-constructed profiles that are not DB-backed, set:

```rust
provider_key: String::new(),
```

for Codex/Claude/Antigravity/Grok and later use `openai-codex` only inside the Pi adapter.

- [ ] **Step 4: Make product matches exhaustive without enabling Pi-only features**

Compiler errors in helpers such as `relay_api_key_for_profile` must explicitly reject Pi:

```rust
AccountProduct::Pi => {
    return Err("Pi 一期不支持中转站账户。".to_string());
}
```

Use similar explicit errors for any existing relay/model/usage helper whose semantics do not apply to Pi. Do not use `_ =>` because future product additions should remain compiler-visible.

- [ ] **Step 5: Run all Rust tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/features/accounts/store.rs src-tauri/src/products src-tauri/src/features
git commit -m "refactor: carry provider keys through account profiles"
```

---

### Task 4: Add the Pi Rust adapter boundary and path/credential primitives

**Files:**
- Create: `src-tauri/src/products/pi/mod.rs`
- Create: `src-tauri/src/products/pi/auth.rs`
- Modify: `src-tauri/src/products/mod.rs`

**Interfaces:**
- Produces: `PI_PROVIDER_OPENAI_CODEX: &str`.
- Produces: `pi_agent_dir(state: &AppState) -> PathBuf`.
- Produces: `auth_path(state: &AppState) -> PathBuf`.
- Produces: `OpenAiCodexCredential` deserialization/validation helpers.
- Does not produce switching or DB mutation yet.

- [ ] **Step 1: Write failing adapter tests**

Create `auth.rs` with tests first. The production API should be designed around this exact type:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenAiCodexCredential {
    pub(crate) r#type: String,
    pub(crate) access: String,
    pub(crate) refresh: String,
    pub(crate) expires: f64,
    pub(crate) account_id: String,
}
```

Tests must cover:

```rust
#[test]
fn parses_supported_openai_codex_oauth_credential() {
    let value = serde_json::json!({
        "type": "oauth",
        "access": "access",
        "refresh": "refresh",
        "expires": 1234,
        "accountId": "acct-a"
    });
    let credential = parse_openai_codex_credential(&value).unwrap();
    assert_eq!(credential.account_id, "acct-a");
}

#[test]
fn rejects_api_key_credential_in_phase_one() {
    let value = serde_json::json!({ "type": "api_key", "key": "secret" });
    assert!(parse_openai_codex_credential(&value).is_err());
}
```

Also test empty `access`, empty `refresh`, non-finite `expires`, and empty `accountId`.

For path resolution, extract an environment-independent helper so tests do not mutate process-global env in parallel:

```rust
fn resolve_agent_dir(home: &Path, env_value: Option<&OsStr>) -> PathBuf
```

with tests for default `~/.pi/agent` and explicit `PI_CODING_AGENT_DIR`.

- [ ] **Step 2: Run the Pi adapter tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::auth::tests
```

Expected: FAIL because the module/functions are not implemented.

- [ ] **Step 3: Implement the minimal adapter**

`src-tauri/src/products/pi/mod.rs`:

```rust
pub(crate) mod auth;

pub(crate) use auth::{
    auth_path, parse_openai_codex_credential, OpenAiCodexCredential,
    PI_PROVIDER_OPENAI_CODEX,
};
```

`src-tauri/src/products/pi/auth.rs` must define:

```rust
pub(crate) const PI_PROVIDER_OPENAI_CODEX: &str = "openai-codex";
const PI_AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";

pub(crate) fn pi_agent_dir(state: &AppState) -> PathBuf {
    let home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    resolve_agent_dir(home, std::env::var_os(PI_AGENT_DIR_ENV).as_deref())
}

pub(crate) fn auth_path(state: &AppState) -> PathBuf {
    pi_agent_dir(state).join("auth.json")
}
```

`parse_openai_codex_credential` must reject unsupported/invalid values with a stable error string and must not log the JSON value.

- [ ] **Step 4: Register the module and run tests**

Add to `src-tauri/src/products/mod.rs`:

```rust
pub(crate) mod pi;
```

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::auth::tests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/products/mod.rs src-tauri/src/products/pi
git commit -m "feat: add Pi product adapter primitives"
```

---

### Task 5: Persist `active_product = pi` and explicitly gate unsupported backend operations

**Files:**
- Modify: `src-tauri/src/features/accounts/commands.rs`
- Modify: any compile-reported product-dispatch files outside `commands.rs` that match `AccountProduct`

**Interfaces:**
- Produces: `active_product()` maps stored `"pi"` to `AccountProduct::Pi`.
- Produces: `set_active_product(..., AccountProduct::Pi)` persists `"pi"` through existing generic code.
- Phase 2 will add Pi dispatch for `get_app_status`, `import_current_profile`, and `switch_profile`.

- [ ] **Step 1: Add a failing active-product test**

Use a temporary/in-memory database fixture consistent with existing command tests and assert:

```rust
set_setting(&connection, "active_product", "pi").unwrap();
assert_eq!(active_product(&state).unwrap(), AccountProduct::Pi);
```

If `active_product` tests already construct an `AppState`, extend that fixture rather than introducing a second test harness.

- [ ] **Step 2: Run the targeted test and verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml active_product
```

Expected: FAIL because `"pi"` currently falls through to Codex.

- [ ] **Step 3: Add the explicit mapping**

Update:

```rust
match get_setting(&connection, "active_product")?.as_deref() {
    Some("claude") => AccountProduct::Claude,
    Some("antigravity") => AccountProduct::Antigravity,
    Some("grok") => AccountProduct::Grok,
    Some("pi") => AccountProduct::Pi,
    _ => AccountProduct::Codex,
}
```

For command paths that are not implemented until Phase 2, explicitly reject Pi with messages such as:

```rust
AccountProduct::Pi => Err("Pi 账号切换将在下一阶段启用。".to_string())
```

Do not route Pi to Codex implementations.

- [ ] **Step 4: Run the complete Rust validation set**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/features/accounts/commands.rs src-tauri/src
git commit -m "feat: wire Pi product through backend dispatch"
```

---

## Phase 1 Exit Criteria

Before starting Phase 2, verify all of the following:

- [ ] Fresh databases create `provider_key TEXT NOT NULL DEFAULT ''`.
- [ ] Existing databases migrate idempotently.
- [ ] `(product, provider_key, account_id)` uniqueness works as designed.
- [ ] Existing products still expose empty `providerKey`/`provider_key` values on the Rust side.
- [ ] `AccountProduct::Pi` serializes as `pi` and displays as `Pi`.
- [ ] `active_product = pi` round-trips correctly.
- [ ] Pi auth path defaults to `~/.pi/agent/auth.json`.
- [ ] `PI_CODING_AGENT_DIR` overrides the Pi agent directory when present in the Cortana process environment.
- [ ] Supported `openai-codex` OAuth credential shape validates; API-key shape is rejected.
- [ ] No Pi switch/import/write behavior exists yet.
- [ ] Full Rust fmt, clippy, and test commands pass.

## Phase 1 Handoff to Phase 2

Phase 2 may rely on these exact interfaces:

```rust
AccountProduct::Pi
ProfileSummary { provider_key: String, .. }
pub(crate) const PI_PROVIDER_OPENAI_CODEX: &str = "openai-codex";
pub(crate) fn auth_path(state: &AppState) -> PathBuf;
pub(crate) fn parse_openai_codex_credential(value: &Value) -> Result<OpenAiCodexCredential, String>;
pub(crate) struct OpenAiCodexCredential { ... }
```

Do not rename these during Phase 2 unless the design spec is updated in the same change.