# Pi Phase 2 Auth Synchronization and Switching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement reliable Pi `openai-codex` account detection, import, token back-sync, and account switching while preserving every other Pi Provider credential.

**Architecture:** Treat `~/.pi/agent/auth.json` as the runtime source of truth. All Pi read-modify-write operations use a Rust lock implementation compatible with Pi's `proper-lockfile@4.1.2` protocol (`<auth.json>.lock` directory, stale timeout, heartbeat). Pi account operations run inside one critical section that coordinates the auth file and SQLite transaction, synchronizes refreshed current credentials before replacement, and restores the file if the database commit fails.

**Tech Stack:** Rust 2021, Tauri 2 `spawn_blocking`, rusqlite transactions, serde_json, `filetime` for cross-platform lock-directory mtime updates, Node.js fixture using `proper-lockfile@4.1.2` for interoperability tests.

**Spec:** `docs/superpowers/specs/2026-09-05-pi-account-switching-design.md`

**Depends on:** `docs/superpowers/plans/2026-09-05-pi-phase-1-database-rust-adapter.md`

## Global Constraints

- Pi reference baseline is `da840b6216578c2a571d0374ac6a2091a83f9d91`.
- Pi currently depends on `proper-lockfile` version `4.1.2`.
- Pi calls `proper-lockfile` with `realpath: false` and a stale threshold of `30_000 ms` for auth storage.
- Pi's lock path is exactly `<absolute auth path>.lock`; the lock is an atomically-created directory.
- A Cortana Pi profile stores only the `openai-codex` credential object, never the entire `auth.json` map.
- Only `auth.json["openai-codex"]` may change during a switch.
- `access`, `refresh`, `expires`, and `accountId` are the supported Phase 1/2 OAuth fields.
- `accountId` is the stable identity; token strings are mutable state and must be back-synchronized.
- Cortana must never log or return full Pi access/refresh tokens to the frontend.
- Cortana must not actively refresh Pi OAuth tokens.
- Unsupported `openai-codex` credential formats are hard errors; they are not treated as missing and are not overwritten by force-switch.
- `force = true` only bypasses the supported-but-unmanaged account protection.
- Deleting a Cortana Pi profile must not remove `openai-codex` from Pi.
- Do not implement Pi frontend UI in this phase.

---

## File Structure

**Modify**

- `src-tauri/Cargo.toml` — add `filetime` for lock heartbeat mtimes.
- `src-tauri/Cargo.lock` — generated dependency lock update.
- `package.json` — add root test-only `proper-lockfile@4.1.2` dev dependency.
- `pnpm-lock.yaml` — generated dependency lock update.
- `src-tauri/src/products/pi/mod.rs` — export the full Pi account adapter.
- `src-tauri/src/products/pi/auth.rs` — add full auth-map read/merge/write helpers.
- `src-tauri/src/features/accounts/commands.rs` — dispatch Pi status/import/switch/update alias and reject unsupported operations.

**Create**

- `src-tauri/src/products/pi/lock.rs` — `proper-lockfile`-compatible lock guard.
- `src-tauri/src/products/pi/accounts.rs` — Pi app status, import, back-sync, switch transaction, alias update.
- `scripts/proper-lockfile-fixture.cjs` — Node interoperability fixture using the exact Pi lock dependency version.

**Reuse without changing semantics**

- `src-tauri/src/features/accounts/store.rs` — generic profile summary/list APIs from Phase 1.
- `src-tauri/src/platform/db.rs` — database open/error helpers and provider-aware schema from Phase 1.

---

### Task 1: Add an exact `proper-lockfile@4.1.2` interoperability fixture

**Files:**
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Create: `scripts/proper-lockfile-fixture.cjs`

**Interfaces:**
- Produces: a Node fixture that can hold a Pi-compatible lock or attempt a non-blocking lock.
- Consumed by: Rust lock interoperability tests in Task 2.

- [ ] **Step 1: Add the exact development dependency**

Run:

```bash
pnpm add -Dw proper-lockfile@4.1.2
```

