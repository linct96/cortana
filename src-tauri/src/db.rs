use super::*;
use rusqlite::{Transaction, TransactionBehavior};

const LATEST_DATABASE_VERSION: i64 = 3;

pub(super) fn initialize_database(state: &AppState) -> Result<(), String> {
    let mut connection = open_database(state)?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL;")
        .map_err(database_error)?;
    let mut version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(database_error)?;
    if version > LATEST_DATABASE_VERSION {
        return Err("本地账户数据库由更高版本的 Cortana 创建，当前版本无法打开。".to_string());
    }
    if version < 1 {
        migrate_to_v1(&mut connection)?;
        connection
            .pragma_update(None, "user_version", 1)
            .map_err(database_error)?;
        version = 1;
    }
    if version < 2 {
        migrate_to_v2(&mut connection)?;
        version = 2;
    }
    if version < 3 {
        migrate_to_v3(&mut connection)?;
    }
    // 旧版本进程可能在迁移后再次写回令牌，新版本每次启动都清理。
    connection
        .execute("DELETE FROM settings WHERE key = 'web_access_token'", [])
        .map_err(database_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state.database_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn migrate_to_v1(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS settings (
              key TEXT PRIMARY KEY NOT NULL,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS model_pricing (
              model_id TEXT PRIMARY KEY NOT NULL,
              display_name TEXT NOT NULL,
              input_cost_per_million TEXT NOT NULL,
              output_cost_per_million TEXT NOT NULL,
              cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
              cache_write_cost_per_million TEXT NOT NULL DEFAULT '0'
            );
            ",
        )
        .map_err(database_error)?;
    let has_agents_profiles = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'agents_profiles')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    let has_instruction_profiles = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'instruction_profiles')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if has_agents_profiles && !has_instruction_profiles {
        connection
            .execute(
                "ALTER TABLE agents_profiles RENAME TO instruction_profiles",
                [],
            )
            .map_err(database_error)?;
    }
    migrate_instruction_profiles(&mut *connection)?;
    let has_profiles = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'profiles')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    let has_accounts = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'accounts')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if has_profiles && !has_accounts {
        connection
            .execute("ALTER TABLE profiles RENAME TO accounts", [])
            .map_err(database_error)?;
    }
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS instruction_profiles (
              id TEXT PRIMARY KEY NOT NULL,
              product TEXT NOT NULL,
              name TEXT NOT NULL COLLATE NOCASE,
              content TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              UNIQUE(product, name)
            );
            CREATE TABLE IF NOT EXISTS accounts (
              id TEXT PRIMARY KEY NOT NULL,
              product TEXT NOT NULL DEFAULT 'codex',
              account_type TEXT NOT NULL DEFAULT 'oauth',
              api_base_url TEXT,
              account_id TEXT NOT NULL DEFAULT '',
              chatgpt_user_id TEXT NOT NULL DEFAULT '',
              email TEXT NOT NULL DEFAULT '',
              alias TEXT NOT NULL,
              plan_type TEXT NOT NULL DEFAULT '',
              auth_json TEXT NOT NULL,
              usage_primary_percent REAL,
              usage_primary_window_minutes INTEGER,
              usage_primary_resets_at INTEGER,
              usage_secondary_percent REAL,
              usage_secondary_window_minutes INTEGER,
              usage_secondary_resets_at INTEGER,
              antigravity_quota_json TEXT,
              usage_updated_at INTEGER,
              usage_refresh_attempted_at INTEGER,
              oauth_invalidated_at INTEGER,
              reset_credits_available_count INTEGER,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              last_used_at INTEGER,
              sort_order INTEGER NOT NULL DEFAULT 0
            );
            DROP INDEX IF EXISTS profiles_account_id_idx;
            DROP INDEX IF EXISTS profiles_auth_hash_idx;
            DROP INDEX IF EXISTS accounts_auth_hash_idx;
            DROP INDEX IF EXISTS accounts_relay_idx;
            CREATE INDEX IF NOT EXISTS accounts_account_id_idx ON accounts(account_id);
            ",
        )
        .map_err(database_error)?;
    for (name, definition) in [
        ("product", "TEXT NOT NULL DEFAULT 'codex'"),
        ("account_type", "TEXT NOT NULL DEFAULT 'oauth'"),
        ("api_base_url", "TEXT"),
        ("chatgpt_user_id", "TEXT NOT NULL DEFAULT ''"),
        ("plan_type", "TEXT NOT NULL DEFAULT ''"),
        ("usage_primary_percent", "REAL"),
        ("usage_primary_window_minutes", "INTEGER"),
        ("usage_primary_resets_at", "INTEGER"),
        ("usage_secondary_percent", "REAL"),
        ("usage_secondary_window_minutes", "INTEGER"),
        ("usage_secondary_resets_at", "INTEGER"),
        ("antigravity_quota_json", "TEXT"),
        ("usage_updated_at", "INTEGER"),
        ("usage_refresh_attempted_at", "INTEGER"),
        ("oauth_invalidated_at", "INTEGER"),
        ("reset_credits_available_count", "INTEGER"),
    ] {
        ensure_account_column(connection, name, definition)?;
    }
    for name in ["credits_balance", "credits_unlimited", "auth_hash"] {
        if account_column_exists(connection, name)? {
            connection
                .execute(&format!("ALTER TABLE accounts DROP COLUMN {name}"), [])
                .map_err(database_error)?;
        }
    }
    let added_sort_order =
        ensure_account_column(connection, "sort_order", "INTEGER NOT NULL DEFAULT 0")?;
    if added_sort_order {
        let profile_ids = connection
            .prepare("SELECT id FROM accounts ORDER BY last_used_at DESC, created_at ASC")
            .map_err(database_error)?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        let transaction = connection.transaction().map_err(database_error)?;
        for (sort_order, profile_id) in profile_ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE accounts SET sort_order = ?1 WHERE id = ?2",
                    params![sort_order as i64, profile_id],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;
    }
    backfill_codex_user_ids(connection)?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS accounts_codex_identity_idx ON accounts(product, account_type, account_id, chatgpt_user_id)",
            [],
        )
        .map_err(database_error)?;
    connection
        .execute(
            "DELETE FROM settings WHERE key IN ('profile_order', 'active_profile_id', 'active_agents_profile_id')",
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

fn migrate_to_v2(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    backfill_codex_user_ids(&transaction)?;
    backfill_relay_fingerprints(&transaction)?;
    deduplicate_accounts(&transaction)?;
    transaction
        .execute_batch(
            "
            DROP INDEX IF EXISTS accounts_codex_identity_idx;
            CREATE UNIQUE INDEX IF NOT EXISTS accounts_codex_oauth_identity_uq
              ON accounts(account_id, chatgpt_user_id)
              WHERE product = 'codex' AND account_type = 'oauth'
                AND account_id <> '' AND chatgpt_user_id <> '';
            CREATE UNIQUE INDEX IF NOT EXISTS accounts_oauth_account_identity_uq
              ON accounts(product, account_id)
              WHERE product <> 'codex' AND account_type = 'oauth' AND account_id <> '';
            CREATE UNIQUE INDEX IF NOT EXISTS accounts_oauth_email_identity_uq
              ON accounts(product, email COLLATE NOCASE)
              WHERE product <> 'codex' AND account_type = 'oauth' AND email <> '';
            CREATE UNIQUE INDEX IF NOT EXISTS accounts_relay_identity_uq
              ON accounts(product, api_base_url, account_id)
              WHERE account_type = 'relay' AND api_base_url IS NOT NULL AND account_id <> '';
            PRAGMA user_version = 2;
            ",
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

fn migrate_to_v3(connection: &mut Connection) -> Result<(), String> {
    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute("DELETE FROM settings WHERE key = 'web_access_token'", [])
        .map_err(database_error)?;
    transaction
        .execute_batch("PRAGMA user_version = 3;")
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

fn backfill_relay_fingerprints(connection: &Connection) -> Result<(), String> {
    let rows = connection
        .prepare(
            "SELECT id, product, auth_json FROM accounts
             WHERE account_type = 'relay' AND account_id = ''",
        )
        .map_err(database_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    for (id, product, auth_json) in rows {
        let Ok(auth) = serde_json::from_str::<Value>(&auth_json) else {
            continue;
        };
        let key = match product.as_str() {
            "codex" => "OPENAI_API_KEY",
            "claude" => "authToken",
            _ => continue,
        };
        let Some(secret) = auth
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
        else {
            continue;
        };
        connection
            .execute(
                "UPDATE accounts SET account_id = ?1 WHERE id = ?2",
                params![credential_fingerprint(secret), id],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

#[derive(Clone)]
struct AccountIdentityRow {
    id: String,
    product: String,
    account_type: String,
    api_base_url: Option<String>,
    account_id: String,
    chatgpt_user_id: String,
    email: String,
    created_at: i64,
    updated_at: i64,
    last_used_at: Option<i64>,
    sort_order: i64,
}

fn deduplicate_accounts(transaction: &Transaction<'_>) -> Result<(), String> {
    let rows = transaction
        .prepare(
            "SELECT id, product, account_type, api_base_url, account_id, chatgpt_user_id,
                    email, created_at, updated_at, last_used_at, sort_order
             FROM accounts",
        )
        .map_err(database_error)?
        .query_map([], |row| {
            Ok(AccountIdentityRow {
                id: row.get(0)?,
                product: row.get(1)?,
                account_type: row.get(2)?,
                api_base_url: row.get(3)?,
                account_id: row.get(4)?,
                chatgpt_user_id: row.get(5)?,
                email: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
                last_used_at: row.get(9)?,
                sort_order: row.get(10)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let mut groups = (0..rows.len()).collect::<Vec<_>>();
    // ponytail: 账户规模很小；增长到上万条时再改为哈希分组。
    for left in 0..rows.len() {
        for right in (left + 1)..rows.len() {
            if same_account_identity(&rows[left], &rows[right]) {
                let from = groups[right];
                let to = groups[left];
                for group in &mut groups {
                    if *group == from {
                        *group = to;
                    }
                }
            }
        }
    }
    let mut merged = HashMap::<usize, Vec<&AccountIdentityRow>>::new();
    for (index, group) in groups.into_iter().enumerate() {
        merged.entry(group).or_default().push(&rows[index]);
    }
    for group in merged.values().filter(|group| group.len() > 1) {
        let survivor = group
            .iter()
            .max_by_key(|row| {
                (
                    row.updated_at,
                    row.last_used_at.unwrap_or(i64::MIN),
                    &row.id,
                )
            })
            .expect("duplicate group is not empty");
        let created_at = group
            .iter()
            .map(|row| row.created_at)
            .min()
            .unwrap_or(survivor.created_at);
        let last_used_at = group.iter().filter_map(|row| row.last_used_at).max();
        let sort_order = group
            .iter()
            .map(|row| row.sort_order)
            .min()
            .unwrap_or(survivor.sort_order);
        for duplicate in group.iter().filter(|row| row.id != survivor.id) {
            transaction
                .execute(
                    "UPDATE settings SET value = ?1
                     WHERE key = 'antigravity_active_profile_id' AND value = ?2",
                    params![survivor.id, duplicate.id],
                )
                .map_err(database_error)?;
            transaction
                .execute("DELETE FROM accounts WHERE id = ?1", params![duplicate.id])
                .map_err(database_error)?;
        }
        transaction
            .execute(
                "UPDATE accounts
                 SET created_at = ?1, last_used_at = ?2, sort_order = ?3
                 WHERE id = ?4",
                params![created_at, last_used_at, sort_order, survivor.id],
            )
            .map_err(database_error)?;
    }
    normalize_account_order(transaction)
}

fn same_account_identity(left: &AccountIdentityRow, right: &AccountIdentityRow) -> bool {
    if left.product != right.product || left.account_type != right.account_type {
        return false;
    }
    match left.account_type.as_str() {
        "oauth" if left.product == "codex" => {
            !left.account_id.is_empty()
                && !left.chatgpt_user_id.is_empty()
                && left.account_id == right.account_id
                && left.chatgpt_user_id == right.chatgpt_user_id
        }
        "oauth" => {
            (!left.account_id.is_empty() && left.account_id == right.account_id)
                || (!left.email.is_empty() && left.email.eq_ignore_ascii_case(&right.email))
        }
        "relay" => {
            !left.account_id.is_empty()
                && left.account_id == right.account_id
                && left.api_base_url == right.api_base_url
        }
        _ => false,
    }
}

fn normalize_account_order(connection: &Connection) -> Result<(), String> {
    let rows = connection
        .prepare(
            "SELECT id, product FROM accounts
             ORDER BY product, sort_order, created_at, id",
        )
        .map_err(database_error)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let mut product = String::new();
    let mut sort_order = 0_i64;
    for (id, next_product) in rows {
        if next_product != product {
            product = next_product;
            sort_order = 0;
        }
        connection
            .execute(
                "UPDATE accounts SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            )
            .map_err(database_error)?;
        sort_order += 1;
    }
    Ok(())
}

pub(super) fn credential_fingerprint(secret: &str) -> String {
    use std::fmt::Write as _;

    let mut fingerprint = String::from("token:");
    for byte in Sha256::digest(secret.trim().as_bytes()) {
        write!(&mut fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}

fn migrate_instruction_profiles(connection: &mut Connection) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'instruction_profiles')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if !exists || instruction_profile_column_exists(connection, "product")? {
        return Ok(());
    }

    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute_batch(
            "
            CREATE TABLE instruction_profiles_v2 (
              id TEXT PRIMARY KEY NOT NULL,
              product TEXT NOT NULL,
              name TEXT NOT NULL COLLATE NOCASE,
              content TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              UNIQUE(product, name)
            );
            INSERT INTO instruction_profiles_v2 (id, product, name, content, created_at, updated_at)
              SELECT id, 'codex', name, content, created_at, updated_at FROM instruction_profiles;
            DROP TABLE instruction_profiles;
            ALTER TABLE instruction_profiles_v2 RENAME TO instruction_profiles;
            ",
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

fn instruction_profile_column_exists(connection: &Connection, name: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('instruction_profiles') WHERE name = ?1)",
            params![name],
            |row| row.get(0),
        )
        .map_err(database_error)
}

fn backfill_codex_user_ids(connection: &Connection) -> Result<(), String> {
    let rows = connection
        .prepare(
            "SELECT id, auth_json FROM accounts WHERE product = 'codex' AND account_type = 'oauth' AND chatgpt_user_id = ''",
        )
        .map_err(database_error)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    for (id, auth_json) in rows {
        let Ok(auth) = serde_json::from_str(&auth_json) else {
            continue;
        };
        let user_id = oauth::chatgpt_user_id_from_auth_json(&auth);
        if !user_id.is_empty() {
            connection
                .execute(
                    "UPDATE accounts SET chatgpt_user_id = ?1 WHERE id = ?2",
                    params![user_id, id],
                )
                .map_err(database_error)?;
        }
    }
    Ok(())
}

pub(super) fn ensure_account_column(
    connection: &Connection,
    name: &str,
    definition: &str,
) -> Result<bool, String> {
    let exists = account_column_exists(connection, name)?;
    if !exists {
        connection
            .execute(
                &format!("ALTER TABLE accounts ADD COLUMN {name} {definition}"),
                [],
            )
            .map_err(database_error)?;
    }
    Ok(!exists)
}

fn account_column_exists(connection: &Connection, name: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('accounts') WHERE name = ?1)",
            params![name],
            |row| row.get(0),
        )
        .map_err(database_error)
}

pub(super) fn open_database(state: &AppState) -> Result<Connection, String> {
    let connection = Connection::open(&state.database_path).map_err(database_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(database_error)?;
    Ok(connection)
}

pub(super) fn database_error(error: rusqlite::Error) -> String {
    format!("本地账户数据库不可用：{error}")
}

pub(super) fn get_setting(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

pub(super) fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(database_error)?;
    Ok(())
}

pub(super) fn set_usage_refresh_settings(
    connection: &mut Connection,
    enabled: bool,
    active_interval_minutes: u64,
    inactive_interval_minutes: u64,
) -> Result<(), String> {
    let transaction = connection.transaction().map_err(database_error)?;
    for (key, value) in [
        ("usage_refresh_enabled", enabled.to_string()),
        (
            "usage_refresh_active_interval_minutes",
            active_interval_minutes.to_string(),
        ),
        (
            "usage_refresh_inactive_interval_minutes",
            inactive_interval_minutes.to_string(),
        ),
    ] {
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)
}

pub(super) fn set_web_access_settings(
    connection: &mut Connection,
    enabled: bool,
    port: u16,
) -> Result<(), String> {
    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES ('web_access_enabled', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![enabled.to_string()],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES ('web_access_port', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![port.to_string()],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn saves_web_access_settings_together() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
            )
            .unwrap();
        set_web_access_settings(&mut connection, true, 11456).unwrap();
        assert_eq!(
            get_setting(&connection, "web_access_enabled").unwrap(),
            Some("true".to_string())
        );
        assert_eq!(
            get_setting(&connection, "web_access_port").unwrap(),
            Some("11456".to_string())
        );
    }

    #[test]
    fn backfills_codex_user_id_from_saved_auth() {
        let directory =
            std::env::temp_dir().join(format!("cortana-db-user-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        let encode = |value: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let claims = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "account-1",
                "chatgpt_user_id": "user-1"
            }
        });
        let id_token = format!(
            "{}.{}.{}",
            encode(br#"{"alg":"none","typ":"JWT"}"#),
            encode(claims.to_string().as_bytes()),
            encode(b"signature")
        );
        let auth_json = json!({"tokens": {"id_token": id_token}}).to_string();
        open_database(&state)
            .unwrap()
            .execute_batch(
                "DROP INDEX accounts_codex_oauth_identity_uq;
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        open_database(&state)
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, product, account_type, account_id, alias, auth_json, created_at, updated_at) VALUES ('legacy', 'codex', 'oauth', 'account-1', 'Legacy', ?1, 1, 1)",
                params![auth_json],
            )
            .unwrap();

        initialize_database(&state).unwrap();

        assert_eq!(
            open_database(&state)
                .unwrap()
                .query_row(
                    "SELECT chatgpt_user_id FROM accounts WHERE id = 'legacy'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "user-1"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migrates_legacy_database() {
        let directory = std::env::temp_dir().join(format!("cortana-db-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        let connection = open_database(&state).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents_profiles (
                   id TEXT PRIMARY KEY NOT NULL,
                   name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                   content TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL
                 );
                 INSERT INTO agents_profiles VALUES ('default', 'Default', '# Rules', 1, 1);",
            )
            .unwrap();
        drop(connection);

        initialize_database(&state).unwrap();
        let connection = open_database(&state).unwrap();
        connection
            .execute_batch(
                "ALTER TABLE accounts ADD COLUMN credits_balance TEXT;
                 ALTER TABLE accounts ADD COLUMN credits_unlimited INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE accounts ADD COLUMN auth_hash TEXT;
                 CREATE INDEX accounts_auth_hash_idx ON accounts(auth_hash);",
            )
            .unwrap();
        set_setting(&connection, "profile_order", "[]").unwrap();
        set_setting(&connection, "active_profile_id", "legacy-account").unwrap();
        set_setting(&connection, "active_agents_profile_id", "legacy-prompt").unwrap();
        set_setting(&connection, "web_access_token", "legacy-secret").unwrap();
        connection
            .execute_batch("PRAGMA user_version = 0;")
            .unwrap();
        drop(connection);

        initialize_database(&state).unwrap();

        let connection = open_database(&state).unwrap();
        assert!(account_column_exists(&connection, "product").unwrap());
        assert!(account_column_exists(&connection, "chatgpt_user_id").unwrap());
        assert!(!account_column_exists(&connection, "credits_balance").unwrap());
        assert!(!account_column_exists(&connection, "credits_unlimited").unwrap());
        assert!(!account_column_exists(&connection, "auth_hash").unwrap());
        assert_eq!(get_setting(&connection, "profile_order").unwrap(), None);
        assert_eq!(get_setting(&connection, "active_profile_id").unwrap(), None);
        assert_eq!(
            get_setting(&connection, "active_agents_profile_id").unwrap(),
            None
        );
        assert_eq!(get_setting(&connection, "web_access_token").unwrap(), None);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            LATEST_DATABASE_VERSION
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT product || ':' || content FROM instruction_profiles WHERE id = 'default'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "codex:# Rules"
        );
        connection
            .execute(
                "INSERT INTO instruction_profiles (id, product, name, content, created_at, updated_at) VALUES ('claude-default', 'claude', 'Default', '# Claude', 1, 1)",
                [],
            )
            .unwrap();
        set_setting(&connection, "web_access_token", "rewritten-secret").unwrap();
        drop(connection);

        initialize_database(&state).unwrap();
        assert_eq!(
            get_setting(&open_database(&state).unwrap(), "web_access_token").unwrap(),
            None
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deduplicates_accounts_and_backfills_relay_fingerprints() {
        let directory =
            std::env::temp_dir().join(format!("cortana-db-identity-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        let mut connection = open_database(&state).unwrap();
        migrate_to_v1(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts
                   (id, product, account_type, account_id, email, alias, auth_json, created_at, updated_at, sort_order)
                 VALUES
                   ('old', 'grok', 'oauth', 'grok-user', 'USER@example.com', 'Old', '{}', 1, 1, 4),
                   ('new', 'grok', 'oauth', 'grok-user', 'user@example.com', 'New', '{}', 2, 3, 7);
                 INSERT INTO accounts
                   (id, product, account_type, api_base_url, alias, auth_json, created_at, updated_at, sort_order)
                 VALUES
                   ('relay', 'codex', 'relay', 'https://relay.example/v1', 'Relay',
                    '{\"OPENAI_API_KEY\":\"secret\"}', 1, 1, 0);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        drop(connection);

        initialize_database(&state).unwrap();

        let connection = open_database(&state).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT id || ':' || sort_order FROM accounts WHERE product = 'grok'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "new:0"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT account_id FROM accounts WHERE id = 'relay'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            credential_fingerprint("secret")
        );
        assert!(connection
            .execute(
                "INSERT INTO accounts
                   (id, product, account_type, account_id, email, alias, auth_json, created_at, updated_at)
                 VALUES ('duplicate', 'grok', 'oauth', 'grok-user', 'other@example.com', 'Duplicate', '{}', 4, 4)",
                [],
            )
            .is_err());
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
}
