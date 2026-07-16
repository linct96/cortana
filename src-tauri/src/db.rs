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
              name TEXT NOT NULL COLLATE NOCASE UNIQUE,
              content TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS accounts (
              id TEXT PRIMARY KEY NOT NULL,
              account_type TEXT NOT NULL DEFAULT 'oauth',
              api_base_url TEXT,
              account_id TEXT NOT NULL DEFAULT '',
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
              usage_updated_at INTEGER,
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
        ("account_type", "TEXT NOT NULL DEFAULT 'oauth'"),
        ("api_base_url", "TEXT"),
        ("plan_type", "TEXT NOT NULL DEFAULT ''"),
        ("usage_primary_percent", "REAL"),
        ("usage_primary_window_minutes", "INTEGER"),
        ("usage_primary_resets_at", "INTEGER"),
        ("usage_secondary_percent", "REAL"),
        ("usage_secondary_window_minutes", "INTEGER"),
        ("usage_secondary_resets_at", "INTEGER"),
        ("usage_updated_at", "INTEGER"),
        ("reset_credits_available_count", "INTEGER"),
    ] {
        ensure_account_column(&connection, name, definition)?;
    }
    for name in ["credits_balance", "credits_unlimited", "auth_hash"] {
        if account_column_exists(&connection, name)? {
            connection
                .execute(&format!("ALTER TABLE accounts DROP COLUMN {name}"), [])
                .map_err(database_error)?;
        }
    }
    let added_sort_order =
        ensure_account_column(&connection, "sort_order", "INTEGER NOT NULL DEFAULT 0")?;
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
                    params![sort_order, profile_id],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;
    }
    connection
        .execute(
            "DELETE FROM settings WHERE key IN ('profile_order', 'active_profile_id', 'active_agents_profile_id')",
            [],
        )
        .map_err(database_error)?;
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
        drop(connection);

        initialize_database(&state).unwrap();

        let connection = open_database(&state).unwrap();
        assert!(!account_column_exists(&connection, "credits_balance").unwrap());
        assert!(!account_column_exists(&connection, "credits_unlimited").unwrap());
        assert!(!account_column_exists(&connection, "auth_hash").unwrap());
        assert_eq!(get_setting(&connection, "profile_order").unwrap(), None);
        assert_eq!(get_setting(&connection, "active_profile_id").unwrap(), None);
        assert_eq!(
            get_setting(&connection, "active_agents_profile_id").unwrap(),
            None
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT content FROM instruction_profiles WHERE id = 'default'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "# Rules"
        );
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
}