Verify `package.json` contains:

```json
"proper-lockfile": "4.1.2"
```

Do not use a caret or tilde range because the test is intended to pin Pi's current protocol implementation.

- [ ] **Step 2: Create the Node fixture**

Create `scripts/proper-lockfile-fixture.cjs`:

```js
const lockfile = require('proper-lockfile');

const [mode, path, holdMsRaw] = process.argv.slice(2);
const holdMs = Number(holdMsRaw || 0);

async function main() {
  if (!mode || !path) throw new Error('usage: fixture <hold|try> <path> [holdMs]');

  if (mode === 'hold') {
    const release = await lockfile.lock(path, {
      realpath: false,
      stale: 30_000,
      retries: 0,
    });
    process.stdout.write('LOCKED\n');
    await new Promise((resolve) => setTimeout(resolve, holdMs));
    await release();
    process.stdout.write('RELEASED\n');
    return;
  }

  if (mode === 'try') {
    try {
      const release = await lockfile.lock(path, {
        realpath: false,
        stale: 30_000,
        retries: 0,
      });
      await release();
      process.stdout.write('ACQUIRED\n');
    } catch (error) {
      if (error && error.code === 'ELOCKED') {
        process.stdout.write('ELOCKED\n');
        process.exitCode = 42;
        return;
      }
      throw error;
    }
    return;
  }

  throw new Error(`unknown mode: ${mode}`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
```

- [ ] **Step 3: Verify the fixture uses the pinned dependency**

Run:

```bash
node -e "console.log(require('proper-lockfile/package.json').version)"
```

Expected output:

```text
4.1.2
```

- [ ] **Step 4: Commit**

```bash
git add package.json pnpm-lock.yaml scripts/proper-lockfile-fixture.cjs
git commit -m "test: pin Pi lock interoperability fixture"
```

---

### Task 2: Implement a `proper-lockfile`-compatible Rust lock guard

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Create: `src-tauri/src/products/pi/lock.rs`
- Modify: `src-tauri/src/products/pi/mod.rs`

**Interfaces:**
- Produces: `PiAuthLock::acquire(path: &Path) -> Result<PiAuthLock, String>`.
- Produces for tests: `PiAuthLock::acquire_with_policy(path, LockPolicy)`.
- Produces: `PiAuthLock::ensure_healthy(&self) -> Result<(), String>`.
- Lock ownership protocol is compatible with `proper-lockfile@4.1.2`.

- [ ] **Step 1: Add `filetime`**

Add to `[dependencies]`:

```toml
filetime = "0.2"
```

Then run:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 2: Write failing Rust lock tests**

Define this policy type in `lock.rs`:

```rust
#[derive(Debug, Clone, Copy)]
struct LockPolicy {
    stale: Duration,
    heartbeat: Duration,
    timeout: Duration,
}
```

Production defaults:

```rust
const DEFAULT_POLICY: LockPolicy = LockPolicy {
    stale: Duration::from_secs(30),
    heartbeat: Duration::from_secs(15),
    timeout: Duration::from_secs(30),
};
```

Add unit tests for:

1. First caller creates `<auth>.lock` as a directory.
2. Second Rust caller cannot acquire a fresh lock.
3. A stale lock directory older than 30 seconds is removed and reacquired.
4. Dropping the guard removes only its own lock directory.
5. If the lock directory mtime changes unexpectedly, `ensure_healthy()` returns a compromised-lock error and Drop does not remove a potentially foreign lock.

Use short test-only timeouts such as 50–250 ms; never make unit tests wait 30 seconds.

- [ ] **Step 3: Run tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::lock::tests
```

Expected: FAIL because the guard is not implemented.

- [ ] **Step 4: Implement atomic lock acquisition**

Use the exact lock path rule:

```rust
fn lock_path(auth_path: &Path) -> PathBuf {
    let mut value = auth_path.as_os_str().to_os_string();
    value.push(".lock");
    PathBuf::from(value)
}
```

Acquisition loop:

```text
create_dir(<auth>.lock)
  success -> own lock
  AlreadyExists -> stat mtime
    stale -> remove_dir and retry
    fresh -> backoff/retry until timeout
  other error -> fail
