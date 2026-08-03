use super::state::AppState;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{fs, time::Duration};

pub(crate) fn initialize_database(state: &AppState) -> Result<(), String> {
    let connection = open_database(state)?;
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;

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
              product TEXT NOT NULL,
              account_type TEXT NOT NULL,
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
              model_profile_id TEXT,
              default_model_id TEXT,
              upstream_protocol TEXT NOT NULL DEFAULT 'openaiResponses',
              upstream_auth_mode TEXT NOT NULL DEFAULT 'bearer',
              anthropic_max_tokens INTEGER NOT NULL DEFAULT 16384,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              last_used_at INTEGER,
              sort_order INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS model_profiles (
              id TEXT PRIMARY KEY NOT NULL,
              product TEXT NOT NULL,
              name TEXT NOT NULL COLLATE NOCASE,
              models_json TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              UNIQUE(product, name)
            );

            CREATE INDEX IF NOT EXISTS accounts_account_id_idx ON accounts(account_id);
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
            ",
        )
        .map_err(database_error)?;
    migrate_account_gateway_schema(&connection)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state.database_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn migrate_account_gateway_schema(connection: &Connection) -> Result<(), String> {
    for (name, definition) in [
        (
            "upstream_protocol",
            "TEXT NOT NULL DEFAULT 'openaiResponses'",
        ),
        ("upstream_auth_mode", "TEXT NOT NULL DEFAULT 'bearer'"),
        ("anthropic_max_tokens", "INTEGER NOT NULL DEFAULT 16384"),
    ] {
        if !account_column_exists(connection, name)? {
            connection
                .execute_batch(&format!(
                    "ALTER TABLE accounts ADD COLUMN {name} {definition}"
                ))
                .map_err(database_error)?;
        }
    }
    connection
        .execute_batch(
            "
            DROP INDEX IF EXISTS accounts_relay_identity_uq;
            CREATE UNIQUE INDEX accounts_relay_identity_uq
              ON accounts(product, api_base_url, account_id, upstream_protocol, upstream_auth_mode)
              WHERE account_type = 'relay' AND api_base_url IS NOT NULL AND account_id <> '';
            ",
        )
        .map_err(database_error)
}

fn account_column_exists(connection: &Connection, name: &str) -> Result<bool, String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(accounts)")
        .map_err(database_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(database_error)?;
    for column in columns {
        if column.map_err(database_error)? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn credential_fingerprint(secret: &str) -> String {
    use std::fmt::Write as _;

    let mut fingerprint = String::from("token:");
    for byte in Sha256::digest(secret.trim().as_bytes()) {
        write!(&mut fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}

pub(crate) fn open_database(state: &AppState) -> Result<Connection, String> {
    let connection = Connection::open(&state.database_path).map_err(database_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(database_error)?;
    Ok(connection)
}

pub(crate) fn database_error(error: rusqlite::Error) -> String {
    format!("本地账户数据库不可用：{error}")
}

pub(crate) fn get_setting(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

pub(crate) fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(database_error)?;
    Ok(())
}

pub(crate) fn set_usage_refresh_settings(
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

pub(crate) fn set_web_access_settings(
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
    use super::{
        account_column_exists, get_setting, migrate_account_gateway_schema, set_web_access_settings,
    };
    use rusqlite::Connection;

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
    fn migrates_gateway_columns_and_rebuilds_relay_identity_idempotently() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "
                CREATE TABLE accounts (
                  id TEXT PRIMARY KEY NOT NULL,
                  product TEXT NOT NULL,
                  account_type TEXT NOT NULL,
                  api_base_url TEXT,
                  account_id TEXT NOT NULL DEFAULT ''
                );
                CREATE UNIQUE INDEX accounts_relay_identity_uq
                  ON accounts(product, api_base_url, account_id)
                  WHERE account_type = 'relay' AND api_base_url IS NOT NULL AND account_id <> '';
                INSERT INTO accounts (id, product, account_type, api_base_url, account_id)
                  VALUES ('old', 'codex', 'relay', 'https://example.com/v1', 'token:1');
                ",
            )
            .unwrap();

        migrate_account_gateway_schema(&connection).unwrap();
        migrate_account_gateway_schema(&connection).unwrap();

        assert!(account_column_exists(&connection, "upstream_protocol").unwrap());
        assert!(account_column_exists(&connection, "upstream_auth_mode").unwrap());
        assert!(account_column_exists(&connection, "anthropic_max_tokens").unwrap());
        let defaults: (String, String, i64) = connection
            .query_row(
                "SELECT upstream_protocol, upstream_auth_mode, anthropic_max_tokens FROM accounts WHERE id = 'old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            defaults,
            ("openaiResponses".into(), "bearer".into(), 16_384)
        );

        connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, upstream_protocol, upstream_auth_mode) VALUES ('chat', 'codex', 'relay', 'https://example.com/v1', 'token:1', 'openaiChatCompletions', 'bearer')",
                [],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO accounts (id, product, account_type, api_base_url, account_id, upstream_protocol, upstream_auth_mode) VALUES ('duplicate', 'codex', 'relay', 'https://example.com/v1', 'token:1', 'openaiChatCompletions', 'bearer')",
                [],
            )
            .is_err());
    }
}
