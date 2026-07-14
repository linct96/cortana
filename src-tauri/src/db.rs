use super::*;

pub(super) fn initialize_database(state: &AppState) -> Result<(), String> {
    let mut connection = open_database(state)?;
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
            CREATE TABLE IF NOT EXISTS agents_profiles (
              id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL COLLATE NOCASE UNIQUE,
              content TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            );
            ",
        )
        .map_err(database_error)?;
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
            CREATE TABLE IF NOT EXISTS accounts (
              id TEXT PRIMARY KEY NOT NULL,
              account_type TEXT NOT NULL DEFAULT 'oauth',
              api_base_url TEXT,
              account_id TEXT NOT NULL DEFAULT '',
              email TEXT NOT NULL DEFAULT '',
              alias TEXT NOT NULL,
              plan_type TEXT NOT NULL DEFAULT '',
              auth_json TEXT NOT NULL,
              auth_hash TEXT NOT NULL,
              usage_primary_percent REAL,
              usage_primary_window_minutes INTEGER,
              usage_primary_resets_at INTEGER,
              usage_secondary_percent REAL,
              usage_secondary_window_minutes INTEGER,
              usage_secondary_resets_at INTEGER,
              credits_balance TEXT,
              credits_unlimited INTEGER NOT NULL DEFAULT 0,
              usage_updated_at INTEGER,
              reset_credits_available_count INTEGER,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              last_used_at INTEGER,
              sort_order INTEGER NOT NULL DEFAULT 0
            );
            DROP INDEX IF EXISTS profiles_account_id_idx;
            DROP INDEX IF EXISTS profiles_auth_hash_idx;
            CREATE INDEX IF NOT EXISTS accounts_account_id_idx ON accounts(account_id);
            CREATE INDEX IF NOT EXISTS accounts_auth_hash_idx ON accounts(auth_hash);
            ",
        )
        .map_err(database_error)?;
    for (name, definition) in [
        ("account_type", "TEXT NOT NULL DEFAULT 'oauth'"),
        ("api_base_url", "TEXT"),
        ("plan_type", "TEXT NOT NULL DEFAULT ''"),
        ("usage_primary_percent", "REAL"),
        ("usage_primary_window_minutes", "INTEGER"),
        ("usage_primary_resets_at", "INTEGER"),
        ("usage_secondary_percent", "REAL"),
        ("usage_secondary_window_minutes", "INTEGER"),
        ("usage_secondary_resets_at", "INTEGER"),
        ("credits_balance", "TEXT"),
        ("credits_unlimited", "INTEGER NOT NULL DEFAULT 0"),
        ("usage_updated_at", "INTEGER"),
        ("reset_credits_available_count", "INTEGER"),
    ] {
        ensure_account_column(&connection, name, definition)?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS accounts_relay_idx ON accounts(account_type, api_base_url, auth_hash)",
            [],
        )
        .map_err(database_error)?;
    let added_sort_order =
        ensure_account_column(&connection, "sort_order", "INTEGER NOT NULL DEFAULT 0")?;
    if added_sort_order {
        let mut profile_ids = connection
            .prepare("SELECT id FROM accounts ORDER BY last_used_at DESC, created_at ASC")
            .map_err(database_error)?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        if let Some(order) = get_setting(&connection, "profile_order")?
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        {
            let positions = order
                .into_iter()
                .enumerate()
                .map(|(index, id)| (id, index))
                .collect::<HashMap<_, _>>();
            profile_ids.sort_by_key(|id| positions.get(id).copied().unwrap_or(usize::MAX));
        }
        let transaction = connection.transaction().map_err(database_error)?;
        for (sort_order, profile_id) in profile_ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE accounts SET sort_order = ?1 WHERE id = ?2",
                    params![sort_order, profile_id],
                )
                .map_err(database_error)?;
        }
        transaction
            .execute("DELETE FROM settings WHERE key = 'profile_order'", [])
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state.database_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn ensure_account_column(
    connection: &Connection,
    name: &str,
    definition: &str,
) -> Result<bool, String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('accounts') WHERE name = ?1)",
            params![name],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
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

pub(super) fn open_database(state: &AppState) -> Result<Connection, String> {
    Connection::open(&state.database_path).map_err(database_error)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::*;

    #[test]
    fn legacy_profiles_table_is_renamed_without_losing_accounts() {
        let directory =
            std::env::temp_dir().join(format!("codex-switcher-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.clone(),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        upsert_profile_from_auth(
            &state,
            &json!({ "tokens": { "refresh_token": "rt-1" } }).to_string(),
            "saved",
            false,
        )
        .unwrap();
        open_database(&state)
            .unwrap()
            .execute("ALTER TABLE accounts RENAME TO profiles", [])
            .unwrap();

        initialize_database(&state).unwrap();

        let connection = open_database(&state).unwrap();
        let table_names = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(table_names.contains(&"accounts".to_string()));
        assert!(!table_names.contains(&"profiles".to_string()));
        assert_eq!(list_profiles(&connection, None).unwrap().len(), 1);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
}