```

Use exponential delay with jitter bounded to a small maximum; keep the exact delay policy internal because interoperability depends on lock ownership semantics, not a byte-for-byte retry schedule.

After creating the directory, read its metadata mtime and store it as `owned_mtime`.

- [ ] **Step 5: Add heartbeat ownership tracking**

Start a background thread that waits on a stop channel with `recv_timeout(heartbeat)`.

On every timeout:

1. Stat the lock directory.
2. Compare current mtime to the guard's last owned mtime.
3. If it differs unexpectedly, mark the lock compromised and stop heartbeating.
4. Otherwise call `filetime::set_file_mtime(lock_path, FileTime::now())`.
5. Stat again and store the actual resulting mtime.

On Drop:

1. Signal the heartbeat thread to stop.
2. Join it.
3. Re-check ownership.
4. Remove the lock directory only if ownership is still valid.

Never recursively delete the lock path; `proper-lockfile` creates an empty directory and uses `rmdir` semantics.

- [ ] **Step 6: Add Node -> Rust interoperability test**

In a test, create an empty auth file and spawn:

```text
node scripts/proper-lockfile-fixture.cjs hold <auth-path> 1500
```

Wait until stdout contains `LOCKED`, then call Rust acquisition with a 200 ms timeout.

Assert Rust returns the user-facing busy error:

```text
Pi 正在更新认证信息，暂时无法切换账号，请稍后重试。
```

- [ ] **Step 7: Add Rust -> Node interoperability test**

Acquire `PiAuthLock`, then run:

```text
node scripts/proper-lockfile-fixture.cjs try <auth-path>
```

Assert:

```text
exit code = 42
stdout contains ELOCKED
```

Drop the Rust guard, run the Node `try` command again, and assert it prints `ACQUIRED`.

- [ ] **Step 8: Run all lock tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::lock
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/products/pi/lock.rs src-tauri/src/products/pi/mod.rs
git commit -m "feat: add Pi compatible auth file lock"
```

---

### Task 3: Add full Pi auth-map read, merge, and safe-write helpers

**Files:**
- Modify: `src-tauri/src/products/pi/auth.rs`

**Interfaces:**
- Produces: `read_auth_storage(path: &Path) -> Result<AuthStorage, String>`.
- Produces: `openai_codex_credential(&AuthStorage) -> Result<Option<OpenAiCodexCredential>, String>`.
- Produces: `merge_openai_codex(storage, credential) -> AuthStorage`.
- Produces: `write_auth_storage(path, storage) -> Result<(), String>`.
- Produces: `restore_auth_contents(path, original: Option<&[u8]>) -> Result<(), String>`.

- [ ] **Step 1: Write failing merge and corruption tests**

Define:

```rust
type AuthStorage = serde_json::Map<String, serde_json::Value>;
```

Tests must prove:

```rust
let original = serde_json::json!({
    "anthropic": { "type": "oauth", "access": "a" },
    "openai-codex": {
        "type": "oauth",
        "access": "old",
        "refresh": "old-r",
        "expires": 1,
        "accountId": "acct-a"
    },
    "github-copilot": { "type": "oauth", "token": "c" }
});
```

After merging B:

- `anthropic` is exactly equal to its original JSON value.
- `github-copilot` is exactly equal to its original JSON value.
- only `openai-codex` changes.

Also test:

