use super::{codex::*, db::*, *};

const ACTIVE_AGENTS_PROFILE_ID: &str = "active_agents_profile_id";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentsProfile {
    id: String,
    name: String,
    content: String,
    is_active: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentsStatus {
    profiles: Vec<AgentsProfile>,
    active_profile_id: Option<String>,
    path: String,
    file_state: String,
}

fn agents_path(state: &AppState) -> Result<PathBuf, String> {
    Ok(auth_path(state)?.with_file_name("AGENTS.md"))
}

#[tauri::command]
pub(super) fn get_agents_status(state: State<'_, AppState>) -> Result<AgentsStatus, String> {
    get_agents_status_internal(&state)
}

#[tauri::command]
pub(super) fn create_agents_profile(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<AgentsProfile, String> {
    create_agents_profile_internal(&state, &name, &content)
}

#[tauri::command]
pub(super) fn update_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
    name: String,
    content: String,
    force: bool,
) -> Result<AgentsProfile, String> {
    update_agents_profile_internal(&state, &profile_id, &name, &content, force)
}

#[tauri::command]
pub(super) fn activate_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
    force: bool,
) -> Result<AgentsProfile, String> {
    activate_agents_profile_internal(&state, &profile_id, force)
}

#[tauri::command]
pub(super) fn import_current_agents(
    state: State<'_, AppState>,
    name: String,
) -> Result<AgentsProfile, String> {
    import_current_agents_internal(&state, &name)
}

#[tauri::command]
pub(super) fn delete_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    delete_agents_profile_internal(&state, &profile_id)
}

fn get_agents_status_internal(state: &AppState) -> Result<AgentsStatus, String> {
    let connection = open_database(state)?;
    let configured_active_id = get_setting(&connection, ACTIVE_AGENTS_PROFILE_ID)?;
    let path = agents_path(state)?;
    let file_content = read_optional_file(&path)?;
    let active_content = configured_active_id
        .as_deref()
        .map(|id| get_profile_content(&connection, id))
        .transpose()?
        .flatten();
    let managed_active_id = match (&configured_active_id, &active_content, &file_content) {
        (Some(id), Some(saved), Some(current)) if saved == current => Some(id.clone()),
        _ => None,
    };
    let file_state = if managed_active_id.is_some() {
        "managed"
    } else if file_content.as_deref().is_none_or(str::is_empty) {
        "missing"
    } else if active_content.is_some() {
        "external"
    } else {
        "unmanaged"
    };

    Ok(AgentsStatus {
        profiles: list_agents_profiles(&connection, managed_active_id.as_deref())?,
        active_profile_id: managed_active_id,
        path: path.display().to_string(),
        file_state: file_state.to_string(),
    })
}