- missing auth file returns an empty map only when the caller explicitly requests creation/write behavior; status reads distinguish missing from `{}`.
- invalid top-level JSON returns the exact protective error and does not write.
- top-level arrays are rejected.
- unsupported `openai-codex` type returns unsupported credential error.

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::auth::tests
```

Expected: FAIL until the helpers are implemented.

- [ ] **Step 3: Implement full-map parsing without normalizing other providers**

Parse top-level JSON into `serde_json::Value`, require `Value::Object`, then retain the map as-is.

Do not deserialize the entire file into a typed Rust struct. Only deserialize the `openai-codex` value into `OpenAiCodexCredential` so unknown Provider fields survive untouched.

- [ ] **Step 4: Implement Provider-only merge**

Use:

```rust
pub(crate) fn merge_openai_codex(
    mut storage: AuthStorage,
    credential: &OpenAiCodexCredential,
) -> Result<AuthStorage, String> {
    storage.insert(
        PI_PROVIDER_OPENAI_CODEX.to_string(),
        serde_json::to_value(credential).map_err(|error| error.to_string())?,
    );
    Ok(storage)
}
```

Do not rebuild unrelated Provider entries.

- [ ] **Step 5: Implement protected writes and restoration**

The caller always holds `PiAuthLock` while writing.

Required behavior:

- Ensure parent directory exists; Unix creation mode is `0700`.
- If creating `auth.json`, create it with Unix mode `0600`.
- If it already exists, do not widen its permissions.
- Serialize with `serde_json::to_vec_pretty` and trailing newline.
- Keep the original bytes in the transaction caller before writing.
- On Unix, write to a same-directory unique temporary file, copy the destination permissions, `sync_all`, then `rename` over the target.
- On Windows, where replacement rename semantics differ, use a lock-protected truncate/write/sync fallback and rely on the original-byte rollback path for transaction failure.
- `restore_auth_contents` restores the exact original bytes if the file existed; if the file did not exist before the operation, remove the newly-created file.

- [ ] **Step 6: Add write/restore tests**

Cover:

- new file creation.
- existing file with unrelated Providers.
- restored file bytes exactly equal the pre-switch bytes.
- invalid JSON never gets replaced with `{}`.
- Unix-only permission assertion: new file mode masked with `0o777` equals `0o600`.

- [ ] **Step 7: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::auth::tests
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/products/pi/auth.rs
git commit -m "feat: add Pi provider credential file operations"
```

---

### Task 4: Implement Pi managed/unmanaged/missing status and token back-sync

**Files:**
- Create: `src-tauri/src/products/pi/accounts.rs`
- Modify: `src-tauri/src/products/pi/mod.rs`

**Interfaces:**
- Produces: `app_status(app: &tauri::AppHandle, state: &AppState) -> Result<AppStatus, String>`.
- Produces internally: `find_profile_by_account_id(...)`.
- Produces internally: `sync_managed_credential(...)`.
- Phase 3 consumes `AppStatus` through the existing `get_app_status` Tauri command.

- [ ] **Step 1: Write failing status tests**

Use temporary `AppState` paths and a temporary Pi agent directory. Cover these exact states:

1. No auth file -> `authState.kind == "missing"`, no active profile.
2. Auth file exists without `openai-codex` -> missing.
3. Supported credential with unknown `accountId` -> unmanaged + `detected_profile`.
4. Supported credential with matching DB profile -> managed + that profile `is_active == true`.
5. Matching DB profile has old token JSON but disk has same accountId/new tokens -> DB `auth_json` is updated to the disk credential before status returns.
6. Broken top-level JSON -> error, not missing.
7. Unsupported `openai-codex` credential -> explicit unsupported error.

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::status
```

Expected: FAIL because `accounts.rs` does not exist.

- [ ] **Step 3: Implement the Pi profile lookup**

Use this identity query:

```sql
SELECT id
FROM accounts
WHERE product = 'pi'
  AND provider_key = 'openai-codex'
  AND account_type = 'oauth'
  AND account_id = ?1
LIMIT 1
```

Never compare access or refresh tokens for identity.

- [ ] **Step 4: Implement temporary detected profile construction**

The temporary profile must contain:

```text
id = detected
product = Pi
provider_key = openai-codex
account_type = oauth
account_id = credential.accountId
email = ''
plan_type = ''
usage fields = None
is_active = true
alias = OpenAI Codex · <last 8 chars of accountId, or full id if shorter>
```

Do not derive email/plan from undocumented JWT claims.

- [ ] **Step 5: Implement managed credential back-sync**

While holding `PiAuthLock`, begin an `IMMEDIATE` SQLite transaction and update only the managed profile's `auth_json` and `updated_at` when the stored credential differs from the disk credential.

Use semantic JSON equality by serializing the validated credential into a `Value`; do not compare pretty-print whitespace.

Commit before returning status.

- [ ] **Step 6: Implement `app_status`**

Algorithm:

```text
resolve auth path
if file missing -> missing status without creating file
acquire PiAuthLock
read/parse auth map
if openai-codex missing -> missing
validate credential
open DB transaction
find accountId
  found -> back-sync token JSON, managed
  not found -> unmanaged + detected profile
commit if mutated
list profiles with active id
release lock
return AppStatus
```

Use existing autostart and local-web helpers so `AppStatus` remains structurally identical to other products.

- [ ] **Step 7: Run status tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::status
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/products/pi/accounts.rs src-tauri/src/products/pi/mod.rs
git commit -m "feat: detect and sync Pi account state"
```

---

### Task 5: Implement `import_current_profile` with Provider-scoped upsert

**Files:**
- Modify: `src-tauri/src/products/pi/accounts.rs`

**Interfaces:**
- Produces: `import_current_profile(state: &AppState, alias: Option<String>) -> Result<ProfileSummary, String>`.
- Identity key: `(product=pi, provider_key=openai-codex, account_id)`.

- [ ] **Step 1: Write failing import tests**

Cover:

1. New account creates one Pi OAuth profile.
2. Importing the same `accountId` twice updates `auth_json` instead of creating a second row.
3. Re-import preserves an existing user alias.
4. An explicit non-empty alias is used for a newly-created row.
5. Missing `openai-codex` returns a clear error.
6. Unsupported credential returns an error.

- [ ] **Step 2: Run import tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::import
```

Expected: FAIL.

- [ ] **Step 3: Implement the import transaction**

Under `PiAuthLock`:

```text
read latest auth map
validate openai-codex
begin IMMEDIATE DB transaction
lookup (pi, openai-codex, accountId)
  existing -> update auth_json + updated_at only
  new -> insert full row
commit
return summary with current account active
```

New-row values must be:

```text
product = pi
provider_key = openai-codex
account_type = oauth
api_base_url = NULL
account_id = credential.accountId
chatgpt_user_id = ''
email = ''
plan_type = ''
auth_json = only the credential object
usage_* = NULL
antigravity_quota_json = NULL
oauth_invalidated_at = NULL
reset_credits_available_count = NULL
model_profile_id = NULL
default_model_id = NULL
```

Use the existing defaults for gateway-only columns.

- [ ] **Step 4: Run import tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::import
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/products/pi/accounts.rs
git commit -m "feat: import current Pi OpenAI Codex account"
```

---

### Task 6: Implement the Pi switch transaction with external-change protection and rollback

**Files:**
- Modify: `src-tauri/src/products/pi/accounts.rs`

**Interfaces:**
- Produces: `switch_profile(state: &AppState, profile_id: &str, force: bool) -> Result<ProfileSummary, String>`.
- Consumes: lock/auth helpers from Tasks 2–3.
- Guarantees: managed current token back-sync occurs before replacement.

- [ ] **Step 1: Write failing switch tests**

Add tests for all of these cases:

```text
managed A -> B                       succeeds
missing openai-codex -> B            succeeds
unmanaged D -> B, force=false        blocked; file unchanged
unmanaged D -> B, force=true         succeeds
unsupported current credential -> B  blocked even with force=true
managed A disk token refreshed -> B  A database auth_json updated first
B switch preserves other Providers   exact JSON values unchanged
wrong product target                  rejected
wrong provider_key target             rejected
relay target                          rejected
```