fn create_agents_profile_internal(
    state: &AppState,
    name: &str,
    content: &str,
) -> Result<AgentsProfile, String> {
    let connection = open_database(state)?;
    let name = validate_name(&connection, name, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    connection
        .execute(
            "INSERT INTO agents_profiles (id, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, name, content, now],
        )
        .map_err(database_error)?;
    get_agents_profile(&connection, &id, None)
}

fn update_agents_profile_internal(
    state: &AppState,
    profile_id: &str,
    name: &str,
    content: &str,
    force: bool,
) -> Result<AgentsProfile, String> {
    let mut connection = open_database(state)?;
    let name = validate_name(&connection, name, Some(profile_id))?;
    get_profile_content(&connection, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    let configured_active_id = get_setting(&connection, ACTIVE_AGENTS_PROFILE_ID)?;
    let is_configured_active = configured_active_id.as_deref() == Some(profile_id);
    let path = agents_path(state)?;
    let backup = is_configured_active
        .then(|| read_optional_file(&path))
        .transpose()?
        .flatten();
    if is_configured_active {
        ensure_external_change_can_be_replaced(
            &connection,
            configured_active_id.as_deref(),
            backup.as_deref(),
            content,
            force,
        )?;
        write_file_atomically(&path, content)?;
    }

    let now = now_millis();
    let transaction = connection.transaction().map_err(database_error)?;
    let result = transaction
        .execute(
            "UPDATE agents_profiles SET name = ?1, content = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, content, now, profile_id],
        )
        .map_err(database_error)
        .and_then(|_| transaction.commit().map_err(database_error));
    if let Err(error) = result {
        if is_configured_active {
            restore_optional_file(&path, backup.as_deref())?;
        }
        return Err(error);
    }
    get_agents_profile(
        &connection,
        profile_id,
        is_configured_active.then_some(profile_id),
    )
}

fn activate_agents_profile_internal(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<AgentsProfile, String> {
    let mut connection = open_database(state)?;
    let content = get_profile_content(&connection, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    let configured_active_id = get_setting(&connection, ACTIVE_AGENTS_PROFILE_ID)?;
    let path = agents_path(state)?;
    let backup = read_optional_file(&path)?;
    ensure_external_change_can_be_replaced(
        &connection,
        configured_active_id.as_deref(),
        backup.as_deref(),
        &content,
        force,
    )?;
    write_file_atomically(&path, &content)?;

    let transaction = connection.transaction().map_err(database_error)?;
    let result = transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![ACTIVE_AGENTS_PROFILE_ID, profile_id],
        )
        .map_err(database_error)
        .and_then(|_| transaction.commit().map_err(database_error));
    if let Err(error) = result {
        restore_optional_file(&path, backup.as_deref())?;
        return Err(error);
    }
    get_agents_profile(&connection, profile_id, Some(profile_id))
}

fn import_current_agents_internal(state: &AppState, name: &str) -> Result<AgentsProfile, String> {
    let path = agents_path(state)?;
    let content = read_optional_file(&path)?
        .filter(|content| !content.is_empty())
        .ok_or_else(|| "当前 AGENTS.md 为空，无法同步。".to_string())?;
    let mut connection = open_database(state)?;
    let name = validate_name(&connection, name, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO agents_profiles (id, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, name, content, now],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![ACTIVE_AGENTS_PROFILE_ID, id],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)?;
    get_agents_profile(&connection, &id, Some(&id))
}

fn delete_agents_profile_internal(state: &AppState, profile_id: &str) -> Result<(), String> {
    let mut connection = open_database(state)?;
    let transaction = connection.transaction().map_err(database_error)?;
    let changed = transaction
        .execute(
            "DELETE FROM agents_profiles WHERE id = ?1",
            params![profile_id],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("提示词方案不存在。".to_string());
    }
    transaction
        .execute(
            "DELETE FROM settings WHERE key = ?1 AND value = ?2",
            params![ACTIVE_AGENTS_PROFILE_ID, profile_id],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

fn ensure_external_change_can_be_replaced(
    connection: &Connection,
    active_id: Option<&str>,
    current: Option<&str>,
    next: &str,
    force: bool,
) -> Result<(), String> {
    let current = current.unwrap_or_default();
    let managed = active_id
        .map(|id| get_profile_content(connection, id))
        .transpose()?
        .flatten()
        .is_some_and(|saved| saved == current);
    if !force && !current.is_empty() && !managed && current != next {
        return Err(
            "检测到工具外的 AGENTS.md 变更。请先同步当前文件，或确认后强制覆盖。".to_string(),
        );
    }
    Ok(())
}

fn validate_name(
    connection: &Connection,
    name: &str,
    excluded_id: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空。".to_string());
    }
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agents_profiles WHERE name = ?1 COLLATE NOCASE AND (?2 IS NULL OR id <> ?2))",
            params![name, excluded_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if exists {
        return Err("方案名称已存在。".to_string());
    }
    Ok(name.to_string())
}

fn get_profile_content(connection: &Connection, id: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT content FROM agents_profiles WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn list_agents_profiles(
    connection: &Connection,
    active_id: Option<&str>,
) -> Result<Vec<AgentsProfile>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, content, created_at, updated_at FROM agents_profiles ORDER BY created_at ASC")
        .map_err(database_error)?;
    let profiles = statement
        .query_map([], |row| agents_profile_from_row(row, active_id))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

fn get_agents_profile(
    connection: &Connection,
    id: &str,
    active_id: Option<&str>,
) -> Result<AgentsProfile, String> {
    connection
        .query_row(
            "SELECT id, name, content, created_at, updated_at FROM agents_profiles WHERE id = ?1",
            params![id],
            |row| agents_profile_from_row(row, active_id),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "提示词方案不存在。".to_string())
}

fn agents_profile_from_row(
    row: &rusqlite::Row<'_>,
    active_id: Option<&str>,
) -> rusqlite::Result<AgentsProfile> {
    let id: String = row.get(0)?;
    Ok(AgentsProfile {
        is_active: active_id == Some(id.as_str()),
        id,
        name: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> (PathBuf, AppState) {
        let directory =
            std::env::temp_dir().join(format!("cortana-agents-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        (directory, state)
    }

    #[test]
    fn creates_inactive_then_activates_exact_content() {
        let (directory, state) = test_state();
        let profile = create_agents_profile_internal(&state, "Default", "# Rules\n").unwrap();
        assert!(!agents_path(&state).unwrap().exists());
        update_agents_profile_internal(&state, &profile.id, "Default", "# Updated\n", false)
            .unwrap();
        assert!(!agents_path(&state).unwrap().exists());

        activate_agents_profile_internal(&state, &profile.id, false).unwrap();

        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "# Updated\n"
        );
        let status = get_agents_status_internal(&state).unwrap();
        assert_eq!(status.file_state, "managed");
        assert_eq!(
            status.active_profile_id.as_deref(),
            Some(profile.id.as_str())
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn protects_and_can_import_external_content() {
        let (directory, state) = test_state();
        let first = create_agents_profile_internal(&state, "First", "first").unwrap();
        let second = create_agents_profile_internal(&state, "Second", "second").unwrap();
        activate_agents_profile_internal(&state, &first.id, false).unwrap();
        write_file_atomically(&agents_path(&state).unwrap(), "external").unwrap();

        assert!(activate_agents_profile_internal(&state, &second.id, false)
            .unwrap_err()
            .contains("工具外"));
        let imported = import_current_agents_internal(&state, "Imported").unwrap();
        assert_eq!(imported.content, "external");
        assert!(imported.is_active);
        write_file_atomically(&agents_path(&state).unwrap(), "external-again").unwrap();
        activate_agents_profile_internal(&state, &second.id, true).unwrap();
        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "second"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn updates_active_file_and_leaves_it_when_deleted() {
        let (directory, state) = test_state();
        let profile = create_agents_profile_internal(&state, "Default", "old").unwrap();
        activate_agents_profile_internal(&state, &profile.id, false).unwrap();

        update_agents_profile_internal(&state, &profile.id, "Renamed", "new", false).unwrap();
        delete_agents_profile_internal(&state, &profile.id).unwrap();

        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "new"
        );
        assert_eq!(
            get_agents_status_internal(&state).unwrap().file_state,
            "unmanaged"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