- [ ] **Step 2: Run switch tests and verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::switch
```

Expected: FAIL.

- [ ] **Step 3: Implement current-state resolution inside the lock**

Inside a single `PiAuthLock` critical section:

1. Capture `original_bytes: Option<Vec<u8>>`.
2. Parse full storage.
3. Validate current `openai-codex` if present.
4. Start an `IMMEDIATE` DB transaction.
5. If current credential matches a managed profile, update that row's credential JSON first.
6. If it is supported but unmanaged and `force == false`, return:

```text
检测到工具外的 Pi OpenAI Codex 登录变化。请先同步当前账号，或确认后强制切换。
```

7. If current credential is unsupported, return the unsupported-format error regardless of `force`.

- [ ] **Step 4: Validate the target profile before writing**

Query the target with all constraints in SQL:

```sql
SELECT auth_json
FROM accounts
WHERE id = ?1
  AND product = 'pi'
  AND provider_key = 'openai-codex'
  AND account_type = 'oauth'
```

Parse the stored credential and require the stored `accountId` to match the row `account_id`.

- [ ] **Step 5: Merge and write only `openai-codex`**

Generate a merged full auth map from the disk map plus target credential. Call `lock.ensure_healthy()` immediately before write and immediately after write.

Do not modify any other Provider key.

- [ ] **Step 6: Update database activity and commit**

Within the same transaction:

```sql
UPDATE accounts
SET last_used_at = ?1, updated_at = ?1
WHERE id = ?2 AND product = 'pi';
```

Then commit.

If commit fails after the file write:

1. Keep the Pi lock held.
2. Restore `original_bytes` exactly.
3. If restore succeeds, return the DB error.
4. If restore fails, return:

```text
Pi 账号切换失败，且认证文件自动恢复失败。请立即检查 <authPath>。
```

Do not claim a successful switch in either failure path.

- [ ] **Step 7: Add an injected commit-failure test**

Refactor the transaction's finalization behind a test-only seam or helper so a unit test can force the post-file-write DB commit path to fail.

The test must assert the auth file bytes exactly equal `original_bytes` after the function returns an error.

Do not simulate this by failing before the file write; the regression being protected is the half-switched state.

- [ ] **Step 8: Run switch tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests::switch
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/products/pi/accounts.rs
git commit -m "feat: switch Pi accounts transactionally"
```

---

### Task 7: Implement alias-only updates and confirm delete semantics

**Files:**
- Modify: `src-tauri/src/products/pi/accounts.rs`
- Modify: `src-tauri/src/features/accounts/commands.rs`

**Interfaces:**
- Produces: `update_alias(state, profile_id, alias) -> Result<ProfileSummary, String>`.
- Existing generic `delete_profile` remains file-neutral for Pi.

- [ ] **Step 1: Write failing alias/delete tests**

Verify:

- Pi alias update changes only `alias` and `updated_at`.
- Alias update rejects non-Pi or wrong-provider rows.
- Deleting an active Pi profile deletes the DB row but leaves `auth.json` byte-for-byte unchanged.
- Calling `app_status` after deleting the active profile returns unmanaged with a detected profile.

- [ ] **Step 2: Run tests and verify they fail where expected**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests
```

- [ ] **Step 3: Implement alias-only update**

Use:

```sql
UPDATE accounts
SET alias = ?1, updated_at = ?2
WHERE id = ?3
  AND product = 'pi'
  AND provider_key = 'openai-codex'
  AND account_type = 'oauth'
```

Reject an empty/whitespace-only alias with the same validation style used by other products.

Do not accept `auth_json` edits for Pi through the generic profile edit command.

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml products::pi::accounts::tests
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/products/pi/accounts.rs src-tauri/src/features/accounts/commands.rs
git commit -m "feat: support Pi account aliases"
```

---

### Task 8: Wire Pi into the existing Tauri account commands

**Files:**
- Modify: `src-tauri/src/features/accounts/commands.rs`
- Modify: `src-tauri/src/products/pi/mod.rs`

**Interfaces:**
- `get_app_status(Pi) -> pi::app_status`.
- `switch_profile(Pi) -> pi::switch_profile`.
- `import_current_profile(Pi) -> pi::import_current_profile`.
- `update_profile(Pi) -> pi::update_alias`.
- Unsupported Pi operations return explicit errors.

- [ ] **Step 1: Add command-dispatch tests where practical**

At minimum, add tests or compile-time exhaustive matches proving Pi takes its own branch for status/import/switch/update.

The required production matches are:

```rust
AccountProduct::Pi => pi::app_status(&app, &state),
```

```rust
AccountProduct::Pi => pi::switch_profile(&state, &profile_id, force)?,
```

```rust
AccountProduct::Pi => pi::import_current_profile(&state, alias)?,
```

```rust
AccountProduct::Pi => pi::update_alias(&state, &profile_id, &alias)?,
```

- [ ] **Step 2: Explicitly reject unsupported Pi commands**

Required errors:

```text
add_relay_profile(Pi)     -> "Pi 一期不支持中转站账户。"
update_relay_profile(Pi)  -> "Pi 一期不支持中转站账户。"
refresh_profile_usage(Pi) -> "Pi 一期不支持额度刷新。"
get_profile_auth(Pi)      -> "Pi 一期不提供原始凭据编辑。"
get_relay_api_key(Pi)     -> "Pi 一期不支持中转站账户。"
```

Do not let Pi fall through to generic Codex behavior.

- [ ] **Step 3: Run complete Rust tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/features/accounts/commands.rs src-tauri/src/products/pi
git commit -m "feat: expose Pi account operations"
```

---

### Task 9: Run the full cross-platform validation set

**Files:**
- No source changes unless validation reveals a defect.

- [ ] **Step 1: Frontend dependency/install validation**

```bash
pnpm install --frozen-lockfile
```

Expected: exit 0.

- [ ] **Step 2: Rust formatting**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Expected: exit 0.

- [ ] **Step 3: Rust lint**

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: exit 0.

- [ ] **Step 4: Rust tests including Node lock interoperability**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all tests pass, including both Node->Rust and Rust->Node lock ownership checks.

- [ ] **Step 5: Confirm no secret-bearing test output**

Search test/log output and source assertions for literal production-like access/refresh tokens. Test credentials must be obvious fixtures such as `access-a`/`refresh-a` only.

- [ ] **Step 6: Commit only if validation required fixes**

```bash
git add -A
git commit -m "fix: stabilize Pi auth synchronization"
```

Skip this commit if validation produced no source changes.

---

## Phase 2 Exit Criteria

- [ ] Rust and Node mutually recognize the same `<auth.json>.lock` ownership.
- [ ] Stale locks follow the 30-second Pi compatibility threshold.
- [ ] A lock guard heartbeats while held and does not remove a compromised/foreign lock.
- [ ] Invalid top-level `auth.json` is never overwritten.
- [ ] Unsupported `openai-codex` credentials are never force-overwritten.
- [ ] Status correctly distinguishes managed, unmanaged, and missing.
- [ ] Managed disk credential updates are back-synchronized to SQLite.
- [ ] Import deduplicates by `(pi, openai-codex, accountId)`.
- [ ] A -> B -> A switching uses the latest stored credential for A.
- [ ] Switching changes only `auth.json["openai-codex"]`.
- [ ] Other Provider JSON values remain unchanged.
- [ ] Supported unmanaged external login blocks a normal switch.
- [ ] Force-switch only bypasses that unmanaged protection.
- [ ] DB commit failure after file write restores the original auth bytes.
- [ ] Deleting the current Cortana Pi profile leaves Pi logged in and makes status unmanaged.
- [ ] Pi raw credential editing, relay, models, usage refresh, and OAuth initiation remain unavailable.
- [ ] Full `cargo fmt`, `cargo clippy`, and `cargo test` validation is green before Phase 3 starts.

## Phase 2 Handoff to Phase 3

Phase 3 may rely on these backend behaviors without adding new Pi authentication logic:

```text
get_app_status(product=pi)
import_current_profile(product=pi)
switch_profile(product=pi, profileId, force)
update_profile(product=pi, alias, authJson=null)
delete_profile(product=pi)
reorder_profiles(product=pi)
```

All Pi-specific credential manipulation must remain inside `src-tauri/src/products/pi/**`; React code must never parse, merge, or store access/refresh tokens.